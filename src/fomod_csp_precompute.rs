//! Builds [`Precompute`], the read-only data every CSP solver phase shares.
//!
//! Everything expensive happens here, once, before the solve starts: per-plugin
//! evidence, the flag dependency graph, the destination-to-group reverse
//! indices, the contested-plugin set, and the decomposition into independent
//! components. [`build_precompute`] is the entry point;
//! [`crate::fomod_csp_solver`] calls it and then never recomputes.
//!
//! Two scans with different key sets feed those indices, and the difference is
//! load-bearing. The group scan walks every plugin's atoms, so `group_dests` and
//! `dest_to_groups` cover every non-excluded destination a plugin can produce.
//! The target scan walks the installed file tree, so `dest_to_plugins`,
//! `dest_to_size_match_groups`, `dest_to_hash_capable_groups`,
//! `conditional_dests` and `contested_plugins` only ever see destinations that
//! are present in the target and not excluded. See the "Index spaces" and
//! "Keying asymmetry" sections on [`Precompute`].
//!
//! ## Determinism
//!
//! The same inputs must always produce the same `Precompute`, because the option
//! cache and the memo table are keyed off values derived from it.
//!
//! - Every reverse-index vector is sorted ascending and deduplicated.
//! - Each component is sorted ascending, and components are ordered
//!   size-descending with a min-member-ascending tiebreak. The tiebreak makes
//!   the order total, so two equal-size components cannot swap between runs.
//!   See `PARITY-NOTES.md`.
//! - [`hash_flag_subset`] folds part of the flag map into the `u64` signature
//!   that gates option-cache and memo-key equality, reusing
//!   [`crate::utils::fnv1a_hash`] and [`crate::utils::hash_combine`]. Its byte
//!   layout is load-bearing: change the fold and every cached key changes with
//!   it.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::fomod_atom::{AtomIndex, ExpandedAtoms, Origin, TargetTree};
use crate::fomod_csp_types::{GroupRef, InferenceOverrides, Precompute};
use crate::fomod_ir::{FomodCondition, FomodConditionType, FomodInstaller};
use crate::fomod_propagator::PropagationResult;
use crate::utils::{fnv1a_hash, hash_combine};

/// Collect every flag name a condition tree references, into `out`.
///
/// A `Flag` node inserts its `flag_name`, skipping an empty one. A `Composite`
/// recurses into every child. Every other leaf (File, Game, Plugin, Fomod, Fomm,
/// Fose) contributes nothing.
pub fn collect_condition_flags(c: &FomodCondition, out: &mut HashSet<String>) {
    match c.r#type {
        FomodConditionType::Flag => {
            if !c.flag_name.is_empty() {
                out.insert(c.flag_name.clone());
            }
        }
        FomodConditionType::Composite => {
            for child in &c.children {
                collect_condition_flags(child, out);
            }
        }
        _ => {}
    }
}

/// True when any leaf of the condition tree depends on state inference cannot
/// observe: a File, Game, Plugin, Fomod, Fomm or Fose condition.
///
/// `Flag` is false, a `Composite` is the disjunction of its children (so an
/// empty one is false), and every other leaf type is true.
pub fn condition_depends_on_external_state(c: &FomodCondition) -> bool {
    match c.r#type {
        FomodConditionType::Flag => false,
        FomodConditionType::Composite => c.children.iter().any(condition_depends_on_external_state),
        _ => true,
    }
}

/// Fold a subset of the flag map into a `u64` cache-key signature.
///
/// Only the flags named in `keys` are folded, in `keys` order. Two callers: the
/// option enumerator passes `group_cache_flags[gidx]` and the backtracker passes
/// `memo_flags`; both are sorted byte-ascending. Each key folds its name first,
/// then either its value or the `0xA5A5A5A5A5A5A5A5` sentinel when the flag is
/// absent, which keeps an absent flag distinct from one set to the empty string.
/// The fold is key-order-sensitive by design, and its byte layout gates both
/// option-cache and memo-key equality, so do not reorder or reseed it.
pub fn hash_flag_subset(flags: &HashMap<String, String>, keys: &[String]) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for k in keys {
        hash_combine(&mut h, fnv1a_hash(k.as_bytes()));
        match flags.get(k) {
            Some(v) => hash_combine(&mut h, fnv1a_hash(v.as_bytes())),
            None => hash_combine(&mut h, 0xA5A5A5A5A5A5A5A5),
        }
    }
    h
}

/// Sort ascending, then deduplicate in place.
fn sort_unique(v: &mut Vec<i32>) {
    v.sort_unstable();
    v.dedup();
}

/// Flag name -> the `(flat_plugin_index, flag_value)` pairs that set it.
type FlagSetterMap = HashMap<String, Vec<(i32, String)>>;

/// Propagate evidence from a flag-gated condition back to the plugins that set
/// the flag values it expects.
///
/// A `Flag` node adds `weight` to every setter of that flag whose set value
/// equals the condition's expected value. A `Composite` recurses with the same
/// weight. Every other leaf contributes nothing.
fn collect_flag_evidence(
    cond: &FomodCondition,
    flag_setters: &FlagSetterMap,
    evidence: &mut [i32],
    weight: i32,
) {
    match cond.r#type {
        FomodConditionType::Flag => {
            if let Some(list) = flag_setters.get(&cond.flag_name) {
                for (pidx, pval) in list {
                    if *pval == cond.flag_value {
                        evidence[*pidx as usize] += weight;
                    }
                }
            }
        }
        FomodConditionType::Composite => {
            for child in &cond.children {
                collect_flag_evidence(child, flag_setters, evidence, weight);
            }
        }
        _ => {}
    }
}

/// Score every plugin on how well its files overlap the target tree.
///
/// Returns one entry per plugin in the whole installer, indexed by flat plugin
/// index in document order. Higher means stronger evidence that the plugin was
/// originally selected. Every entry starts at 0 and only grows; no rule
/// subtracts.
///
/// Three rules contribute, each with its own filter set. "auto" below means an
/// atom whose `always_install` or `install_if_usable` flag is set, and
/// "size-compatible" means either side reports size 0 (unknown) or the two sizes
/// are equal.
///
/// ```text
/// direct (per non-excluded target dest, per matching atom;
///         the atom must be Origin::Plugin, not auto, and size-compatible)
///   exactly 1 matching atom .. +3 to that atom's plugin
///   2+ matching atoms ........ per matching atom, +2 when that atom's plugin
///                              owns any atom on this dest whose nonzero
///                              content hash equals the target's nonzero
///                              hash, otherwise +1
///
/// indirect (weight = hits, added to every plugin that sets the flag value a
///           Flag leaf of the gating condition expects)
///   conditional pattern ci:
///     hits = atoms of per_conditional[ci] that are non-excluded, present in
///            the target, size-compatible and hash-compatible. Auto atoms
///            count here.
///   step visibility (only when the condition has no external-state leaf):
///     hits = atoms of that step's plugins that are non-excluded, present in
///            the target and size-compatible. Auto atoms are skipped here,
///            and hash is not tested.
/// ```
///
/// The direct rule counts matching atoms, not distinct plugins. One plugin that
/// supplies the same destination from two different sources contributes two
/// matching atoms, so it misses the `+3` branch and is scored once per atom in
/// the contested branch instead.
///
/// The target tree is iterated in unspecified (hash-map) order, but every
/// accumulation is a commutative `+=`, so the result is deterministic.
pub fn compute_evidence(
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    atom_index: &AtomIndex,
    target: &TargetTree,
    excluded: &HashSet<String>,
) -> Vec<i32> {
    let mut total_plugins = 0i32;
    for step in &installer.steps {
        for group in &step.groups {
            total_plugins += group.plugins.len() as i32;
        }
    }

    let mut evidence = vec![0i32; total_plugins as usize];

    for (dest, tf) in target {
        if excluded.contains(dest) {
            continue;
        }
        let Some(list) = atom_index.get(dest) else {
            continue;
        };

        let mut matches: Vec<i32> = Vec::new();
        for atom in list {
            if atom.origin != Origin::Plugin {
                continue;
            }
            if atom.always_install || atom.install_if_usable {
                continue;
            }
            if tf.size != 0 && atom.file_size != 0 && tf.size != atom.file_size {
                continue;
            }
            matches.push(atom.plugin_index);
        }

        if matches.len() == 1 {
            evidence[matches[0] as usize] += 3; // unique producer: strong signal
        } else {
            for &idx in &matches {
                let mut hash_matched = false; // contested: 2 if hash confirms, else 1
                for atom in list {
                    if atom.plugin_index == idx
                        && tf.hash != 0
                        && atom.content_hash != 0
                        && atom.content_hash == tf.hash
                    {
                        hash_matched = true;
                        break;
                    }
                }
                evidence[idx as usize] += if hash_matched { 2 } else { 1 };
            }
        }
    }

    // Flag setters: flat plugin index p -> the (name, value) pairs it sets.
    let mut setters: FlagSetterMap = HashMap::new();
    {
        let mut p = 0i32;
        for step in &installer.steps {
            for group in &step.groups {
                for plugin in &group.plugins {
                    for (fn_name, fv) in &plugin.condition_flags {
                        setters
                            .entry(fn_name.clone())
                            .or_default()
                            .push((p, fv.clone()));
                    }
                    p += 1;
                }
            }
        }
    }

    // Conditional-pattern indirect evidence.
    if !installer.conditional_patterns.is_empty() {
        for (ci, pattern) in installer.conditional_patterns.iter().enumerate() {
            let mut hits = 0i32;
            if ci < atoms.per_conditional.len() {
                for atom in &atoms.per_conditional[ci] {
                    if excluded.contains(&atom.dest_path) {
                        continue;
                    }
                    let Some(tfv) = target.get(&atom.dest_path) else {
                        continue;
                    };
                    let size_match =
                        tfv.size == 0 || atom.file_size == 0 || tfv.size == atom.file_size;
                    if !size_match {
                        continue;
                    }
                    let hash_match =
                        tfv.hash == 0 || atom.content_hash == 0 || tfv.hash == atom.content_hash;
                    if !hash_match {
                        continue;
                    }
                    hits += 1;
                }
            }
            if hits > 0 {
                collect_flag_evidence(&pattern.condition, &setters, &mut evidence, hits);
            }
        }
    }

    // Step-visibility indirect evidence.
    {
        let mut flat_idx = 0i32;
        for step in &installer.steps {
            let step_flat_start = flat_idx;
            for group in &step.groups {
                flat_idx += group.plugins.len() as i32;
            }

            let Some(visible) = &step.visible else {
                continue;
            };
            if condition_depends_on_external_state(visible) {
                continue;
            }

            let mut hits = 0i32;
            for fi in step_flat_start..flat_idx {
                if fi as usize >= atoms.per_plugin.len() {
                    break;
                }
                for atom in &atoms.per_plugin[fi as usize] {
                    if atom.always_install || atom.install_if_usable {
                        continue;
                    }
                    if excluded.contains(&atom.dest_path) {
                        continue;
                    }
                    let Some(tfv) = target.get(&atom.dest_path) else {
                        continue;
                    };
                    let size_match =
                        tfv.size == 0 || atom.file_size == 0 || tfv.size == atom.file_size;
                    if !size_match {
                        continue;
                    }
                    hits += 1;
                }
            }
            if hits > 0 {
                collect_flag_evidence(visible, &setters, &mut evidence, hits);
            }
        }
    }

    evidence
}

/// Add an undirected edge between two groups. A self-edge is a no-op.
fn link_groups(graph: &mut [HashSet<i32>], a: i32, b: i32) {
    if a == b {
        return;
    }
    graph[a as usize].insert(b);
    graph[b as usize].insert(a);
}

/// Split the groups into connected components of the dependency graph, found by
/// breadth-first search.
///
/// Three kinds of edge: groups that share a destination path; a flag-setter
/// group to every group that reads the same flag; and every pair of setters of
/// the same flag. Edge construction is commutative and idempotent, so the graph
/// does not depend on the iteration order of `dest_to_groups` or of the setter
/// map.
///
/// Each finished component is sorted ascending, which makes it deterministic
/// despite the unordered neighbor-visit order. Components are then ordered
/// size-descending with a min-member-ascending tiebreak, a total order, so
/// equal-size components cannot swap between runs. See the module doc.
fn build_components(pre: &Precompute) -> Vec<Vec<i32>> {
    let n = pre.groups.len();
    let mut graph: Vec<HashSet<i32>> = vec![HashSet::new(); n];

    for groups in pre.dest_to_groups.values() {
        for i in 0..groups.len() {
            for j in (i + 1)..groups.len() {
                link_groups(&mut graph, groups[i], groups[j]);
            }
        }
    }

    let mut setters: HashMap<String, Vec<i32>> = HashMap::new();
    let mut readers: HashMap<String, Vec<i32>> = HashMap::new();
    for g in 0..n {
        for fn_name in &pre.group_sets_flags[g] {
            setters.entry(fn_name.clone()).or_default().push(g as i32);
        }
        for fn_name in &pre.group_reads_flags[g] {
            readers.entry(fn_name.clone()).or_default().push(g as i32);
        }
    }

    for (flag, setv) in &setters {
        if let Some(rv) = readers.get(flag) {
            for &s in setv {
                for &r in rv {
                    link_groups(&mut graph, s, r);
                }
            }
        }
        for i in 0..setv.len() {
            for j in (i + 1)..setv.len() {
                link_groups(&mut graph, setv[i], setv[j]);
            }
        }
    }

    let mut comps: Vec<Vec<i32>> = Vec::new();
    let mut seen = vec![false; n];
    for i in 0..n {
        if seen[i] {
            continue;
        }
        let mut comp: Vec<i32> = Vec::new();
        let mut q: VecDeque<i32> = VecDeque::new();
        q.push_back(i as i32);
        seen[i] = true;
        while let Some(cur) = q.pop_front() {
            comp.push(cur);
            for &nxt in &graph[cur as usize] {
                if seen[nxt as usize] {
                    continue;
                }
                seen[nxt as usize] = true;
                q.push_back(nxt);
            }
        }
        comp.sort_unstable();
        comps.push(comp);
    }

    // Size descending, then min-member ascending (comp[0] is the sorted
    // minimum). A total, deterministic order; see the module doc.
    comps.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));
    comps
}

/// Build the [`Precompute`] the solver phases share.
///
/// `groups` and `evidence` come from the caller
/// ([`crate::fomod_csp_solver::solve_fomod_csp`]) and are moved in; the seven
/// reference inputs are borrowed for `'a`. Every reverse-index vector comes out
/// sorted ascending and deduplicated.
///
/// `propagation` is stored, not consumed here. The option enumerator reads
/// `propagation.narrowed_domains` to drop options that would select a pruned
/// plugin; nothing in this function or in the solver skips work because
/// propagation resolved a group.
///
/// Performs no I/O and logs nothing. It allocates one entry per installer plugin
/// and per target destination, so cost is linear in `atoms` plus `target`.
#[allow(clippy::too_many_arguments)]
pub fn build_precompute<'a>(
    installer: &'a FomodInstaller,
    atoms: &'a ExpandedAtoms,
    atom_index: &'a AtomIndex,
    target: &'a TargetTree,
    excluded: &'a HashSet<String>,
    overrides: Option<&'a InferenceOverrides>,
    propagation: Option<&'a PropagationResult>,
    groups: Vec<GroupRef>,
    evidence: Vec<i32>,
) -> Precompute<'a> {
    let mut p = Precompute {
        installer,
        atoms,
        atom_index,
        target,
        excluded,
        overrides,
        propagation,
        groups,
        evidence,
        plugin_to_group: Vec::new(),
        plugin_unique_support: Vec::new(),
        needed_flags: HashSet::new(),
        group_sets_flags: Vec::new(),
        group_reads_flags: Vec::new(),
        group_cache_flags: Vec::new(),
        flag_to_setter_groups: HashMap::new(),
        memo_flags: Vec::new(),
        group_dests: Vec::new(),
        dest_to_groups: HashMap::new(),
        dest_to_plugins: HashMap::new(),
        dest_to_size_match_groups: HashMap::new(),
        dest_to_hash_capable_groups: HashMap::new(),
        conditional_dests: HashSet::new(),
        contested_plugins: Vec::new(),
        components: Vec::new(),
    };

    let mut total_plugins = 0i32;
    for step in &installer.steps {
        for group in &step.groups {
            total_plugins += group.plugins.len() as i32;
        }
    }

    p.plugin_to_group = vec![-1i32; total_plugins as usize];
    p.plugin_unique_support = vec![0i32; total_plugins as usize];

    let ng = p.groups.len();
    p.group_sets_flags = vec![HashSet::new(); ng];
    p.group_reads_flags = vec![HashSet::new(); ng];
    p.group_cache_flags = vec![Vec::new(); ng];
    p.group_dests = vec![HashSet::new(); ng];

    // needed_flags: from every step.visible, every plugin type_pattern condition
    // and dependencies, and every conditional pattern condition.
    for step in &installer.steps {
        if let Some(v) = &step.visible {
            collect_condition_flags(v, &mut p.needed_flags);
        }
        for group in &step.groups {
            for plugin in &group.plugins {
                for tp in &plugin.type_patterns {
                    collect_condition_flags(&tp.condition, &mut p.needed_flags);
                }
                if let Some(dep) = &plugin.dependencies {
                    collect_condition_flags(dep, &mut p.needed_flags);
                }
            }
        }
    }
    for cp in &installer.conditional_patterns {
        collect_condition_flags(&cp.condition, &mut p.needed_flags);
    }

    // Main group loop: reverse indices, sets/reads flags, group dests.
    for gidx in 0..p.groups.len() {
        let gref = p.groups[gidx];
        let step = &installer.steps[gref.step_idx as usize];
        let group = &step.groups[gref.group_idx as usize];

        if let Some(v) = &step.visible {
            collect_condition_flags(v, &mut p.group_reads_flags[gidx]);
        }

        for pi in 0..gref.plugin_count {
            let flat_plugin = gref.flat_start + pi;
            p.plugin_to_group[flat_plugin as usize] = gidx as i32;
            let plugin = &group.plugins[pi as usize];

            for (fn_name, _fv) in &plugin.condition_flags {
                p.group_sets_flags[gidx].insert(fn_name.clone());
            }
            for tp in &plugin.type_patterns {
                collect_condition_flags(&tp.condition, &mut p.group_reads_flags[gidx]);
            }
            if let Some(dep) = &plugin.dependencies {
                collect_condition_flags(dep, &mut p.group_reads_flags[gidx]);
            }

            if (flat_plugin as usize) < atoms.per_plugin.len() {
                let mut seen: HashSet<String> = HashSet::new();
                for atom in &atoms.per_plugin[flat_plugin as usize] {
                    if excluded.contains(&atom.dest_path) {
                        continue;
                    }
                    if !seen.insert(atom.dest_path.clone()) {
                        continue;
                    }
                    p.group_dests[gidx].insert(atom.dest_path.clone());
                    p.dest_to_groups
                        .entry(atom.dest_path.clone())
                        .or_default()
                        .push(gidx as i32);
                }
            }
        }
    }

    for groups_vec in p.dest_to_groups.values_mut() {
        sort_unique(groups_vec);
    }

    for gidx in 0..p.group_sets_flags.len() {
        for fn_name in &p.group_sets_flags[gidx] {
            p.flag_to_setter_groups
                .entry(fn_name.clone())
                .or_default()
                .push(gidx as i32);
        }
    }
    for setters in p.flag_to_setter_groups.values_mut() {
        sort_unique(setters);
    }

    for g in 0..p.group_cache_flags.len() {
        let mut keys: Vec<String> = p.group_reads_flags[g].iter().cloned().collect();
        keys.sort_unstable();
        p.group_cache_flags[g] = keys;
    }

    {
        let mut memo_flag_set: HashSet<String> = p.needed_flags.clone();
        for s in &p.group_sets_flags {
            for fn_name in s {
                memo_flag_set.insert(fn_name.clone());
            }
        }
        let mut memo: Vec<String> = memo_flag_set.into_iter().collect();
        memo.sort_unstable();
        p.memo_flags = memo;
    }

    // Contested / reverse-index loop over the target tree. Everything written
    // below this point is keyed by non-excluded destinations that exist in the
    // target, unlike `dest_to_groups` and `group_dests` above, which are keyed
    // by every destination a plugin can produce.
    let mut contested_plugins: HashSet<i32> = HashSet::new();

    for (dest, tf) in target {
        if excluded.contains(dest) {
            continue;
        }
        let Some(list) = atom_index.get(dest) else {
            continue;
        };

        let mut size_match_groups: Vec<i32> = Vec::new();
        let mut hash_capable_groups: Vec<i32> = Vec::new();
        let mut candidate_plugins: HashSet<i32> = HashSet::new();
        let mut producer_plugins: HashSet<i32> = HashSet::new();
        let mut sources: HashSet<String> = HashSet::new();
        let mut has_conditional = false;

        for atom in list {
            if atom.origin == Origin::Conditional {
                has_conditional = true;
            }

            if atom.origin != Origin::Plugin || atom.plugin_index < 0 {
                continue;
            }

            producer_plugins.insert(atom.plugin_index);

            if atom.plugin_index >= p.plugin_to_group.len() as i32 {
                continue;
            }

            let gidx = p.plugin_to_group[atom.plugin_index as usize];
            if gidx < 0 {
                continue;
            }

            let size_match = tf.size == 0 || atom.file_size == 0 || tf.size == atom.file_size;
            let hash_capable = size_match
                && (tf.hash == 0 || atom.content_hash == 0 || tf.hash == atom.content_hash);

            if size_match {
                size_match_groups.push(gidx);
            }
            if hash_capable {
                hash_capable_groups.push(gidx);
            }

            let mut candidate = size_match;
            if candidate && tf.hash != 0 && atom.content_hash != 0 && tf.hash != atom.content_hash {
                candidate = false;
            }
            if candidate {
                candidate_plugins.insert(atom.plugin_index);
            }

            sources.insert(atom.source_path.clone());
            // Unconditional: every plugin producing any surviving target dest
            // becomes "contested", conflicted or not. Do not add a multiplicity
            // test here. `contested_plugins` feeds `contested_signature`, which
            // is a MemoKey field, so a narrower set changes every memo key and
            // therefore which subtrees are pruned.
            contested_plugins.insert(atom.plugin_index);
        }

        sort_unique(&mut size_match_groups);
        sort_unique(&mut hash_capable_groups);

        let mut producers_vec: Vec<i32> = producer_plugins.into_iter().collect();
        sort_unique(&mut producers_vec);
        p.dest_to_plugins.insert(dest.clone(), producers_vec);

        p.dest_to_size_match_groups
            .insert(dest.clone(), size_match_groups);
        p.dest_to_hash_capable_groups
            .insert(dest.clone(), hash_capable_groups);

        if candidate_plugins.len() == 1 {
            let unique_plugin = *candidate_plugins.iter().next().unwrap();
            if unique_plugin >= 0 && (unique_plugin as usize) < p.plugin_unique_support.len() {
                p.plugin_unique_support[unique_plugin as usize] += 1;
            }
        }

        if has_conditional {
            p.conditional_dests.insert(dest.clone());
        }

        // Redundant in practice: every `candidate_plugins` member already went
        // through the unconditional insert above. Removing it would assert a
        // no-op that nothing here proves, and this set decides every memo key,
        // so leave it in place.
        if sources.len() > 1 {
            for flat_plugin in &candidate_plugins {
                contested_plugins.insert(*flat_plugin);
            }
        }
    }

    let mut cp: Vec<i32> = contested_plugins.into_iter().collect();
    cp.sort_unstable();
    p.contested_plugins = cp;

    let components = build_components(&p);
    p.components = components;
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_atom::{FomodAtom, TargetFile};
    use crate::fomod_ir::{
        FomodConditionOp, FomodConditionalPattern, FomodGroup, FomodGroupType, FomodPlugin,
        FomodStep, FomodTypePattern,
    };
    use crate::types::PluginType;

    // --- small IR/atom builders -------------------------------------------

    fn flag_cond(name: &str, value: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Flag,
            flag_name: name.to_string(),
            flag_value: value.to_string(),
            ..FomodCondition::default()
        }
    }

    fn file_cond(path: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::File,
            file_path: path.to_string(),
            file_state: "Active".to_string(),
            ..FomodCondition::default()
        }
    }

    fn composite(op: FomodConditionOp, children: Vec<FomodCondition>) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Composite,
            op,
            children,
            ..FomodCondition::default()
        }
    }

    fn plugin(name: &str) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            ..FomodPlugin::default()
        }
    }

    fn plugin_atom(dest: &str, source: &str, plugin_index: i32) -> FomodAtom {
        FomodAtom {
            source_path: source.to_string(),
            dest_path: dest.to_string(),
            origin: Origin::Plugin,
            plugin_index,
            ..FomodAtom::default()
        }
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Build GroupRefs in installer document order, assigning `flat_start` from
    /// the running plugin count, as the solver does before its per-step priority
    /// sort. That sort is orthogonal to what these tests cover.
    fn doc_order_group_refs(installer: &FomodInstaller) -> Vec<GroupRef> {
        let mut groups = Vec::new();
        let mut flat = 0i32;
        for (si, step) in installer.steps.iter().enumerate() {
            for (gi, group) in step.groups.iter().enumerate() {
                let pc = group.plugins.len() as i32;
                groups.push(GroupRef {
                    step_idx: si as i32,
                    group_idx: gi as i32,
                    flat_start: flat,
                    plugin_count: pc,
                });
                flat += pc;
            }
        }
        groups
    }

    // --- hash_flag_subset (H1) --------------------------------------------

    #[test]
    fn hash_flag_subset_present_key_folds_key_then_value() {
        let flags: HashMap<String, String> =
            [("a".to_string(), "b".to_string())].into_iter().collect();
        // Golden constant: pins the byte layout of the fold.
        assert_eq!(
            hash_flag_subset(&flags, &["a".to_string()]),
            0x9874ce55450d2e52
        );
        // And it equals the manual composition of the reused primitives.
        let mut h = 14695981039346656037u64;
        hash_combine(&mut h, fnv1a_hash(b"a"));
        hash_combine(&mut h, fnv1a_hash(b"b"));
        assert_eq!(hash_flag_subset(&flags, &["a".to_string()]), h);
    }

    #[test]
    fn hash_flag_subset_absent_key_uses_sentinel() {
        let empty: HashMap<String, String> = HashMap::new();
        assert_eq!(
            hash_flag_subset(&empty, &["a".to_string()]),
            0x923681afa569f252
        );
        // A present empty-string value differs from an absent key (sentinel).
        let present_empty: HashMap<String, String> =
            [("a".to_string(), String::new())].into_iter().collect();
        assert_eq!(
            hash_flag_subset(&present_empty, &["a".to_string()]),
            0xfd8588ed44ed70d2
        );
        assert_ne!(
            hash_flag_subset(&empty, &["a".to_string()]),
            hash_flag_subset(&present_empty, &["a".to_string()])
        );
    }

    #[test]
    fn hash_flag_subset_is_key_order_sensitive_and_deterministic() {
        let flags: HashMap<String, String> = [
            ("a".to_string(), "x".to_string()),
            ("b".to_string(), "y".to_string()),
        ]
        .into_iter()
        .collect();
        let ab = hash_flag_subset(&flags, &["a".to_string(), "b".to_string()]);
        let ba = hash_flag_subset(&flags, &["b".to_string(), "a".to_string()]);
        assert_eq!(ab, 0x7d466b326b1e3c28);
        assert_eq!(ba, 0xac0d26d67ea23017);
        assert_ne!(ab, ba, "key order is folded, so it changes the result");
        // Determinism.
        assert_eq!(
            ab,
            hash_flag_subset(&flags, &["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn hash_flag_subset_empty_keys_is_offset_basis() {
        let flags: HashMap<String, String> = HashMap::new();
        assert_eq!(hash_flag_subset(&flags, &[]), 14695981039346656037);
    }

    // --- collect_condition_flags / external state (C1, C2) ----------------

    #[test]
    fn collect_condition_flags_skips_empty_and_non_flag_leaves() {
        let mut out = HashSet::new();
        collect_condition_flags(&flag_cond("f1", "on"), &mut out);
        collect_condition_flags(&flag_cond("", "x"), &mut out); // empty name skipped
        collect_condition_flags(&file_cond("a.esp"), &mut out); // leaf contributes nothing
        collect_condition_flags(
            &composite(
                FomodConditionOp::And,
                vec![
                    flag_cond("f2", "a"),
                    file_cond("b.esp"),
                    flag_cond("f3", "b"),
                ],
            ),
            &mut out,
        );
        assert_eq!(out, set(&["f1", "f2", "f3"]));
    }

    #[test]
    fn condition_depends_on_external_state_matrix() {
        assert!(!condition_depends_on_external_state(&flag_cond("f", "v")));
        assert!(condition_depends_on_external_state(&file_cond("a.esp")));
        // Composite with only flags -> false.
        assert!(!condition_depends_on_external_state(&composite(
            FomodConditionOp::Or,
            vec![flag_cond("a", "1"), flag_cond("b", "2")]
        )));
        // Composite containing any external leaf -> true.
        assert!(condition_depends_on_external_state(&composite(
            FomodConditionOp::And,
            vec![flag_cond("a", "1"), file_cond("x.esp")]
        )));
        // Empty composite -> false (no external leaf anywhere).
        assert!(!condition_depends_on_external_state(&composite(
            FomodConditionOp::And,
            vec![]
        )));
    }

    // --- compute_evidence (E1-E5) -----------------------------------------

    /// One step, one SelectAny group with `plugins.len()` plugins.
    fn single_group_installer(plugins: Vec<FomodPlugin>) -> FomodInstaller {
        FomodInstaller {
            steps: vec![FomodStep {
                groups: vec![FomodGroup {
                    r#type: FomodGroupType::SelectAny,
                    plugins,
                    ..FomodGroup::default()
                }],
                ..FomodStep::default()
            }],
            ..FomodInstaller::default()
        }
    }

    fn index_of(atoms: &[FomodAtom]) -> AtomIndex {
        let mut idx: AtomIndex = HashMap::new();
        for a in atoms {
            idx.entry(a.dest_path.clone()).or_default().push(a.clone());
        }
        idx
    }

    #[test]
    fn evidence_unique_size_compatible_producer_scores_three() {
        // Two plugins, dest "d" produced only by plugin 0 (size compatible).
        let installer = single_group_installer(vec![plugin("P0"), plugin("P1")]);
        let a0 = FomodAtom {
            file_size: 100,
            ..plugin_atom("d", "s0", 0)
        };
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()], vec![]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0]);
        let target: TargetTree = [("d".to_string(), TargetFile { size: 100, hash: 0 })]
            .into_iter()
            .collect();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &HashSet::new());
        assert_eq!(ev, vec![3, 0]);
    }

    #[test]
    fn evidence_size_incompatible_atom_excluded_from_matches() {
        let installer = single_group_installer(vec![plugin("P0")]);
        // atom file_size 200 vs target size 100 -> not a match -> matches empty
        // -> no evidence (the else-branch over an empty match list is a no-op).
        let a0 = FomodAtom {
            file_size: 200,
            ..plugin_atom("d", "s0", 0)
        };
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0]);
        let target: TargetTree = [("d".to_string(), TargetFile { size: 100, hash: 0 })]
            .into_iter()
            .collect();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &HashSet::new());
        assert_eq!(ev, vec![0]);
    }

    #[test]
    fn evidence_auto_atoms_are_excluded() {
        let installer = single_group_installer(vec![plugin("P0")]);
        let a0 = FomodAtom {
            always_install: true,
            file_size: 100,
            ..plugin_atom("d", "s0", 0)
        };
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0]);
        let target: TargetTree = [("d".to_string(), TargetFile { size: 100, hash: 0 })]
            .into_iter()
            .collect();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &HashSet::new());
        assert_eq!(ev, vec![0]);
    }

    #[test]
    fn evidence_contested_hash_match_scores_two_no_hash_scores_one() {
        // dest "d" produced by plugin 0 (hash match) and plugin 1 (no hash).
        let installer = single_group_installer(vec![plugin("P0"), plugin("P1")]);
        let a0 = FomodAtom {
            file_size: 100,
            content_hash: 0xABCD,
            ..plugin_atom("d", "s0", 0)
        };
        let a1 = FomodAtom {
            file_size: 100,
            content_hash: 0, // no hash confirmation
            ..plugin_atom("d", "s1", 1)
        };
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()], vec![a1.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0, a1]);
        let target: TargetTree = [(
            "d".to_string(),
            TargetFile {
                size: 100,
                hash: 0xABCD,
            },
        )]
        .into_iter()
        .collect();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &HashSet::new());
        assert_eq!(ev, vec![2, 1]);
    }

    #[test]
    fn evidence_excluded_dest_is_skipped() {
        let installer = single_group_installer(vec![plugin("P0")]);
        let a0 = FomodAtom {
            file_size: 100,
            ..plugin_atom("d", "s0", 0)
        };
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0]);
        let target: TargetTree = [("d".to_string(), TargetFile { size: 100, hash: 0 })]
            .into_iter()
            .collect();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &set(&["d"]));
        assert_eq!(ev, vec![0]);
    }

    #[test]
    fn evidence_conditional_pattern_propagates_to_flag_setter() {
        // Plugin 0 sets flag "F"="on"; a conditional pattern gated on F==on
        // produces one target-matching file -> +1 hit to plugin 0.
        let mut p0 = plugin("P0");
        p0.condition_flags = vec![("F".to_string(), "on".to_string())];
        let mut installer = single_group_installer(vec![p0, plugin("P1")]);
        installer.conditional_patterns = vec![FomodConditionalPattern {
            condition: flag_cond("F", "on"),
            ..FomodConditionalPattern::default()
        }];

        let cond_atom = FomodAtom {
            file_size: 50,
            origin: Origin::Conditional,
            conditional_index: 0,
            plugin_index: -1,
            ..FomodAtom::default()
        };
        let cond_atom = FomodAtom {
            dest_path: "cd".to_string(),
            source_path: "cs".to_string(),
            ..cond_atom
        };
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![], vec![]],
            per_conditional: vec![vec![cond_atom.clone()]],
            ..ExpandedAtoms::default()
        };
        // atom_index need not include the conditional dest for the conditional
        // evidence path (it reads per_conditional + target directly).
        let index: AtomIndex = HashMap::new();
        let target: TargetTree = [("cd".to_string(), TargetFile { size: 50, hash: 0 })]
            .into_iter()
            .collect();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &HashSet::new());
        assert_eq!(ev, vec![1, 0]);
    }

    #[test]
    fn evidence_step_visibility_propagates_and_skips_external_visible() {
        // Step 0 sets no flags; step 1 has a plugin producing a target file and
        // is gated (visible) on flag F==on set by step 0's plugin. The step-1
        // hits propagate to step 0's flag setter (plugin index 0). No hash test
        // is applied in this path.
        let mut p0 = plugin("P0");
        p0.condition_flags = vec![("F".to_string(), "on".to_string())];
        let step0 = FomodStep {
            groups: vec![FomodGroup {
                r#type: FomodGroupType::SelectAny,
                plugins: vec![p0],
                ..FomodGroup::default()
            }],
            ..FomodStep::default()
        };
        let step1 = FomodStep {
            visible: Some(flag_cond("F", "on")),
            groups: vec![FomodGroup {
                r#type: FomodGroupType::SelectAny,
                plugins: vec![plugin("P1")],
                ..FomodGroup::default()
            }],
            ..FomodStep::default()
        };
        let installer = FomodInstaller {
            steps: vec![step0, step1],
            ..FomodInstaller::default()
        };
        // Plugin 1 (flat index 1) produces a target-matching file (size only).
        let a1 = FomodAtom {
            file_size: 70,
            ..plugin_atom("vd", "vs", 1)
        };
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![], vec![a1.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a1]);
        // Note "vd" also gets a direct +3 for plugin 1 (unique producer).
        let target: TargetTree = [("vd".to_string(), TargetFile { size: 70, hash: 0 })]
            .into_iter()
            .collect();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &HashSet::new());
        // plugin 0: +1 indirect (step-visibility hit). plugin 1: +3 direct.
        assert_eq!(ev, vec![1, 3]);

        // If the visible condition depends on external state, the step-vis path
        // is skipped: plugin 0 gets no indirect evidence.
        let mut installer2 = installer.clone();
        installer2.steps[1].visible = Some(composite(
            FomodConditionOp::And,
            vec![flag_cond("F", "on"), file_cond("ext.esp")],
        ));
        let ev2 = compute_evidence(&installer2, &atoms, &index, &target, &HashSet::new());
        assert_eq!(ev2, vec![0, 3]);
    }

    // --- build_precompute (P1-P10) ----------------------------------------

    #[test]
    fn precompute_reverse_indices_sorted_deduped_and_plugin_to_group() {
        // Two groups: group 0 has plugins {0,1}, group 1 has plugin {2}. Plugins
        // 1 and 2 both produce "shared"; plugin 0 produces "d0".
        let g0 = FomodGroup {
            r#type: FomodGroupType::SelectAny,
            plugins: vec![plugin("P0"), plugin("P1")],
            ..FomodGroup::default()
        };
        let g1 = FomodGroup {
            r#type: FomodGroupType::SelectExactlyOne,
            plugins: vec![plugin("P2")],
            ..FomodGroup::default()
        };
        let installer = FomodInstaller {
            steps: vec![FomodStep {
                groups: vec![g0, g1],
                ..FomodStep::default()
            }],
            ..FomodInstaller::default()
        };
        let a0 = plugin_atom("d0", "s0", 0);
        // Plugin 1 lists "shared" twice (duplicate dest within a plugin) to
        // exercise the per-plugin `seen` dedup.
        let a1a = plugin_atom("shared", "s1", 1);
        let a1b = plugin_atom("shared", "s1b", 1);
        let a2 = plugin_atom("shared", "s2", 2);
        let atoms = ExpandedAtoms {
            per_plugin: vec![
                vec![a0.clone()],
                vec![a1a.clone(), a1b.clone()],
                vec![a2.clone()],
            ],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0, a1a, a1b, a2]);
        let target: TargetTree = [
            ("d0".to_string(), TargetFile { size: 0, hash: 0 }),
            ("shared".to_string(), TargetFile { size: 0, hash: 0 }),
        ]
        .into_iter()
        .collect();
        let excluded = HashSet::new();
        let groups = doc_order_group_refs(&installer);
        let evidence = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer, &atoms, &index, &target, &excluded, None, None, groups, evidence,
        );

        assert_eq!(pre.plugin_to_group, vec![0, 0, 1]);
        // "shared" is produced by groups 0 (plugin 1) and 1 (plugin 2) - sorted,
        // deduped (plugin 1 pushed gidx 0 once thanks to per-plugin seen dedup).
        assert_eq!(pre.dest_to_groups["shared"], vec![0, 1]);
        assert_eq!(pre.dest_to_groups["d0"], vec![0]);
        // Reverse indices are all sorted-ascending and deduped.
        for v in pre.dest_to_groups.values() {
            assert!(v.windows(2).all(|w| w[0] < w[1]));
        }
        for v in pre.dest_to_plugins.values() {
            assert!(v.windows(2).all(|w| w[0] < w[1]));
        }
        for v in pre.dest_to_size_match_groups.values() {
            assert!(v.windows(2).all(|w| w[0] < w[1]));
        }
        for v in pre.dest_to_hash_capable_groups.values() {
            assert!(v.windows(2).all(|w| w[0] < w[1]));
        }
        for v in pre.flag_to_setter_groups.values() {
            assert!(v.windows(2).all(|w| w[0] < w[1]));
        }
        assert!(pre.contested_plugins.windows(2).all(|w| w[0] < w[1]));
        // "shared" has two producers -> contested.
        assert_eq!(pre.dest_to_plugins["shared"], vec![1, 2]);
        assert!(pre.contested_plugins.contains(&1));
        assert!(pre.contested_plugins.contains(&2));
    }

    #[test]
    fn precompute_plugin_unique_support_gated_on_single_candidate() {
        // "u" produced only by plugin 0 (candidate). "c" produced by plugins 0
        // and 1 (two candidates -> no unique support bump).
        let installer = single_group_installer(vec![plugin("P0"), plugin("P1")]);
        let a_u = plugin_atom("u", "su", 0);
        let a_c0 = plugin_atom("c", "sc0", 0);
        let a_c1 = plugin_atom("c", "sc1", 1);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a_u.clone(), a_c0.clone()], vec![a_c1.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a_u, a_c0, a_c1]);
        let target: TargetTree = [
            ("u".to_string(), TargetFile { size: 0, hash: 0 }),
            ("c".to_string(), TargetFile { size: 0, hash: 0 }),
        ]
        .into_iter()
        .collect();
        let excluded = HashSet::new();
        let groups = doc_order_group_refs(&installer);
        let evidence = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer, &atoms, &index, &target, &excluded, None, None, groups, evidence,
        );
        // Only "u" has exactly one candidate (plugin 0). "c" has two candidates.
        assert_eq!(pre.plugin_unique_support, vec![1, 0]);
    }

    #[test]
    fn precompute_conditional_dests_and_needed_flags_and_sorted_flag_lists() {
        // Plugin 0 reads flag F (type pattern) and sets flag G; a conditional
        // pattern reads flag H. Dest "cd" has a Conditional-origin atom.
        let mut p0 = plugin("P0");
        p0.type_patterns = vec![FomodTypePattern {
            condition: flag_cond("F", "1"),
            result_type: PluginType::Required,
        }];
        p0.condition_flags = vec![("G".to_string(), "gv".to_string())];
        let mut installer = single_group_installer(vec![p0]);
        installer.conditional_patterns = vec![FomodConditionalPattern {
            condition: flag_cond("H", "1"),
            ..FomodConditionalPattern::default()
        }];

        let plug_atom = plugin_atom("cd", "ps", 0);
        let cond_atom = FomodAtom {
            dest_path: "cd".to_string(),
            source_path: "cs".to_string(),
            origin: Origin::Conditional,
            conditional_index: 0,
            plugin_index: -1,
            ..FomodAtom::default()
        };
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![plug_atom.clone()]],
            per_conditional: vec![vec![cond_atom.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[plug_atom, cond_atom]);
        let target: TargetTree = [("cd".to_string(), TargetFile { size: 0, hash: 0 })]
            .into_iter()
            .collect();
        let excluded = HashSet::new();
        let groups = doc_order_group_refs(&installer);
        let evidence = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer, &atoms, &index, &target, &excluded, None, None, groups, evidence,
        );

        // needed_flags = F (type pattern) + H (conditional). G is set-only,
        // never read, so it is not needed.
        assert!(pre.needed_flags.contains("F"));
        assert!(pre.needed_flags.contains("H"));
        assert!(!pre.needed_flags.contains("G"));
        // group 0 sets G, reads F.
        assert_eq!(pre.group_sets_flags[0], set(&["G"]));
        assert_eq!(pre.group_reads_flags[0], set(&["F"]));
        assert_eq!(pre.group_cache_flags[0], vec!["F".to_string()]);
        // memo_flags = needed (F,H) union set flags (G), sorted byte-ascending.
        assert_eq!(
            pre.memo_flags,
            vec!["F".to_string(), "G".to_string(), "H".to_string()]
        );
        // "cd" has a Conditional-origin atom -> conditional dest.
        assert!(pre.conditional_dests.contains("cd"));
        // flag_to_setter_groups: G -> [0].
        assert_eq!(pre.flag_to_setter_groups["G"], vec![0]);
    }

    #[test]
    fn precompute_group_cache_flags_are_byte_ascending() {
        // A plugin reading three flags in a non-sorted insertion order; the
        // cache-flag key list must be byte-ascending.
        let mut p0 = plugin("P0");
        p0.type_patterns = vec![
            FomodTypePattern {
                condition: flag_cond("zeta", "1"),
                result_type: PluginType::Required,
            },
            FomodTypePattern {
                condition: flag_cond("Alpha", "1"),
                result_type: PluginType::Required,
            },
            FomodTypePattern {
                condition: flag_cond("mid", "1"),
                result_type: PluginType::Required,
            },
        ];
        let installer = single_group_installer(vec![p0]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![]],
            ..ExpandedAtoms::default()
        };
        let index: AtomIndex = HashMap::new();
        let target: TargetTree = HashMap::new();
        let excluded = HashSet::new();
        let groups = doc_order_group_refs(&installer);
        let evidence = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer, &atoms, &index, &target, &excluded, None, None, groups, evidence,
        );
        // Uppercase 'A' (0x41) sorts before lowercase 'm'/'z' bytes.
        assert_eq!(
            pre.group_cache_flags[0],
            vec!["Alpha".to_string(), "mid".to_string(), "zeta".to_string()]
        );
    }

    // --- build_components (K1-K3) -----------------------------------------

    /// N independent single-plugin groups, each producing its own dest.
    fn independent_groups_installer(n: usize) -> FomodInstaller {
        let mut groups = Vec::new();
        for i in 0..n {
            groups.push(FomodGroup {
                r#type: FomodGroupType::SelectAny,
                plugins: vec![plugin(&format!("P{i}"))],
                ..FomodGroup::default()
            });
        }
        FomodInstaller {
            steps: vec![FomodStep {
                groups,
                ..FomodStep::default()
            }],
            ..FomodInstaller::default()
        }
    }

    #[test]
    fn components_two_independent_ordered_by_size_desc() {
        // Group 0 and group 1 share dest "shared" (2-member component); group 2
        // is alone (1-member). Component order: size descending -> [ [0,1], [2] ].
        let installer = independent_groups_installer(3);
        let a0 = plugin_atom("shared", "s0", 0);
        let a1 = plugin_atom("shared", "s1", 1);
        let a2 = plugin_atom("solo", "s2", 2);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()], vec![a1.clone()], vec![a2.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0, a1, a2]);
        let target: TargetTree = HashMap::new();
        let excluded = HashSet::new();
        let groups = doc_order_group_refs(&installer);
        let evidence = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer, &atoms, &index, &target, &excluded, None, None, groups, evidence,
        );
        assert_eq!(pre.components, vec![vec![0, 1], vec![2]]);
    }

    #[test]
    fn components_equal_size_tiebreak_is_min_member_ascending() {
        // Two disjoint 2-member components: {0,1} sharing "sh_a" and {2,3}
        // sharing "sh_b". Equal size -> min-member ascending: [0,1] before [2,3].
        let installer = independent_groups_installer(4);
        let a0 = plugin_atom("sh_a", "s0", 0);
        let a1 = plugin_atom("sh_a", "s1", 1);
        let a2 = plugin_atom("sh_b", "s2", 2);
        let a3 = plugin_atom("sh_b", "s3", 3);
        let atoms = ExpandedAtoms {
            per_plugin: vec![
                vec![a0.clone()],
                vec![a1.clone()],
                vec![a2.clone()],
                vec![a3.clone()],
            ],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0, a1, a2, a3]);
        let target: TargetTree = HashMap::new();
        let excluded = HashSet::new();
        let groups = doc_order_group_refs(&installer);
        let evidence = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer, &atoms, &index, &target, &excluded, None, None, groups, evidence,
        );
        assert_eq!(pre.components, vec![vec![0, 1], vec![2, 3]]);
    }

    #[test]
    fn components_flag_setter_and_reader_land_in_one_component() {
        // Group 0 sets flag F; group 1 reads flag F (via a type pattern). No
        // shared dest, but the flag edge links them into a single component.
        let mut p0 = plugin("P0");
        p0.condition_flags = vec![("F".to_string(), "on".to_string())];
        let mut p1 = plugin("P1");
        p1.type_patterns = vec![FomodTypePattern {
            condition: flag_cond("F", "on"),
            result_type: PluginType::Required,
        }];
        let installer = FomodInstaller {
            steps: vec![FomodStep {
                groups: vec![
                    FomodGroup {
                        r#type: FomodGroupType::SelectAny,
                        plugins: vec![p0],
                        ..FomodGroup::default()
                    },
                    FomodGroup {
                        r#type: FomodGroupType::SelectAny,
                        plugins: vec![p1],
                        ..FomodGroup::default()
                    },
                ],
                ..FomodStep::default()
            }],
            ..FomodInstaller::default()
        };
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![], vec![]],
            ..ExpandedAtoms::default()
        };
        let index: AtomIndex = HashMap::new();
        let target: TargetTree = HashMap::new();
        let excluded = HashSet::new();
        let groups = doc_order_group_refs(&installer);
        let evidence = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer, &atoms, &index, &target, &excluded, None, None, groups, evidence,
        );
        assert_eq!(pre.components, vec![vec![0, 1]]);
    }

    #[test]
    fn precompute_is_deterministic() {
        let installer = independent_groups_installer(4);
        let a0 = plugin_atom("sh_a", "s0", 0);
        let a1 = plugin_atom("sh_a", "s1", 1);
        let a2 = plugin_atom("sh_b", "s2", 2);
        let a3 = plugin_atom("sh_b", "s3", 3);
        let atoms = ExpandedAtoms {
            per_plugin: vec![
                vec![a0.clone()],
                vec![a1.clone()],
                vec![a2.clone()],
                vec![a3.clone()],
            ],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0, a1, a2, a3]);
        let target: TargetTree = [
            ("sh_a".to_string(), TargetFile { size: 0, hash: 0 }),
            ("sh_b".to_string(), TargetFile { size: 0, hash: 0 }),
        ]
        .into_iter()
        .collect();
        let excluded = HashSet::new();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre_a = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_order_group_refs(&installer),
            ev.clone(),
        );
        let pre_b = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_order_group_refs(&installer),
            ev,
        );
        assert_eq!(pre_a, pre_b);
    }
}
