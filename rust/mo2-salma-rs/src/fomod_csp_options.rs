//! CSP per-group option enumeration - Rust port of `src/FomodCSPOptions.hpp` /
//! `.cpp`.
//!
//! Enumerates the candidate plugin-selection options for a single FOMOD group
//! under a given flag state and SelectAny cap, then reduces them to a compact,
//! high-quality, cached set. The CSP solver (Task 9) materializes each group's
//! options through [`get_options_for_group`], keyed by
//! [`OptionCacheKey`](crate::fomod_csp_types::OptionCacheKey) so repeated visits
//! to the same `(group, flag signature)` pair are O(1).
//!
//! ## Determinism
//!
//! Two C++ sites are nondeterministic (unstable sorts, unordered-map iteration)
//! and are made run-to-run deterministic here with TOTAL-order tiebreaks:
//!
//! - `generate_raw_options` sorts plugins by evidence DESC (unstable in C++) and
//!   the small-group powerset by score DESC (unstable); both get an
//!   index/mask-ascending tiebreak.
//! - `reduce_options` builds the surviving candidate list from an unordered map
//!   then unstable-sorts by (evidence DESC, unique DESC, useful DESC, extra ASC);
//!   this port adds a raw-option-index-ascending final tiebreak.
//!
//! Exact bit-parity with a specific C++ build is not guaranteed on those ties
//! (only run-to-run determinism); see `rust/PARITY-NOTES.md` "Task 8". The
//! [`option_signature`] byte-fold IS byte-exact (reuses `fnv1a_hash` +
//! `hash_combine` over sorted produced-atoms and flags).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::fomod_csp_precompute::{condition_depends_on_external_state, hash_flag_subset};
use crate::fomod_csp_types::{
    CachedOptions, GroupOption, GroupRef, OptionCacheKey, OptionProfile, Precompute,
    SELECT_ANY_CAP_FULL, SolverStats,
};
use crate::fomod_dependency_evaluator::evaluate_plugin_type;
use crate::fomod_ir::{FomodGroup, FomodGroupType};
use crate::logger::Logger;
use crate::types::PluginType;
use crate::utils::{fnv1a_hash, hash_combine};

/// Check if a group requires exact (exhaustive) search mode. Mirror of the C++
/// `is_exact_group_mode`: true iff `exact_groups` contains `gidx`.
pub fn is_exact_group_mode(gidx: i32, exact_groups: Option<&HashSet<i32>>) -> bool {
    exact_groups.is_some_and(|s| s.contains(&gidx))
}

/// Compute the effective SelectAny cap for a group. Mirror of the C++
/// `effective_select_any_cap`: `SELECT_ANY_CAP_FULL` (0, uncapped) when the
/// group is in exact mode, otherwise `select_any_cap` unchanged.
pub fn effective_select_any_cap(
    gidx: i32,
    select_any_cap: i32,
    exact_groups: Option<&HashSet<i32>>,
) -> i32 {
    if is_exact_group_mode(gidx, exact_groups) {
        SELECT_ANY_CAP_FULL
    } else {
        select_any_cap
    }
}

/// Return a human-readable `step N "StepName" / group M "GroupName"` label for
/// logging. Mirror of the C++ `group_name`. Log-only (no behavioral effect); it
/// is interpolated into the `[solver]` lines emitted by
/// [`get_options_for_group`].
pub fn group_name(pre: &Precompute<'_>, g: &GroupRef) -> String {
    let step = &pre.installer.steps[g.step_idx as usize];
    let group = &step.groups[g.group_idx as usize];
    format!(
        "step {} \"{}\" / group {} \"{}\"",
        g.step_idx, step.name, g.group_idx, group.name
    )
}

/// True if any entry of the mask is selected.
fn any_selected(opt: &[bool]) -> bool {
    opt.iter().any(|&b| b)
}

/// Generate candidate selection options for a single FOMOD group. Mirror of the
/// C++ `generate_raw_options`.
///
/// Classifies each plugin as required/usable/dynamic by evaluating its
/// dependencyType patterns against `flags` (context-free inference), demotes
/// externally-dynamic Required plugins, sorts by descending evidence, then
/// branches on the group cardinality type. SelectAtLeastOne/SelectAny with <= 10
/// plugins (and not forced heuristic) enumerate the full powerset; larger groups
/// use a greedy + neighborhood heuristic. A post-filter drops options where an
/// intra-group flag side-effect makes a selected plugin NotUsable, and at least
/// one option is always returned.
fn generate_raw_options(
    group: &FomodGroup,
    evidence: &[i32],
    flat_start: i32,
    flags: &HashMap<String, String>,
    select_any_cap: i32,
) -> Vec<GroupOption> {
    let n = group.plugins.len();
    if n == 0 {
        return vec![Vec::new()]; // one empty option
    }
    let fs = flat_start as usize;

    let mut required = vec![false; n];
    let mut usable = vec![false; n];
    let mut dynamic_external = vec![false; n];
    for i in 0..n {
        let plugin = &group.plugins[i];
        let dynamic = !plugin.type_patterns.is_empty();
        if dynamic {
            for pattern in &plugin.type_patterns {
                if condition_depends_on_external_state(&pattern.condition) {
                    dynamic_external[i] = true;
                    break;
                }
            }
        }
        let eff = evaluate_plugin_type(plugin, flags, None);
        if eff == PluginType::Required {
            required[i] = true;
        }
        if eff != PluginType::NotUsable {
            usable[i] = true;
        }
    }
    let mut has_required = required.iter().any(|&b| b);
    let has_dynamic_external = dynamic_external.iter().any(|&b| b);

    // Externally-dynamic plugins are unknowable during standalone inference:
    // keep them selectable and do not force them Required.
    if has_dynamic_external {
        for i in 0..n {
            if dynamic_external[i] {
                usable[i] = true;
            }
            if required[i] && dynamic_external[i] {
                required[i] = false;
            }
        }
        has_required = required.iter().any(|&b| b);
    }

    // Order plugins by evidence DESC, with an index-ascending tiebreak (the C++
    // std::sort is unstable; the total tiebreak makes this deterministic).
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| evidence[fs + b].cmp(&evidence[fs + a]).then(a.cmp(&b)));

    let mut options: Vec<GroupOption> = Vec::new();

    match group.r#type {
        FomodGroupType::SelectAll => {
            // Single option: every usable plugin selected.
            options.push(usable.clone());
        }
        FomodGroupType::SelectExactlyOne => {
            let gate = if has_required { &required } else { &usable };
            for &i in &order {
                if !gate[i] {
                    continue;
                }
                let mut opt = vec![false; n];
                opt[i] = true;
                options.push(opt);
            }
        }
        FomodGroupType::SelectAtMostOne => {
            if has_required {
                for &i in &order {
                    if !required[i] {
                        continue;
                    }
                    let mut opt = vec![false; n];
                    opt[i] = true;
                    options.push(opt);
                }
            } else {
                for &i in &order {
                    if !usable[i] {
                        continue;
                    }
                    let mut opt = vec![false; n];
                    opt[i] = true;
                    options.push(opt);
                }
                options.push(vec![false; n]); // trailing empty
            }
        }
        FomodGroupType::SelectAtLeastOne | FomodGroupType::SelectAny => {
            let at_least_one = group.r#type == FomodGroupType::SelectAtLeastOne;
            let mut positive_evidence = 0i32;
            for i in 0..n {
                if usable[i] && evidence[fs + i] > 0 {
                    positive_evidence += 1;
                }
            }

            // No-evidence SelectAny on medium groups: skip the powerset blowup.
            let force_heuristic_select_any = select_any_cap > 0
                && group.r#type == FomodGroupType::SelectAny
                && !has_required
                && positive_evidence == 0
                && n >= 8;

            if n <= 10 && !force_heuristic_select_any {
                let limit: u64 = 1u64 << n;
                let mut scored: Vec<(i32, u64)> = Vec::new();
                for mask in 0..limit {
                    if at_least_one && mask == 0 {
                        continue;
                    }
                    let mut valid = true;
                    for i in 0..n {
                        let selected = (mask & (1u64 << i)) != 0;
                        if required[i] && !selected {
                            valid = false;
                            break;
                        }
                        if selected && !usable[i] {
                            valid = false;
                            break;
                        }
                    }
                    if !valid {
                        continue;
                    }
                    let mut score = 0i32;
                    for i in 0..n {
                        if (mask & (1u64 << i)) != 0 {
                            score += evidence[fs + i];
                        }
                    }
                    scored.push((score, mask));
                }
                // Score DESC, mask ASC tiebreak (the C++ std::sort is unstable).
                scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                for (_, mask) in &scored {
                    let opt: GroupOption = (0..n).map(|i| (mask & (1u64 << i)) != 0).collect();
                    options.push(opt);
                }
            } else {
                let mut greedy = vec![false; n];
                for i in 0..n {
                    if required[i] {
                        greedy[i] = true;
                    }
                    if usable[i] && evidence[fs + i] > 0 {
                        greedy[i] = true;
                    }
                }
                if !at_least_one || any_selected(&greedy) {
                    options.push(greedy.clone());
                }

                for &i in &order {
                    if !greedy[i] || required[i] {
                        continue;
                    }
                    let mut var = greedy.clone();
                    var[i] = false;
                    if at_least_one && !any_selected(&var) {
                        continue;
                    }
                    options.push(var);
                }

                for &i in &order {
                    if !usable[i] {
                        continue;
                    }
                    let mut opt = required.clone();
                    opt[i] = true;
                    options.push(opt);
                }

                if !at_least_one {
                    options.push(required.clone());
                }

                // Bounded pair candidates for SelectAny "filter" groups.
                let allow_pair_candidates =
                    group.r#type == FomodGroupType::SelectAny && positive_evidence > 0;
                if allow_pair_candidates {
                    let mut pair_candidates: Vec<usize> = Vec::new();
                    for &i in &order {
                        if !usable[i] || required[i] {
                            continue;
                        }
                        pair_candidates.push(i);
                    }
                    const FULL_PAIR_LIMIT: usize = 16;
                    const HEURISTIC_PAIR_LIMIT: usize = 8;
                    let pair_limit = if pair_candidates.len() <= FULL_PAIR_LIMIT {
                        pair_candidates.len()
                    } else {
                        HEURISTIC_PAIR_LIMIT
                    };
                    if pair_candidates.len() > pair_limit {
                        pair_candidates.truncate(pair_limit);
                    }

                    for a in 0..pair_candidates.len() {
                        for b in (a + 1)..pair_candidates.len() {
                            let mut opt = required.clone();
                            opt[pair_candidates[a]] = true;
                            opt[pair_candidates[b]] = true;
                            if at_least_one && !any_selected(&opt) {
                                continue;
                            }
                            options.push(opt);
                        }
                    }
                }
            }
        }
    }

    // Enforce intra-group dependencyType consistency using flags set by the
    // option itself: drop options where a selected plugin becomes NotUsable
    // under the local flag overlay (unless it is externally-dynamic).
    options.retain(|opt| {
        let mut local_flags = flags.clone();
        for i in 0..n {
            if !(i < opt.len() && opt[i]) {
                continue;
            }
            for (fn_name, fv) in &group.plugins[i].condition_flags {
                local_flags.insert(fn_name.clone(), fv.clone());
            }
        }
        for i in 0..n {
            if !(i < opt.len() && opt[i]) {
                continue;
            }
            let eff = evaluate_plugin_type(&group.plugins[i], &local_flags, None);
            if eff == PluginType::NotUsable && !dynamic_external[i] {
                return false; // erase
            }
        }
        true // keep
    });

    if options.is_empty() {
        options.push(required.clone());
    }

    options
}

/// Profile the effect of selecting one option in a group. Mirror of the C++
/// `build_option_profile`.
///
/// Accumulates evidence and unique support over the selected plugins, records
/// the flags they write (and whether any is a needed flag), and classifies each
/// non-excluded produced destination as useful (in target) or extra.
fn build_option_profile(
    gref: &GroupRef,
    option: &GroupOption,
    pre: &Precompute<'_>,
) -> OptionProfile {
    let mut p = OptionProfile {
        option: option.clone(),
        ..OptionProfile::default()
    };

    let mut useful: HashSet<String> = HashSet::new();
    let mut extra: HashSet<String> = HashSet::new();

    let pc = gref.plugin_count as usize;
    let fs = gref.flat_start as usize;
    for pi in 0..pc {
        if !(pi < option.len() && option[pi]) {
            continue;
        }
        let flat_plugin = fs + pi;
        p.evidence_score += pre.evidence[flat_plugin];
        p.unique_support += pre.plugin_unique_support[flat_plugin];

        let step = &pre.installer.steps[gref.step_idx as usize];
        let group = &step.groups[gref.group_idx as usize];
        let plugin = &group.plugins[pi];

        for (fn_name, fv) in &plugin.condition_flags {
            p.flags_written.insert(fn_name.clone(), fv.clone());
            if pre.needed_flags.contains(fn_name) {
                p.sets_needed_flag = true;
            }
        }

        if flat_plugin < pre.atoms.per_plugin.len() {
            for atom in &pre.atoms.per_plugin[flat_plugin] {
                if pre.excluded.contains(&atom.dest_path) {
                    continue;
                }
                p.produced.insert(atom.dest_path.clone());
                p.produced_atoms
                    .insert(format!("{}|{}", atom.dest_path, atom.source_path));
                if pre.target.contains_key(&atom.dest_path) {
                    useful.insert(atom.dest_path.clone());
                } else {
                    extra.insert(atom.dest_path.clone());
                }
            }
        }
    }

    p.useful_dests = useful.len() as i32;
    p.extra_dests = extra.len() as i32;
    p
}

/// Byte-exact signature of an option's output (produced atoms + flags written).
/// Mirror of the C++ `option_signature`.
///
/// Copies the produced-atom keys (`"dest|source"`) into a sorted vector and
/// folds each; then copies the written flags into a `(name, value)` vector
/// sorted by name-then-value and folds name then value per pair. The sorts make
/// this deterministic despite the unordered source containers.
fn option_signature(p: &OptionProfile) -> u64 {
    let mut h: u64 = 14695981039346656037;

    let mut produced_atoms: Vec<&String> = p.produced_atoms.iter().collect();
    produced_atoms.sort_unstable();
    for atom_sig in &produced_atoms {
        hash_combine(&mut h, fnv1a_hash(atom_sig.as_bytes()));
    }

    let mut flags: Vec<(&String, &String)> = p.flags_written.iter().collect();
    flags.sort_unstable();
    for (fn_name, fv) in &flags {
        hash_combine(&mut h, fnv1a_hash(fn_name.as_bytes()));
        hash_combine(&mut h, fnv1a_hash(fv.as_bytes()));
    }

    h
}

/// Tiebreaker for options with identical output signatures. Mirror of the C++
/// `better_equivalent_option`: evidence DESC, unique_support DESC, useful_dests
/// DESC, extra_dests ASC.
fn better_equivalent_option(a: &OptionProfile, b: &OptionProfile) -> bool {
    if a.evidence_score != b.evidence_score {
        return a.evidence_score > b.evidence_score;
    }
    if a.unique_support != b.unique_support {
        return a.unique_support > b.unique_support;
    }
    if a.useful_dests != b.useful_dests {
        return a.useful_dests > b.useful_dests;
    }
    a.extra_dests < b.extra_dests
}

/// Pick the candidate at `rank` into `narrowed` if it is in range, not already
/// chosen, and the cap is not yet reached. Mirror of the C++ `pick_rank` lambda
/// (as a free function so the borrows stay simple).
fn pick_rank(
    rank: usize,
    cap: usize,
    candidates: &[usize],
    narrowed: &mut Vec<usize>,
    chosen_rank: &mut [bool],
) {
    if rank >= candidates.len() || chosen_rank[rank] || narrowed.len() >= cap {
        return;
    }
    chosen_rank[rank] = true;
    narrowed.push(candidates[rank]);
}

/// Reduce the raw option set to a compact, high-quality subset. Mirror of the
/// C++ `reduce_options`.
///
/// Three stages: (1) drop extra-only options; (2) collapse options with
/// identical output signatures, keeping the best; (3) for SelectAny/AtLeastOne,
/// cap the count via diversity-then-fill sampling. The candidate ordering uses a
/// TOTAL-order comparator (evidence DESC, unique DESC, useful DESC, extra ASC,
/// then raw-option index ASC) so the result is deterministic despite the
/// unordered-map source; see the module doc.
fn reduce_options(
    gref: &GroupRef,
    raw: &[GroupOption],
    pre: &Precompute<'_>,
    stats: &mut SolverStats,
    select_any_cap: i32,
    exact_mode: bool,
) -> Vec<GroupOption> {
    if raw.is_empty() {
        return Vec::new();
    }

    let mut prof: Vec<OptionProfile> = Vec::with_capacity(raw.len());
    for opt in raw {
        prof.push(build_option_profile(gref, opt, pre));
    }

    // Extra-only drop.
    let mut keep: Vec<usize> = Vec::new();
    for (i, pr) in prof.iter().enumerate() {
        if !exact_mode && pr.extra_dests > 0 && pr.useful_dests == 0 && !pr.sets_needed_flag {
            stats.dropped_extra_only_options += 1;
            continue;
        }
        keep.push(i);
    }
    if keep.is_empty() {
        keep = (0..prof.len()).collect();
    }

    // Signature collapse: keep the best per signature.
    let mut best_by_sig: HashMap<u64, usize> = HashMap::new();
    for &i in &keep {
        let sig = option_signature(&prof[i]);
        match best_by_sig.get(&sig).copied() {
            None => {
                best_by_sig.insert(sig, i);
            }
            Some(best) => {
                if better_equivalent_option(&prof[i], &prof[best]) {
                    best_by_sig.insert(sig, i);
                }
                stats.collapsed_equivalent_options += 1;
            }
        }
    }

    // Candidate ordering: total-order comparator (raw-index ASC final tiebreak).
    let mut candidates: Vec<usize> = best_by_sig.values().copied().collect();
    candidates.sort_by(|&a, &b| {
        prof[b]
            .evidence_score
            .cmp(&prof[a].evidence_score)
            .then(prof[b].unique_support.cmp(&prof[a].unique_support))
            .then(prof[b].useful_dests.cmp(&prof[a].useful_dests))
            .then(prof[a].extra_dests.cmp(&prof[b].extra_dests))
            .then(a.cmp(&b))
    });

    // SelectAny cap via diversity-then-fill sampling.
    let group = &pre.installer.steps[gref.step_idx as usize].groups[gref.group_idx as usize];
    let capped_select_any = group.r#type == FomodGroupType::SelectAny
        || group.r#type == FomodGroupType::SelectAtLeastOne;
    if capped_select_any && select_any_cap > 0 && candidates.len() > select_any_cap as usize {
        let cap = select_any_cap as usize;
        let mut narrowed: Vec<usize> = Vec::with_capacity(cap);
        let mut chosen_rank = vec![false; candidates.len()];

        // Keep the best option, then diversify by number of selected plugins.
        pick_rank(0, cap, &candidates, &mut narrowed, &mut chosen_rank);

        let mut seen_selected_counts: HashSet<i32> = HashSet::new();
        let mut rank = 0usize;
        while rank < candidates.len() && narrowed.len() < cap {
            let opt = &prof[candidates[rank]].option;
            let selected = opt.iter().filter(|&&b| b).count() as i32;
            if seen_selected_counts.insert(selected) {
                pick_rank(rank, cap, &candidates, &mut narrowed, &mut chosen_rank);
            }
            rank += 1;
        }

        let mut rank = 0usize;
        while rank < candidates.len() && narrowed.len() < cap {
            pick_rank(rank, cap, &candidates, &mut narrowed, &mut chosen_rank);
            rank += 1;
        }

        stats.capped_select_any_options += (candidates.len() - narrowed.len()) as i32;
        candidates = narrowed;
    }

    let mut out: Vec<GroupOption> = Vec::with_capacity(candidates.len());
    for &i in &candidates {
        out.push(prof[i].option.clone());
    }
    if out.is_empty() {
        out.push(raw[0].clone());
    }
    out
}

/// Enumerate valid selection options for a group, returning cached results when
/// available. Mirror of the C++ `get_options_for_group`.
///
/// The cache key is `(gidx, hash_flag_subset(flags, group_cache_flags[gidx]),
/// effective_cap, exact_mode)`. On a miss the raw options are generated,
/// filtered against any propagation-narrowed domain, reduced, and stored
/// alongside their per-option profiles. An out-of-range `gidx` logs a
/// `[solver]` error and returns a shared empty [`CachedOptions`] (the C++
/// returns a `static` empty).
pub fn get_options_for_group<'c>(
    gidx: i32,
    pre: &Precompute<'_>,
    flags: &HashMap<String, String>,
    select_any_cap: i32,
    exact_groups: Option<&HashSet<i32>>,
    cache: &'c mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) -> &'c CachedOptions {
    let exact_mode = is_exact_group_mode(gidx, exact_groups);
    let effective_cap = effective_select_any_cap(gidx, select_any_cap, exact_groups);
    if gidx < 0 || gidx as usize >= pre.group_cache_flags.len() {
        let size = pre.group_cache_flags.len();
        Logger::instance().log_error(&format!("[solver] gidx {gidx} out of range (size {size})"));
        static EMPTY_OPTIONS: OnceLock<CachedOptions> = OnceLock::new();
        return EMPTY_OPTIONS.get_or_init(CachedOptions::default);
    }

    let key = OptionCacheKey {
        group_idx: gidx,
        flags_sig: hash_flag_subset(flags, &pre.group_cache_flags[gidx as usize]),
        select_any_cap: effective_cap,
        exact_mode,
    };

    // Only compute on a cache miss (the reduction has stats side effects that
    // must not be double-counted on a hit), so this uses the Vacant-entry form
    // rather than always constructing the value.
    if let std::collections::hash_map::Entry::Vacant(slot) = cache.entry(key) {
        let gref = pre.groups[gidx as usize];
        let group = &pre.installer.steps[gref.step_idx as usize].groups[gref.group_idx as usize];

        let mut raw =
            generate_raw_options(group, &pre.evidence, gref.flat_start, flags, effective_cap);

        // Propagation domain narrowing: drop options that select a pruned plugin.
        if let Some(prop) = pre.propagation {
            let domains = &prop.narrowed_domains;
            if (gref.step_idx as usize) < domains.len()
                && (gref.group_idx as usize) < domains[gref.step_idx as usize].len()
            {
                let group_domain = &domains[gref.step_idx as usize][gref.group_idx as usize];
                raw.retain(|opt| {
                    for i in 0..opt.len().min(group_domain.len()) {
                        if opt[i] && !group_domain[i] {
                            return false; // erase
                        }
                    }
                    true
                });
            }
        }

        let reduced = reduce_options(&gref, &raw, pre, stats, effective_cap, exact_mode);

        let mut entry = CachedOptions {
            options: reduced,
            profiles: Vec::new(),
        };
        entry.profiles.reserve(entry.options.len());
        for opt in &entry.options {
            entry.profiles.push(build_option_profile(&gref, opt, pre));
        }

        // Emit the branching / group stats once per group. The C++ indexes
        // `stats.logged_group_options[gidx]` directly; the guarded lookup here
        // keeps an unsized counter vector from panicking.
        if let Some(logged) = stats.logged_group_options.get_mut(gidx as usize) {
            *logged = true;
            let mut positive_evidence = 0i32;
            let mut usable_plugins = 0i32;
            let pc = gref.plugin_count as usize;
            let fs = gref.flat_start as usize;
            for pi in 0..pc {
                if pre.evidence[fs + pi] > 0 {
                    positive_evidence += 1;
                }
                let eff = evaluate_plugin_type(&group.plugins[pi], flags, None);
                if eff != PluginType::NotUsable {
                    usable_plugins += 1;
                }
            }
            let label = group_name(pre, &gref);
            let raw_count = raw.len();
            let reduced_count = entry.options.len();
            Logger::instance().log(&format!(
                "[solver] Branching {label}: options raw={raw_count} reduced={reduced_count}"
            ));
            let plugin_count = gref.plugin_count;
            Logger::instance().log(&format!(
                "[solver] Group stats {label}: plugins={plugin_count}, usable={usable_plugins}, positive_evidence={positive_evidence}"
            ));
        }

        slot.insert(entry);
    }

    cache.get(&key).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_atom::{AtomIndex, ExpandedAtoms, FomodAtom, Origin, TargetFile, TargetTree};
    use crate::fomod_csp_precompute::{build_precompute, compute_evidence};
    use crate::fomod_ir::{
        FomodCondition, FomodConditionType, FomodGroup, FomodInstaller, FomodPlugin, FomodStep,
        FomodTypePattern,
    };
    use crate::fomod_propagator::PropagationResult;

    // --- builders ---------------------------------------------------------

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

    fn plugin(name: &str) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            ..FomodPlugin::default()
        }
    }

    fn plugin_typed(name: &str, base: PluginType) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            r#type: base,
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

    /// A single group of type `gt` with the given plugins, and its own flat
    /// evidence vector (indexed by local plugin position == flat index).
    fn single_group(gt: FomodGroupType, plugins: Vec<FomodPlugin>) -> FomodInstaller {
        FomodInstaller {
            steps: vec![FomodStep {
                groups: vec![FomodGroup {
                    r#type: gt,
                    plugins,
                    ..FomodGroup::default()
                }],
                ..FomodStep::default()
            }],
            ..FomodInstaller::default()
        }
    }

    fn no_flags() -> HashMap<String, String> {
        HashMap::new()
    }

    /// Set bits of a boolean mask, as ascending local indices.
    fn selected(opt: &[bool]) -> Vec<usize> {
        opt.iter()
            .enumerate()
            .filter_map(|(i, &b)| b.then_some(i))
            .collect()
    }

    // --- is_exact / effective cap -----------------------------------------

    #[test]
    fn exact_mode_and_effective_cap() {
        let exact: HashSet<i32> = [2].into_iter().collect();
        assert!(!is_exact_group_mode(1, None));
        assert!(!is_exact_group_mode(1, Some(&exact)));
        assert!(is_exact_group_mode(2, Some(&exact)));
        assert_eq!(effective_select_any_cap(1, 64, Some(&exact)), 64);
        assert_eq!(
            effective_select_any_cap(2, 64, Some(&exact)),
            SELECT_ANY_CAP_FULL
        );
        assert_eq!(effective_select_any_cap(1, 64, None), 64);
    }

    // --- generate_raw_options per group type (direct, deterministic) -------

    #[test]
    fn raw_options_n_zero_returns_single_empty() {
        let group = FomodGroup {
            r#type: FomodGroupType::SelectAny,
            plugins: vec![],
            ..FomodGroup::default()
        };
        let opts = generate_raw_options(&group, &[], 0, &no_flags(), 64);
        assert_eq!(opts, vec![Vec::<bool>::new()]);
    }

    #[test]
    fn raw_options_select_all_single_usable_mask() {
        // Plugin 1 is NotUsable; SelectAll -> single option selecting the usable.
        let group = FomodGroup {
            r#type: FomodGroupType::SelectAll,
            plugins: vec![plugin("P0"), plugin_typed("P1", PluginType::NotUsable)],
            ..FomodGroup::default()
        };
        let opts = generate_raw_options(&group, &[0, 0], 0, &no_flags(), 64);
        assert_eq!(opts, vec![vec![true, false]]);
    }

    #[test]
    fn raw_options_select_exactly_one_singletons_required_only_by_evidence() {
        // No required: singletons over usable, ordered by evidence DESC.
        let group = FomodGroup {
            r#type: FomodGroupType::SelectExactlyOne,
            plugins: vec![plugin("P0"), plugin("P1"), plugin("P2")],
            ..FomodGroup::default()
        };
        // evidence: P1 highest, then P2, then P0.
        let opts = generate_raw_options(&group, &[1, 5, 3], 0, &no_flags(), 64);
        assert_eq!(
            opts.iter().map(|o| selected(o)).collect::<Vec<_>>(),
            vec![vec![1], vec![2], vec![0],]
        );

        // With a required plugin, only required singletons are emitted.
        let group_req = FomodGroup {
            r#type: FomodGroupType::SelectExactlyOne,
            plugins: vec![
                plugin("P0"),
                plugin_typed("P1", PluginType::Required),
                plugin("P2"),
            ],
            ..FomodGroup::default()
        };
        let opts_req = generate_raw_options(&group_req, &[9, 1, 9], 0, &no_flags(), 64);
        assert_eq!(
            opts_req.iter().map(|o| selected(o)).collect::<Vec<_>>(),
            vec![vec![1]]
        );
    }

    #[test]
    fn raw_options_select_at_most_one_trailing_empty_only_without_required() {
        let group = FomodGroup {
            r#type: FomodGroupType::SelectAtMostOne,
            plugins: vec![plugin("P0"), plugin("P1")],
            ..FomodGroup::default()
        };
        let opts = generate_raw_options(&group, &[2, 5], 0, &no_flags(), 64);
        // Singletons by evidence DESC (P1, P0) then a trailing empty option.
        assert_eq!(
            opts.iter().map(|o| selected(o)).collect::<Vec<_>>(),
            vec![vec![1], vec![0], vec![],]
        );

        // With a required plugin: only the required singleton, NO trailing empty.
        let group_req = FomodGroup {
            r#type: FomodGroupType::SelectAtMostOne,
            plugins: vec![plugin_typed("P0", PluginType::Required), plugin("P1")],
            ..FomodGroup::default()
        };
        let opts_req = generate_raw_options(&group_req, &[0, 9], 0, &no_flags(), 64);
        assert_eq!(
            opts_req.iter().map(|o| selected(o)).collect::<Vec<_>>(),
            vec![vec![0]]
        );
    }

    #[test]
    fn raw_options_select_at_least_one_powerset_skips_zero_and_orders_by_score() {
        // 2 plugins, no required. at_least_one -> mask 0 excluded. Powerset:
        // {0} score 2, {1} score 5, {0,1} score 7 -> order 7,5,2.
        let group = FomodGroup {
            r#type: FomodGroupType::SelectAtLeastOne,
            plugins: vec![plugin("P0"), plugin("P1")],
            ..FomodGroup::default()
        };
        let opts = generate_raw_options(&group, &[2, 5], 0, &no_flags(), 64);
        assert_eq!(
            opts.iter().map(|o| selected(o)).collect::<Vec<_>>(),
            vec![
                vec![0, 1], // score 7
                vec![1],    // score 5
                vec![0],    // score 2
            ]
        );
        // mask 0 (empty) is never present.
        assert!(opts.iter().all(|o| any_selected(o)));
    }

    #[test]
    fn raw_options_select_any_powerset_includes_empty_and_rejects_notusable() {
        // 2 plugins; plugin 1 NotUsable -> any mask selecting bit 1 rejected.
        // SelectAny keeps the empty option. Valid masks: {}, {0}. Scores 0, 3.
        let group = FomodGroup {
            r#type: FomodGroupType::SelectAny,
            plugins: vec![plugin("P0"), plugin_typed("P1", PluginType::NotUsable)],
            ..FomodGroup::default()
        };
        let opts = generate_raw_options(&group, &[3, 9], 0, &no_flags(), 64);
        // {0} score 3 first, then {} score 0.
        assert_eq!(
            opts.iter().map(|o| selected(o)).collect::<Vec<_>>(),
            vec![vec![0], vec![],]
        );
    }

    #[test]
    fn raw_options_heuristic_path_for_large_group() {
        // 11 usable plugins with positive evidence -> heuristic path (n > 10).
        // The greedy "all usable-with-evidence" option must be present, plus
        // per-plugin singletons; and no invalid options.
        let plugins: Vec<FomodPlugin> = (0..11).map(|i| plugin(&format!("P{i}"))).collect();
        let group = FomodGroup {
            r#type: FomodGroupType::SelectAny,
            plugins,
            ..FomodGroup::default()
        };
        let evidence = vec![1i32; 11];
        let opts = generate_raw_options(&group, &evidence, 0, &no_flags(), 64);
        // Greedy = all 11 selected.
        let greedy: Vec<bool> = vec![true; 11];
        assert!(opts.contains(&greedy), "greedy all-selected option present");
        // Each singleton present.
        for i in 0..11 {
            let mut s = vec![false; 11];
            s[i] = true;
            assert!(opts.contains(&s), "singleton {i} present");
        }
    }

    #[test]
    fn raw_options_force_heuristic_gate_on_medium_no_evidence_select_any() {
        // Pins the exhaustive-vs-heuristic gate's SECONDARY trigger
        // (force_heuristic_select_any), which the other option tests never
        // exercise as true. The gate is `n <= 10 && !force_heuristic_select_any`,
        // where force_heuristic_select_any is
        //   cap > 0 && SelectAny && !has_required && positive_evidence == 0
        //   && n >= 8
        // so a medium (8..=10) no-evidence SelectAny group takes the greedy /
        // neighborhood heuristic even though n <= 10 would otherwise enumerate
        // the full powerset. Mirror of src/FomodCSPOptions.cpp:209-215.
        let make = |n: usize, required_first: bool| {
            let mut plugins: Vec<FomodPlugin> = (0..n).map(|i| plugin(&format!("P{i}"))).collect();
            if required_first {
                plugins[0] = plugin_typed("P0", PluginType::Required);
            }
            FomodGroup {
                r#type: FomodGroupType::SelectAny,
                plugins,
                ..FomodGroup::default()
            }
        };
        let ev = |n: usize| vec![0i32; n]; // zero evidence -> positive_evidence == 0

        // n = 8, cap > 0, no evidence, no required -> gate ON -> heuristic path.
        // Heuristic emits the empty greedy, 8 singletons, and the required-only
        // (empty) option: 10 raw options, none multi-select (no positive
        // evidence -> no greedy multi-select and no pair candidates). The
        // powerset path would instead yield 2^8 = 256 options.
        let g8 = make(8, false);
        let heuristic = generate_raw_options(&g8, &ev(8), 0, &no_flags(), 64);
        assert_eq!(
            heuristic.len(),
            10,
            "medium no-evidence SelectAny must take the heuristic path"
        );
        assert!(
            heuristic
                .iter()
                .all(|o| o.iter().filter(|&&b| b).count() <= 1),
            "heuristic path emits no multi-select option here"
        );
        for i in 0..8 {
            let mut s = vec![false; 8];
            s[i] = true;
            assert!(heuristic.contains(&s), "singleton {i} present");
        }
        assert!(heuristic.contains(&vec![false; 8]), "empty option present");

        // Same group, cap == 0 -> gate's `cap > 0` fails -> full powerset (2^8).
        let powerset = generate_raw_options(&g8, &ev(8), 0, &no_flags(), 0);
        assert_eq!(
            powerset.len(),
            256,
            "cap == 0 disables the force-heuristic gate (powerset)"
        );

        // n = 7 (< 8) with cap > 0 -> gate's `n >= 8` fails -> powerset (2^7).
        let g7 = make(7, false);
        let below = generate_raw_options(&g7, &ev(7), 0, &no_flags(), 64);
        assert_eq!(below.len(), 128, "n < 8 stays on the powerset path");

        // n = 8 with a Required plugin -> gate's `!has_required` fails ->
        // powerset. The required bit 0 must be set, so 2^7 = 128 valid masks.
        let g8req = make(8, true);
        let with_required = generate_raw_options(&g8req, &ev(8), 0, &no_flags(), 64);
        assert_eq!(
            with_required.len(),
            128,
            "has_required disables the force-heuristic gate (powerset)"
        );
    }

    #[test]
    fn raw_options_post_filter_drops_selected_notusable_via_local_flag() {
        // Plugin 0 sets flag M=1; plugin 1 becomes NotUsable when M==1 (type
        // pattern, flag-only so not externally dynamic). SelectAll selects both;
        // the post-filter drops it, and the empty-guarantee re-adds required
        // (none) -> a single all-false option.
        let mut p0 = plugin("P0");
        p0.condition_flags = vec![("M".to_string(), "1".to_string())];
        let mut p1 = plugin("P1");
        p1.type_patterns = vec![FomodTypePattern {
            condition: flag_cond("M", "1"),
            result_type: PluginType::NotUsable,
        }];
        let group = FomodGroup {
            r#type: FomodGroupType::SelectAll,
            plugins: vec![p0, p1],
            ..FomodGroup::default()
        };
        let opts = generate_raw_options(&group, &[0, 0], 0, &no_flags(), 64);
        // SelectAll wanted {0,1} but that is dropped; empty-guarantee yields the
        // required-only mask (no required plugins -> all false).
        assert_eq!(opts, vec![vec![false, false]]);
    }

    #[test]
    fn raw_options_dynamic_external_is_kept_and_not_dropped() {
        // Plugin 0 has a type pattern gated on an external File dependency
        // (dynamic_external). SelectAll keeps it even if inferred NotUsable.
        let mut p0 = plugin("P0");
        p0.type_patterns = vec![FomodTypePattern {
            condition: file_cond("marker.esp"),
            result_type: PluginType::NotUsable,
        }];
        let group = FomodGroup {
            r#type: FomodGroupType::SelectAll,
            plugins: vec![p0],
            ..FomodGroup::default()
        };
        // Inferred-Unknown: File leaf false -> pattern does not match -> base
        // Optional -> usable. dynamic_external demotion keeps usable regardless.
        let opts = generate_raw_options(&group, &[0], 0, &no_flags(), 64);
        assert_eq!(opts, vec![vec![true]]);
    }

    // --- option_signature (H3) --------------------------------------------

    #[test]
    fn option_signature_is_order_insensitive_and_byte_exact() {
        let mut a = OptionProfile::default();
        a.produced_atoms.insert("d2|s2".to_string());
        a.produced_atoms.insert("d1|s1".to_string());
        a.flags_written.insert("f".to_string(), "v".to_string());
        // Byte-exact constant computed offline from the C++ fold.
        assert_eq!(option_signature(&a), 0x72e8fd240ab91b68);

        // Same content, different insertion order -> identical signature.
        let mut b = OptionProfile::default();
        b.produced_atoms.insert("d1|s1".to_string());
        b.produced_atoms.insert("d2|s2".to_string());
        b.flags_written.insert("f".to_string(), "v".to_string());
        assert_eq!(option_signature(&a), option_signature(&b));

        // Different produced set -> different signature.
        let mut c = OptionProfile::default();
        c.produced_atoms.insert("d1|s1".to_string());
        c.flags_written.insert("f".to_string(), "v".to_string());
        assert_ne!(option_signature(&a), option_signature(&c));
    }

    #[test]
    fn better_equivalent_option_lexicographic() {
        let base = OptionProfile {
            evidence_score: 5,
            unique_support: 3,
            useful_dests: 2,
            extra_dests: 1,
            ..OptionProfile::default()
        };
        // Higher evidence wins.
        assert!(better_equivalent_option(
            &OptionProfile {
                evidence_score: 6,
                ..base.clone()
            },
            &base
        ));
        // Equal evidence, higher unique wins.
        assert!(better_equivalent_option(
            &OptionProfile {
                unique_support: 4,
                ..base.clone()
            },
            &base
        ));
        // Equal evidence+unique+useful, fewer extra wins.
        assert!(better_equivalent_option(
            &OptionProfile {
                extra_dests: 0,
                ..base.clone()
            },
            &base
        ));
        // Equal on all -> not strictly better.
        assert!(!better_equivalent_option(&base, &base));
    }

    // --- reduce_options via build_precompute state ------------------------

    /// Build a Precompute for a single-step installer with doc-order GroupRefs.
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

    fn index_of(atoms: &[FomodAtom]) -> AtomIndex {
        let mut idx: AtomIndex = HashMap::new();
        for a in atoms {
            idx.entry(a.dest_path.clone()).or_default().push(a.clone());
        }
        idx
    }

    #[test]
    fn reduce_options_drops_extra_only() {
        // Plugin 0 produces "u" (in target); plugin 1 produces "x" (not in
        // target, extra-only). SelectAny; the {1}-only option is extra-only and
        // dropped; the extra-only counter increments.
        let installer = single_group(FomodGroupType::SelectAny, vec![plugin("P0"), plugin("P1")]);
        let a0 = plugin_atom("u", "s0", 0);
        let a1 = plugin_atom("x", "s1", 1);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()], vec![a1.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0, a1]);
        let target: TargetTree = [("u".to_string(), TargetFile { size: 0, hash: 0 })]
            .into_iter()
            .collect();
        let excluded = HashSet::new();
        let evidence = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_order_group_refs(&installer),
            evidence,
        );
        let gref = pre.groups[0];
        // Raw powerset options: {}, {0}, {1}, {0,1}.
        let raw = generate_raw_options(
            &installer.steps[0].groups[0],
            &pre.evidence,
            0,
            &no_flags(),
            64,
        );
        let mut stats = SolverStats::default();
        let reduced = reduce_options(&gref, &raw, &pre, &mut stats, 64, false);
        // The {1}-only option (extra-only) is dropped.
        assert!(stats.dropped_extra_only_options >= 1);
        for opt in &reduced {
            assert_ne!(selected(opt), vec![1], "extra-only {{1}} must be dropped");
        }
    }

    #[test]
    fn reduce_options_collapses_equivalent_signatures() {
        // Two plugins produce the SAME dest+source (identical output atoms) with
        // no target overlap forcing a drop -> the {0} and {1} singletons share a
        // signature and collapse. Use a target dest so they are not extra-only.
        let installer = single_group(
            FomodGroupType::SelectExactlyOne,
            vec![plugin("P0"), plugin("P1")],
        );
        let a0 = plugin_atom("same", "src", 0);
        let a1 = plugin_atom("same", "src", 1);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()], vec![a1.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0, a1]);
        let target: TargetTree = [("same".to_string(), TargetFile { size: 0, hash: 0 })]
            .into_iter()
            .collect();
        let excluded = HashSet::new();
        // Give plugin 0 higher evidence so it is the "better equivalent".
        let evidence = vec![5, 1];
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_order_group_refs(&installer),
            evidence,
        );
        let gref = pre.groups[0];
        let raw = generate_raw_options(
            &installer.steps[0].groups[0],
            &pre.evidence,
            0,
            &no_flags(),
            64,
        );
        // SelectExactlyOne -> singletons {0}, {1}, identical produced_atoms
        // ("same|src") -> collapse to one, keeping the higher-evidence plugin 0.
        let mut stats = SolverStats::default();
        let reduced = reduce_options(&gref, &raw, &pre, &mut stats, 64, false);
        assert_eq!(stats.collapsed_equivalent_options, 1);
        assert_eq!(reduced.len(), 1);
        assert_eq!(selected(&reduced[0]), vec![0]);
    }

    #[test]
    fn reduce_options_select_any_cap_diversity_then_fill() {
        // Construct > cap distinct-signature candidates and cap to 3. Diversity
        // sampling keeps rank 0, then the first new selected-count at each rank,
        // then fills by rank. Assert exactly `cap` kept and the cap counter.
        //
        // 4 plugins each producing a distinct useful dest, distinct evidence so
        // the ordering is total. SelectAny -> powerset (2^4 = 16 masks). With
        // cap = 3 we must trim to 3.
        let installer = single_group(
            FomodGroupType::SelectAny,
            vec![plugin("P0"), plugin("P1"), plugin("P2"), plugin("P3")],
        );
        let a0 = plugin_atom("d0", "s0", 0);
        let a1 = plugin_atom("d1", "s1", 1);
        let a2 = plugin_atom("d2", "s2", 2);
        let a3 = plugin_atom("d3", "s3", 3);
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
            ("d0".to_string(), TargetFile { size: 0, hash: 0 }),
            ("d1".to_string(), TargetFile { size: 0, hash: 0 }),
            ("d2".to_string(), TargetFile { size: 0, hash: 0 }),
            ("d3".to_string(), TargetFile { size: 0, hash: 0 }),
        ]
        .into_iter()
        .collect();
        let excluded = HashSet::new();
        let evidence = vec![4, 3, 2, 1];
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_order_group_refs(&installer),
            evidence,
        );
        let gref = pre.groups[0];
        let raw = generate_raw_options(
            &installer.steps[0].groups[0],
            &pre.evidence,
            0,
            &no_flags(),
            0, // uncapped raw enumeration; cap applied in reduce_options
        );
        let candidate_count_before = {
            // Reduce with no cap to learn how many survive.
            let mut s = SolverStats::default();
            reduce_options(&gref, &raw, &pre, &mut s, 0, false).len()
        };
        assert!(candidate_count_before > 3, "need more than cap candidates");

        let mut stats = SolverStats::default();
        let reduced = reduce_options(&gref, &raw, &pre, &mut stats, 3, false);
        assert_eq!(reduced.len(), 3, "capped to exactly cap");
        assert_eq!(
            stats.capped_select_any_options,
            (candidate_count_before - 3) as i32
        );
        // Rank 0 (highest score = all four selected) is always kept.
        assert!(reduced.contains(&vec![true, true, true, true]));
    }

    // --- get_options_for_group cache + propagation -------------------------

    fn build_pre_two_plugin_selectany<'a>(
        installer: &'a FomodInstaller,
        atoms: &'a ExpandedAtoms,
        index: &'a AtomIndex,
        target: &'a TargetTree,
        excluded: &'a HashSet<String>,
        propagation: Option<&'a PropagationResult>,
    ) -> Precompute<'a> {
        let evidence = compute_evidence(installer, atoms, index, target, excluded);
        build_precompute(
            installer,
            atoms,
            index,
            target,
            excluded,
            None,
            propagation,
            doc_order_group_refs(installer),
            evidence,
        )
    }

    #[test]
    fn get_options_cache_hit_returns_same_and_cap_change_misses() {
        let installer = single_group(FomodGroupType::SelectAny, vec![plugin("P0"), plugin("P1")]);
        let a0 = plugin_atom("d0", "s0", 0);
        let a1 = plugin_atom("d1", "s1", 1);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()], vec![a1.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0, a1]);
        let target: TargetTree = [
            ("d0".to_string(), TargetFile { size: 0, hash: 0 }),
            ("d1".to_string(), TargetFile { size: 0, hash: 0 }),
        ]
        .into_iter()
        .collect();
        let excluded = HashSet::new();
        let pre =
            build_pre_two_plugin_selectany(&installer, &atoms, &index, &target, &excluded, None);

        let mut cache: HashMap<OptionCacheKey, CachedOptions> = HashMap::new();
        let mut stats = SolverStats {
            logged_group_options: vec![false; pre.groups.len()],
            ..SolverStats::default()
        };

        let opts1 =
            get_options_for_group(0, &pre, &no_flags(), 64, None, &mut cache, &mut stats).clone();
        assert_eq!(cache.len(), 1);
        // Cache hit: same key, no new entry.
        let opts2 =
            get_options_for_group(0, &pre, &no_flags(), 64, None, &mut cache, &mut stats).clone();
        assert_eq!(cache.len(), 1);
        assert_eq!(opts1, opts2);

        // A different effective cap misses (new cache entry).
        let _ = get_options_for_group(0, &pre, &no_flags(), 256, None, &mut cache, &mut stats);
        assert_eq!(cache.len(), 2);

        // Exact mode misses too (different key).
        let exact: HashSet<i32> = [0].into_iter().collect();
        let _ = get_options_for_group(
            0,
            &pre,
            &no_flags(),
            64,
            Some(&exact),
            &mut cache,
            &mut stats,
        );
        assert_eq!(cache.len(), 3);

        // Every returned option is a valid mask of the group plugin count.
        for opt in &opts1.options {
            assert_eq!(opt.len(), 2);
        }
        // logged_group_options marked.
        assert!(stats.logged_group_options[0]);
    }

    #[test]
    fn get_options_out_of_range_returns_empty() {
        let installer = single_group(FomodGroupType::SelectAny, vec![plugin("P0")]);
        let a0 = plugin_atom("d0", "s0", 0);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0]);
        let target: TargetTree = HashMap::new();
        let excluded = HashSet::new();
        let pre =
            build_pre_two_plugin_selectany(&installer, &atoms, &index, &target, &excluded, None);
        let mut cache = HashMap::new();
        let mut stats = SolverStats::default();
        let opts = get_options_for_group(99, &pre, &no_flags(), 64, None, &mut cache, &mut stats);
        assert!(opts.options.is_empty());
        assert!(opts.profiles.is_empty());
        assert!(cache.is_empty(), "OOR must not populate the cache");
    }

    #[test]
    fn get_options_propagation_narrowing_drops_pruned_plugin() {
        // SelectExactlyOne over 2 plugins; propagation prunes plugin 1. Options
        // selecting plugin 1 must be dropped.
        let installer = single_group(
            FomodGroupType::SelectExactlyOne,
            vec![plugin("P0"), plugin("P1")],
        );
        let a0 = plugin_atom("d0", "s0", 0);
        let a1 = plugin_atom("d1", "s1", 1);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![a0.clone()], vec![a1.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[a0, a1]);
        let target: TargetTree = [
            ("d0".to_string(), TargetFile { size: 0, hash: 0 }),
            ("d1".to_string(), TargetFile { size: 0, hash: 0 }),
        ]
        .into_iter()
        .collect();
        let excluded = HashSet::new();

        // Propagation: plugin 0 usable, plugin 1 pruned.
        let prop = PropagationResult {
            narrowed_domains: vec![vec![vec![true, false]]],
            ..PropagationResult::default()
        };
        let pre = build_pre_two_plugin_selectany(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            Some(&prop),
        );
        let mut cache = HashMap::new();
        let mut stats = SolverStats {
            logged_group_options: vec![false; pre.groups.len()],
            ..SolverStats::default()
        };
        let opts = get_options_for_group(0, &pre, &no_flags(), 64, None, &mut cache, &mut stats);
        // No surviving option may select plugin 1.
        for opt in &opts.options {
            assert!(
                !(opt.len() > 1 && opt[1]),
                "pruned plugin 1 must not appear"
            );
        }
        // At least the {0} option survives.
        assert!(opts.options.iter().any(|o| selected(o) == vec![0]));
    }
}
