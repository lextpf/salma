//! FOMOD forward simulator + reproduction metrics.
//!
//! Rust port of `src/FomodForwardSimulator.hpp`/`.cpp` (the [`SimulatedTree`],
//! [`simulate`], [`simulate_into`] surface) PLUS the tree-comparison helpers
//! whose C++ home is `src/FomodCSPSolver.cpp`:
//! [`compare_trees_impl`]/[`compare_trees`]/[`collect_mismatched_dests`]. Those
//! three are placed here (not with the solver) because they read a
//! [`SimulatedTree`] and are the simulator's natural scoring companions; Task 9
//! (CSP solver phases) may re-home them, and its `lower_bound` pruning MUST
//! reuse the generic [`compare_trees_impl`] predicate variant rather than
//! duplicate the else-chain.
//!
//! The simulator replicates the priority and document-order conflict
//! resolution of `FomodService::execute_file_operations`, but produces an
//! in-memory file tree instead of writing to disk. It is the scoring oracle
//! every later CSP solver stage depends on: a candidate `[step][group][plugin]`
//! selection grid is replayed through the four phases below, then the resulting
//! tree is diffed against the real target tree via [`compare_trees`].
//!
//! ## Phases (mirror of the four blocks in `simulate_into`)
//!
//! 1. **Required** - every `atoms.required` atom applied unconditionally, in
//!    vector order.
//! 2. **Selected / Required-typed plugins** (chronological pass) - per step in
//!    order, in a VISIBLE step, per group, per plugin: `selected` comes from
//!    the bounds-checked grid; an unselected plugin whose `evaluate_plugin_type`
//!    against the flag state SO FAR is `Required` is promoted. A selected
//!    plugin first accumulates its `condition_flags` (last-write-wins per
//!    name), then applies ALL of its atoms unconditionally (no
//!    always_install/install_if_usable filter). `flat_idx` advances per plugin
//!    ALWAYS. An invisible step advances `flat_idx` past every group's plugins
//!    but applies nothing and sets no flags.
//! 3. **Auto atoms of unselected plugins** (final-flag-state pass) - `flat_idx`
//!    recomputed from 0; step visibility RE-EVALUATED against the now-final
//!    flags (so a step invisible in phase 2 can become visible here and vice
//!    versa). For each plugin not processed in phase 2, `eff_type` is evaluated
//!    against the FINAL flags: `always_install` atoms apply unconditionally;
//!    `install_if_usable` atoms apply only when `eff_type != NotUsable`.
//! 4. **Conditional installs** - per conditional pattern, active via
//!    `evaluate_condition_inferred` (when an override entry exists) or
//!    `evaluate_condition` (normal), applied in order when active.
//!
//! Visibility is re-evaluated in phase 3 rather than cached from phase 2
//! because the flag map grows as phase 2 runs; the C++ comments on the phase-2
//! and phase-3 blocks make this explicit ("eff_type ... against the flag state
//! at this plugin's position" vs "against the FINAL flag state").

use std::collections::{HashMap, HashSet};

use crate::fomod_atom::{ExpandedAtoms, FomodAtom, TargetTree};
use crate::fomod_csp_types::{InferenceOverrides, ReproMetrics};
use crate::fomod_dependency_evaluator::{
    evaluate_condition, evaluate_condition_inferred, evaluate_plugin_type,
};
use crate::fomod_ir::{FomodInstaller, FomodStep};
use crate::types::{FomodDependencyContext, PluginType};

/// The file tree produced by a simulated FOMOD installation. Mirror of
/// `mo2core::SimulatedTree`.
///
/// Maps each lowercased destination path to the single winning [`FomodAtom`]
/// after priority and document-order conflict resolution. Used by the CSP
/// solver to compare a candidate selection against the real target tree
/// without performing any actual extraction.
#[derive(Debug, Clone, Default)]
pub struct SimulatedTree {
    /// dest -> winning atom.
    pub files: HashMap<String, FomodAtom>,
}

/// An atom overwrites the existing entry if it has `>=` priority. Mirror of the
/// C++ file-scoped `should_overwrite`.
///
/// Because atoms are applied in increasing document order (required, then
/// plugins, then conditionals - see the module doc), equal priority means the
/// new atom came later in the XML and should win: last-writer-wins.
fn should_overwrite(existing: &FomodAtom, new_atom: &FomodAtom) -> bool {
    new_atom.priority >= existing.priority
}

/// Insert `atom` at its destination when the slot is empty OR the incumbent
/// loses to it under [`should_overwrite`]. Mirror of the C++ `apply_atom`
/// (`insert_or_assign` on absent-or-overwrite).
fn apply_atom(tree: &mut SimulatedTree, atom: &FomodAtom) {
    let overwrite = match tree.files.get(&atom.dest_path) {
        Some(existing) => should_overwrite(existing, atom),
        None => true,
    };
    if overwrite {
        tree.files.insert(atom.dest_path.clone(), atom.clone());
    }
}

/// Step-visibility decision shared by phases 2 and 3, evaluated against the
/// CURRENT flag map. Mirror of the C++ `compute_step_visibility` closure:
///
/// - no visibility condition -> `true`;
/// - overrides present AND `si` in range -> `evaluate_condition_inferred` with
///   `overrides.step_visible[si]`;
/// - otherwise (including overrides present but the vector too short) ->
///   `evaluate_condition` in normal mode.
fn compute_step_visibility(
    si: usize,
    step: &FomodStep,
    flags: &HashMap<String, String>,
    context: Option<&FomodDependencyContext>,
    overrides: Option<&InferenceOverrides>,
) -> bool {
    let Some(visible) = &step.visible else {
        return true;
    };
    match overrides {
        Some(ov) if si < ov.step_visible.len() => {
            evaluate_condition_inferred(visible, flags, ov.step_visible[si], context)
        }
        _ => evaluate_condition(visible, flags, context),
    }
}

/// Run a forward simulation for a given selection, producing a fresh tree.
/// Mirror of `mo2core::simulate`.
///
/// `selections` is a 3-D boolean grid indexed `selections[step][group][plugin]`.
/// Dimensions need not match the installer's counts; missing or short axes are
/// treated as `false` (deselected). An empty outer slice is valid and means no
/// plugins are explicitly selected (Required promotion and auto atoms still
/// run).
pub fn simulate(
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    selections: &[Vec<Vec<bool>>],
    context: Option<&FomodDependencyContext>,
    overrides: Option<&InferenceOverrides>,
) -> SimulatedTree {
    let mut tree = SimulatedTree::default();
    simulate_into(&mut tree, installer, atoms, selections, context, overrides);
    tree
}

/// In-place version of [`simulate`] that reuses an existing tree's allocation.
/// Mirror of `mo2core::simulate_into`.
///
/// `tree.files` is cleared before simulation begins (capacity is preserved, as
/// in the C++ `unordered_map::clear`), then repopulated with the result.
pub fn simulate_into(
    tree: &mut SimulatedTree,
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    selections: &[Vec<Vec<bool>>],
    context: Option<&FomodDependencyContext>,
    overrides: Option<&InferenceOverrides>,
) {
    tree.files.clear();
    let mut flags: HashMap<String, String> = HashMap::new();

    // Phase 1: Required files.
    for atom in &atoms.required {
        apply_atom(tree, atom);
    }

    // Phase 2: chronological pass for selected / Required-typed plugins.
    let mut processed_in_phase2 = vec![false; atoms.per_plugin.len()];
    let mut flat_idx: usize = 0;
    for (si, step) in installer.steps.iter().enumerate() {
        if !compute_step_visibility(si, step, &flags, context, overrides) {
            // Skip all groups in this invisible step, but still advance flat_idx.
            for group in &step.groups {
                flat_idx += group.plugins.len();
            }
            continue;
        }

        for (gi, group) in step.groups.iter().enumerate() {
            for (pi, plugin) in group.plugins.iter().enumerate() {
                let mut selected = selections
                    .get(si)
                    .and_then(|g| g.get(gi))
                    .and_then(|p| p.get(pi))
                    .copied()
                    .unwrap_or(false);

                if !selected
                    && evaluate_plugin_type(plugin, &flags, context) == PluginType::Required
                {
                    selected = true;
                }

                if selected && flat_idx < atoms.per_plugin.len() {
                    for (name, value) in &plugin.condition_flags {
                        flags.insert(name.clone(), value.clone());
                    }
                    for atom in &atoms.per_plugin[flat_idx] {
                        apply_atom(tree, atom);
                    }
                    processed_in_phase2[flat_idx] = true;
                }
                flat_idx += 1;
            }
        }
    }

    // Phase 3: final-flag-state pass for unselected non-Required plugins.
    flat_idx = 0;
    for (si, step) in installer.steps.iter().enumerate() {
        if !compute_step_visibility(si, step, &flags, context, overrides) {
            for group in &step.groups {
                flat_idx += group.plugins.len();
            }
            continue;
        }

        for group in &step.groups {
            for plugin in &group.plugins {
                if flat_idx >= atoms.per_plugin.len() || processed_in_phase2[flat_idx] {
                    flat_idx += 1;
                    continue;
                }
                let eff_type = evaluate_plugin_type(plugin, &flags, context);
                for atom in &atoms.per_plugin[flat_idx] {
                    // Mirror of the C++ else-if: always_install applies
                    // unconditionally; install_if_usable applies only when the
                    // effective type is not NotUsable. Both branches call
                    // apply_atom, so they collapse to a single OR condition.
                    if atom.always_install
                        || (atom.install_if_usable && eff_type != PluginType::NotUsable)
                    {
                        apply_atom(tree, atom);
                    }
                }
                flat_idx += 1;
            }
        }
    }

    // Phase 4: conditional file installs.
    for (ci, pattern) in installer.conditional_patterns.iter().enumerate() {
        let active = match overrides {
            Some(ov) if ci < ov.conditional_active.len() => evaluate_condition_inferred(
                &pattern.condition,
                &flags,
                ov.conditional_active[ci],
                context,
            ),
            _ => evaluate_condition(&pattern.condition, &flags, context),
        };
        if active && ci < atoms.per_conditional.len() {
            for atom in &atoms.per_conditional[ci] {
                apply_atom(tree, atom);
            }
        }
    }
}

/// Generic tree comparison: walk `sim` vs `target`, calling the three
/// predicates to decide whether each divergence is counted. Mirror of the C++
/// `compare_trees_impl` template.
///
/// The predicates let callers gate mismatch counting: [`compare_trees`] passes
/// always-true, while Task 9's `lower_bound` will pass "is there a group still
/// able to fix this dest" checks. Each predicate receives the destination path.
///
/// The size/hash/reproduced else-chain is load-bearing and mirrors the C++
/// exactly: a size mismatch (both sizes nonzero and differing) is counted and
/// SUPPRESSES the hash check; otherwise a hash mismatch (both hashes nonzero
/// and differing) is counted; otherwise the file is reproduced. A zero size or
/// zero hash on either side falls through toward reproduced.
pub fn compare_trees_impl(
    sim: &SimulatedTree,
    target: &TargetTree,
    excluded: &HashSet<String>,
    mut on_missing: impl FnMut(&str) -> bool,
    mut on_size_mismatch: impl FnMut(&str) -> bool,
    mut on_hash_mismatch: impl FnMut(&str) -> bool,
) -> ReproMetrics {
    let mut m = ReproMetrics::default();

    for (dest, tf) in target {
        if excluded.contains(dest) {
            continue;
        }
        match sim.files.get(dest) {
            None => {
                if on_missing(dest) {
                    m.missing += 1;
                }
            }
            Some(atom) => {
                if tf.size != 0 && atom.file_size != 0 && tf.size != atom.file_size {
                    if on_size_mismatch(dest) {
                        m.size_mismatch += 1;
                    }
                } else if tf.hash != 0 && atom.content_hash != 0 && tf.hash != atom.content_hash {
                    if on_hash_mismatch(dest) {
                        m.hash_mismatch += 1;
                    }
                } else {
                    m.reproduced += 1;
                }
            }
        }
    }

    for dest in sim.files.keys() {
        if excluded.contains(dest) {
            continue;
        }
        if !target.contains_key(dest) {
            m.extra += 1;
        }
    }
    m
}

/// Compare a simulated tree against the target, counting every divergence.
/// Mirror of the C++ `compare_trees` (the always-true-predicate wrapper over
/// [`compare_trees_impl`]).
pub fn compare_trees(
    sim: &SimulatedTree,
    target: &TargetTree,
    excluded: &HashSet<String>,
) -> ReproMetrics {
    compare_trees_impl(sim, target, excluded, |_| true, |_| true, |_| true)
}

/// Collect every destination where the simulation diverges from the target.
/// Mirror of the C++ `collect_mismatched_dests`.
///
/// Categories (same else-chain as [`compare_trees_impl`]): a dest in target but
/// not in sim is `missing`; a dest in both with differing nonzero sizes is a
/// size mismatch; else with differing nonzero hashes a hash mismatch; a dest in
/// sim but not in target is `extra`. Reproduced dests are not collected.
///
/// Ordering: the C++ collects into an `unordered_set` (unspecified order) then
/// `std::sort`s the result vector before returning. This port collects into a
/// [`HashSet`] then sorts into a `Vec<String>`, yielding the same deterministic
/// byte-wise ascending order. Task 9's consumers
/// (`groups_for_mismatches`) only iterate the result and look up
/// `dest_to_groups`, so they depend on determinism, not on a particular order.
pub fn collect_mismatched_dests(
    sim: &SimulatedTree,
    target: &TargetTree,
    excluded: &HashSet<String>,
) -> Vec<String> {
    let mut out: HashSet<String> = HashSet::new();

    for (dest, tf) in target {
        if excluded.contains(dest) {
            continue;
        }
        match sim.files.get(dest) {
            None => {
                out.insert(dest.clone()); // missing
            }
            Some(atom) => {
                if tf.size != 0 && atom.file_size != 0 && tf.size != atom.file_size {
                    out.insert(dest.clone()); // size mismatch
                } else if tf.hash != 0 && atom.content_hash != 0 && tf.hash != atom.content_hash {
                    out.insert(dest.clone()); // hash mismatch
                }
            }
        }
    }

    for dest in sim.files.keys() {
        if excluded.contains(dest) {
            continue;
        }
        if !target.contains_key(dest) {
            out.insert(dest.clone()); // extra
        }
    }

    let mut mismatched: Vec<String> = out.into_iter().collect();
    mismatched.sort();
    mismatched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_atom::{Origin, TargetFile};
    use crate::fomod_ir::{
        FomodCondition, FomodConditionOp, FomodConditionType, FomodConditionalPattern, FomodGroup,
        FomodPlugin, FomodTypePattern,
    };

    // ------------------------------------------------------------------
    // Builders for synthetic IR + atoms.
    // ------------------------------------------------------------------

    fn atom(dest: &str, source: &str, priority: i32, doc: i32) -> FomodAtom {
        FomodAtom {
            source_path: source.to_string(),
            dest_path: dest.to_string(),
            priority,
            document_order: doc,
            ..FomodAtom::default()
        }
    }

    fn plugin(name: &str, ptype: PluginType) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            r#type: ptype,
            ..FomodPlugin::default()
        }
    }

    fn flag_leaf(name: &str, value: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Flag,
            flag_name: name.to_string(),
            flag_value: value.to_string(),
            ..FomodCondition::default()
        }
    }

    fn file_leaf(path: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::File,
            file_path: path.to_string(),
            file_state: "Active".to_string(),
            ..FomodCondition::default()
        }
    }

    /// An always-true condition: an empty `And` composite (start true, no
    /// children). Used to force a type_pattern to always match.
    fn always_true() -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Composite,
            op: FomodConditionOp::And,
            ..FomodCondition::default()
        }
    }

    fn single_step(groups: Vec<FomodGroup>) -> FomodStep {
        FomodStep {
            groups,
            ..FomodStep::default()
        }
    }

    fn group(plugins: Vec<FomodPlugin>) -> FomodGroup {
        FomodGroup {
            plugins,
            ..FomodGroup::default()
        }
    }

    /// Build an installer from a list of steps, plus optional conditional
    /// patterns. `atoms.per_plugin` must be sized to the flat plugin count by
    /// the caller.
    fn installer(
        steps: Vec<FomodStep>,
        conditionals: Vec<FomodConditionalPattern>,
    ) -> FomodInstaller {
        FomodInstaller {
            steps,
            conditional_patterns: conditionals,
            ..FomodInstaller::default()
        }
    }

    fn dests(tree: &SimulatedTree) -> Vec<String> {
        let mut v: Vec<String> = tree.files.keys().cloned().collect();
        v.sort();
        v
    }

    fn winner_source<'a>(tree: &'a SimulatedTree, dest: &str) -> &'a str {
        &tree.files.get(dest).expect("dest present").source_path
    }

    // ------------------------------------------------------------------
    // Overwrite matrix (proves the `>=` rule and application-order dependence).
    // ------------------------------------------------------------------

    #[test]
    fn higher_priority_wins_regardless_of_application_order() {
        // Two required atoms on the same dest, different priorities. Phase 1
        // applies them in vector order, so we can flip the order to prove that
        // the higher priority wins either way (a lower-priority late atom fails
        // the >= test; a higher-priority late atom passes it).
        let inst = installer(vec![], vec![]);

        let atoms_hi_first = ExpandedAtoms {
            required: vec![atom("d", "hi", 5, 0), atom("d", "lo", 3, 1)],
            ..ExpandedAtoms::default()
        };
        let t = simulate(&inst, &atoms_hi_first, &[], None, None);
        assert_eq!(winner_source(&t, "d"), "hi");

        let atoms_lo_first = ExpandedAtoms {
            required: vec![atom("d", "lo", 3, 0), atom("d", "hi", 5, 1)],
            ..ExpandedAtoms::default()
        };
        let t = simulate(&inst, &atoms_lo_first, &[], None, None);
        assert_eq!(winner_source(&t, "d"), "hi");
    }

    #[test]
    fn equal_priority_last_applied_wins_proving_gte_not_gt() {
        let inst = installer(vec![], vec![]);

        let a_then_b = ExpandedAtoms {
            required: vec![atom("d", "A", 2, 0), atom("d", "B", 2, 1)],
            ..ExpandedAtoms::default()
        };
        assert_eq!(
            winner_source(&simulate(&inst, &a_then_b, &[], None, None), "d"),
            "B"
        );

        let b_then_a = ExpandedAtoms {
            required: vec![atom("d", "B", 2, 0), atom("d", "A", 2, 1)],
            ..ExpandedAtoms::default()
        };
        assert_eq!(
            winner_source(&simulate(&inst, &b_then_a, &[], None, None), "d"),
            "A"
        );
    }

    // ------------------------------------------------------------------
    // Required-vs-plugin conflict across phases.
    // ------------------------------------------------------------------

    #[test]
    fn selected_plugin_overwrites_equal_priority_required_because_phase2_is_later() {
        // Phase 1 applies the required atom; phase 2 applies the selected
        // plugin's atom later, so on equal priority the plugin wins (>=).
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let atoms = ExpandedAtoms {
            required: vec![atom("d", "req", 0, 0)],
            per_plugin: vec![vec![atom("d", "plug", 0, 1)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        assert_eq!(
            winner_source(&simulate(&inst, &atoms, &sel, None, None), "d"),
            "plug"
        );
    }

    #[test]
    fn higher_priority_required_beats_selected_plugin() {
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let atoms = ExpandedAtoms {
            required: vec![atom("d", "req", 9, 0)],
            per_plugin: vec![vec![atom("d", "plug", 1, 1)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        assert_eq!(
            winner_source(&simulate(&inst, &atoms, &sel, None, None), "d"),
            "req"
        );
    }

    // ------------------------------------------------------------------
    // Flag accumulation order + last-write-wins per name.
    // ------------------------------------------------------------------

    #[test]
    fn condition_flags_last_write_wins_within_a_plugin() {
        // A selected plugin sets f=a then f=b; only the conditional gated on
        // f==b must fire.
        let mut p = plugin("P", PluginType::Optional);
        p.condition_flags = vec![("f".into(), "a".into()), ("f".into(), "b".into())];
        let inst = installer(
            vec![single_step(vec![group(vec![p])])],
            vec![
                FomodConditionalPattern {
                    condition: flag_leaf("f", "b"),
                    ..FomodConditionalPattern::default()
                },
                FomodConditionalPattern {
                    condition: flag_leaf("f", "a"),
                    ..FomodConditionalPattern::default()
                },
            ],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![]],
            per_conditional: vec![
                vec![atom("cond_b", "cb", 0, 10)],
                vec![atom("cond_a", "ca", 0, 11)],
            ],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        let t = simulate(&inst, &atoms, &sel, None, None);
        assert_eq!(dests(&t), vec!["cond_b".to_string()]);
    }

    #[test]
    fn condition_flags_accumulate_across_selected_plugins() {
        // plugin0 (step0) sets f=x; plugin1 (step1) sets f=y; final f=y.
        let mut p0 = plugin("P0", PluginType::Optional);
        p0.condition_flags = vec![("f".into(), "x".into())];
        let mut p1 = plugin("P1", PluginType::Optional);
        p1.condition_flags = vec![("f".into(), "y".into())];
        let inst = installer(
            vec![
                single_step(vec![group(vec![p0])]),
                single_step(vec![group(vec![p1])]),
            ],
            vec![FomodConditionalPattern {
                condition: flag_leaf("f", "y"),
                ..FomodConditionalPattern::default()
            }],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![], vec![]],
            per_conditional: vec![vec![atom("done", "d", 0, 20)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]], vec![vec![true]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["done".to_string()]
        );
    }

    // ------------------------------------------------------------------
    // Required-type promotion via evaluate_plugin_type mid-walk.
    // ------------------------------------------------------------------

    #[test]
    fn type_pattern_promotes_a_later_plugin_to_required_from_earlier_flag() {
        // plugin0 selected, sets want=yes. plugin1 (later, same step) is
        // Optional but a type_pattern flips it to Required when want==yes. It is
        // NOT selected in the grid, yet its atom must install via promotion.
        let mut p0 = plugin("P0", PluginType::Optional);
        p0.condition_flags = vec![("want".into(), "yes".into())];
        let mut p1 = plugin("P1", PluginType::Optional);
        p1.type_patterns = vec![FomodTypePattern {
            condition: flag_leaf("want", "yes"),
            result_type: PluginType::Required,
        }];
        let inst = installer(vec![single_step(vec![group(vec![p0, p1])])], vec![]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![], vec![atom("promoted", "pr", 0, 5)]],
            ..ExpandedAtoms::default()
        };

        // p0 selected, p1 deselected -> p1 promoted, its atom installs.
        let sel = vec![vec![vec![true, false]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["promoted".to_string()]
        );

        // p0 NOT selected -> want unset -> p1 stays Optional -> nothing installs
        // (its lone atom is a normal, non-auto atom that never applies unselected).
        let sel_none = vec![vec![vec![false, false]]];
        assert!(
            simulate(&inst, &atoms, &sel_none, None, None)
                .files
                .is_empty()
        );
    }

    // ------------------------------------------------------------------
    // Invisible step: advances flat_idx, leaks neither flags nor atoms.
    // ------------------------------------------------------------------

    #[test]
    fn invisible_step_advances_flat_idx_and_leaks_nothing() {
        // step0 invisible (gated on show=1, never set). Its plugin is marked
        // selected and would set leak=1 and install "leaked". step1 visible
        // with a selected plugin installing "kept". The invisible step must
        // install nothing, set no flag, yet advance flat_idx so step1's plugin
        // maps to per_plugin[1] (not [0]).
        let mut hidden = plugin("Hidden", PluginType::Optional);
        hidden.condition_flags = vec![("leak".into(), "1".into())];
        let step0 = FomodStep {
            visible: Some(flag_leaf("show", "1")),
            groups: vec![group(vec![hidden])],
            ..FomodStep::default()
        };
        let step1 = single_step(vec![group(vec![plugin("Kept", PluginType::Optional)])]);
        let inst = installer(
            vec![step0, step1],
            vec![FomodConditionalPattern {
                condition: flag_leaf("leak", "1"),
                ..FomodConditionalPattern::default()
            }],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![
                vec![atom("leaked", "lk", 0, 0)],
                vec![atom("kept", "kp", 0, 1)],
            ],
            per_conditional: vec![vec![atom("leak_cond", "lc", 0, 2)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]], vec![vec![true]]];
        let t = simulate(&inst, &atoms, &sel, None, None);
        // Only "kept" installs: no leaked atom, no leak_cond (flag never set).
        assert_eq!(dests(&t), vec!["kept".to_string()]);
    }

    // ------------------------------------------------------------------
    // Phase-3 visibility flip, both directions.
    // ------------------------------------------------------------------

    #[test]
    fn phase3_step_becomes_visible_with_final_flags_and_auto_atoms_apply() {
        // step0 gated on f=1 (unset during phase 2 -> invisible there). step1's
        // selected plugin sets f=1. In phase 3, step0 is now visible, so its
        // unselected plugin's always_install atom applies.
        let step0 = FomodStep {
            visible: Some(flag_leaf("f", "1")),
            groups: vec![group(vec![plugin("Auto", PluginType::Optional)])],
            ..FomodStep::default()
        };
        let mut setter = plugin("Setter", PluginType::Optional);
        setter.condition_flags = vec![("f".into(), "1".into())];
        let step1 = single_step(vec![group(vec![setter])]);
        let inst = installer(vec![step0, step1], vec![]);

        let mut auto_atom = atom("auto_out", "ao", 0, 0);
        auto_atom.always_install = true;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![auto_atom], vec![]],
            ..ExpandedAtoms::default()
        };
        // step0 plugin unselected; step1 setter selected.
        let sel = vec![vec![vec![false]], vec![vec![true]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["auto_out".to_string()]
        );
    }

    #[test]
    fn phase3_step_becomes_invisible_with_final_flags_and_auto_atoms_do_not_apply() {
        // step0 gated on hide being empty/absent (true during phase 2 ->
        // visible). step1's selected plugin sets hide=1. In phase 3 step0 turns
        // invisible, so its unselected plugin's always_install atom must NOT
        // apply.
        let step0 = FomodStep {
            visible: Some(flag_leaf("hide", "")),
            groups: vec![group(vec![plugin("Auto", PluginType::Optional)])],
            ..FomodStep::default()
        };
        let mut setter = plugin("Setter", PluginType::Optional);
        setter.condition_flags = vec![("hide".into(), "1".into())];
        let step1 = single_step(vec![group(vec![setter])]);
        let inst = installer(vec![step0, step1], vec![]);

        let mut auto_atom = atom("auto_out", "ao", 0, 0);
        auto_atom.always_install = true;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![auto_atom], vec![]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![false]], vec![vec![true]]];
        assert!(simulate(&inst, &atoms, &sel, None, None).files.is_empty());
    }

    // ------------------------------------------------------------------
    // Selected plugin applies auto atoms unconditionally in phase 2.
    // ------------------------------------------------------------------

    #[test]
    fn selected_plugin_applies_install_if_usable_even_when_effective_type_is_not_usable() {
        // A selected plugin whose eff_type would be NotUsable still installs its
        // install_if_usable AND always_install atoms in phase 2 (no filter for
        // selected plugins).
        let mut p = plugin("P", PluginType::Optional);
        p.type_patterns = vec![FomodTypePattern {
            condition: always_true(),
            result_type: PluginType::NotUsable,
        }];
        let inst = installer(vec![single_step(vec![group(vec![p])])], vec![]);

        let mut iiu = atom("iiu_out", "iu", 0, 0);
        iiu.install_if_usable = true;
        let mut always = atom("always_out", "aw", 0, 1);
        always.always_install = true;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![iiu, always]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["always_out".to_string(), "iiu_out".to_string()]
        );
    }

    // ------------------------------------------------------------------
    // Unselected plugin auto-atom gating in phase 3.
    // ------------------------------------------------------------------

    #[test]
    fn unselected_not_usable_plugin_installs_always_but_not_install_if_usable() {
        let mut p = plugin("P", PluginType::Optional);
        p.type_patterns = vec![FomodTypePattern {
            condition: always_true(),
            result_type: PluginType::NotUsable,
        }];
        let inst = installer(vec![single_step(vec![group(vec![p])])], vec![]);

        let mut iiu = atom("iiu_out", "iu", 0, 0);
        iiu.install_if_usable = true;
        let mut always = atom("always_out", "aw", 0, 1);
        always.always_install = true;
        let mut normal = atom("normal_out", "nm", 0, 2); // neither flag -> never applies unselected
        normal.always_install = false;
        normal.install_if_usable = false;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![iiu, always, normal]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![false]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["always_out".to_string()]
        );
    }

    #[test]
    fn unselected_usable_plugin_installs_both_auto_atoms() {
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let mut iiu = atom("iiu_out", "iu", 0, 0);
        iiu.install_if_usable = true;
        let mut always = atom("always_out", "aw", 0, 1);
        always.always_install = true;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![iiu, always]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![false]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["always_out".to_string(), "iiu_out".to_string()]
        );
    }

    // ------------------------------------------------------------------
    // Conditional patterns: flags + override matrix + short-vector fallback.
    // ------------------------------------------------------------------

    #[test]
    fn conditional_pattern_flag_driven_activation() {
        let mut setter = plugin("Setter", PluginType::Optional);
        setter.condition_flags = vec![("c".into(), "1".into())];
        let inst = installer(
            vec![single_step(vec![group(vec![setter])])],
            vec![FomodConditionalPattern {
                condition: flag_leaf("c", "1"),
                ..FomodConditionalPattern::default()
            }],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![]],
            per_conditional: vec![vec![atom("cond_out", "co", 0, 3)]],
            ..ExpandedAtoms::default()
        };
        // selected -> flag set -> pattern fires.
        assert_eq!(
            dests(&simulate(&inst, &atoms, &[vec![vec![true]]], None, None)),
            vec!["cond_out".to_string()]
        );
        // deselected -> flag unset -> pattern does not fire.
        assert!(
            simulate(&inst, &atoms, &[vec![vec![false]]], None, None)
                .files
                .is_empty()
        );
    }

    #[test]
    fn conditional_pattern_external_override_matrix() {
        // A single File-dependency conditional; only ForceTrue activates it.
        let inst = installer(
            vec![],
            vec![FomodConditionalPattern {
                condition: file_leaf("some/dep.esp"),
                ..FomodConditionalPattern::default()
            }],
        );
        let atoms = ExpandedAtoms {
            per_conditional: vec![vec![atom("ext_out", "eo", 0, 0)]],
            ..ExpandedAtoms::default()
        };
        let sel: Vec<Vec<Vec<bool>>> = vec![];

        use crate::fomod_dependency_evaluator::ExternalConditionOverride as X;
        let cases = [
            (X::ForceTrue, true),
            (X::ForceFalse, false),
            (X::Unknown, false),
        ];
        for (mode, expected_present) in cases {
            let ov = InferenceOverrides {
                conditional_active: vec![mode],
                step_visible: vec![],
            };
            let present = !simulate(&inst, &atoms, &sel, None, Some(&ov))
                .files
                .is_empty();
            assert_eq!(present, expected_present, "override {mode:?}");
        }
        // No override at all -> normal eval, File dep with no context -> false.
        assert!(simulate(&inst, &atoms, &sel, None, None).files.is_empty());
    }

    #[test]
    fn conditional_override_vector_shorter_than_pattern_count_falls_back_to_normal() {
        // Two File-dep conditionals; overrides supplies only index 0 (ForceTrue).
        // Pattern 1 is out of override range -> normal eval -> false (no ctx).
        let inst = installer(
            vec![],
            vec![
                FomodConditionalPattern {
                    condition: file_leaf("a.esp"),
                    ..FomodConditionalPattern::default()
                },
                FomodConditionalPattern {
                    condition: file_leaf("b.esp"),
                    ..FomodConditionalPattern::default()
                },
            ],
        );
        let atoms = ExpandedAtoms {
            per_conditional: vec![
                vec![atom("a_out", "a", 0, 0)],
                vec![atom("b_out", "b", 0, 1)],
            ],
            ..ExpandedAtoms::default()
        };
        use crate::fomod_dependency_evaluator::ExternalConditionOverride as X;
        let ov = InferenceOverrides {
            conditional_active: vec![X::ForceTrue],
            step_visible: vec![],
        };
        let t = simulate(&inst, &atoms, &[], None, Some(&ov));
        assert_eq!(dests(&t), vec!["a_out".to_string()]);
    }

    #[test]
    fn step_visibility_override_forces_flag_independent_visibility() {
        // A File-dep visibility condition. ForceFalse hides the step (its
        // selected plugin does not install); ForceTrue shows it.
        let step0 = FomodStep {
            visible: Some(file_leaf("gate.esp")),
            groups: vec![group(vec![plugin("P", PluginType::Optional)])],
            ..FomodStep::default()
        };
        let inst = installer(vec![step0], vec![]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![atom("out", "o", 0, 0)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        use crate::fomod_dependency_evaluator::ExternalConditionOverride as X;

        let ov_true = InferenceOverrides {
            conditional_active: vec![],
            step_visible: vec![X::ForceTrue],
        };
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, Some(&ov_true))),
            vec!["out".to_string()]
        );

        let ov_false = InferenceOverrides {
            conditional_active: vec![],
            step_visible: vec![X::ForceFalse],
        };
        assert!(
            simulate(&inst, &atoms, &sel, None, Some(&ov_false))
                .files
                .is_empty()
        );
    }

    // ------------------------------------------------------------------
    // Short / empty selection grids treated as false everywhere.
    // ------------------------------------------------------------------

    #[test]
    fn empty_and_short_selection_grids_deselect_everything() {
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![atom("out", "o", 0, 0)]],
            ..ExpandedAtoms::default()
        };
        // Fully empty grid.
        assert!(simulate(&inst, &atoms, &[], None, None).files.is_empty());
        // Outer present but group axis short.
        assert!(
            simulate(&inst, &atoms, &[vec![]], None, None)
                .files
                .is_empty()
        );
        // Group present but plugin axis short.
        assert!(
            simulate(&inst, &atoms, &[vec![vec![]]], None, None)
                .files
                .is_empty()
        );
    }

    // ------------------------------------------------------------------
    // simulate_into clears prior contents.
    // ------------------------------------------------------------------

    #[test]
    fn simulate_into_clears_prior_contents_on_reuse() {
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![atom("x", "x", 0, 0)]],
            ..ExpandedAtoms::default()
        };
        let mut tree = SimulatedTree::default();
        // First run selects the plugin -> "x" present.
        simulate_into(&mut tree, &inst, &atoms, &[vec![vec![true]]], None, None);
        assert_eq!(dests(&tree), vec!["x".to_string()]);
        // Second run selects nothing -> tree must be cleared, "x" gone.
        simulate_into(&mut tree, &inst, &atoms, &[], None, None);
        assert!(tree.files.is_empty());
    }

    // ------------------------------------------------------------------
    // compare_trees else-chain + excluded skipping.
    // ------------------------------------------------------------------

    fn sim_with(entries: &[(&str, u64, u64)]) -> SimulatedTree {
        // (dest, file_size, content_hash)
        let mut t = SimulatedTree::default();
        for (dest, size, hash) in entries {
            let mut a = atom(dest, dest, 0, 0);
            a.file_size = *size;
            a.content_hash = *hash;
            a.origin = Origin::Plugin;
            t.files.insert(dest.to_string(), a);
        }
        t
    }

    fn target_with(entries: &[(&str, u64, u64)]) -> TargetTree {
        // (dest, size, hash)
        entries
            .iter()
            .map(|(d, s, h)| (d.to_string(), TargetFile { size: *s, hash: *h }))
            .collect()
    }

    fn excluded(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn compare_trees_size_mismatch_suppresses_hash_check() {
        // Both sizes nonzero and differ -> size_mismatch, hash never consulted
        // even though hashes also differ.
        let sim = sim_with(&[("d", 10, 99)]);
        let target = target_with(&[("d", 20, 88)]);
        let m = compare_trees(&sim, &target, &excluded(&[]));
        assert_eq!(
            m,
            ReproMetrics {
                size_mismatch: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_equal_size_differing_hash_is_hash_mismatch() {
        let sim = sim_with(&[("d", 10, 99)]);
        let target = target_with(&[("d", 10, 88)]);
        let m = compare_trees(&sim, &target, &excluded(&[]));
        assert_eq!(
            m,
            ReproMetrics {
                hash_mismatch: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_zero_target_size_still_runs_hash_check() {
        // target size 0 skips the size branch; nonzero differing hashes on both
        // sides still reach the hash branch.
        let sim = sim_with(&[("d", 10, 99)]);
        let target = target_with(&[("d", 0, 88)]);
        let m = compare_trees(&sim, &target, &excluded(&[]));
        assert_eq!(
            m,
            ReproMetrics {
                hash_mismatch: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_zero_hash_either_side_falls_through_to_reproduced() {
        // sim hash 0 -> hash branch skipped -> reproduced.
        let m = compare_trees(
            &sim_with(&[("d", 10, 0)]),
            &target_with(&[("d", 10, 88)]),
            &excluded(&[]),
        );
        assert_eq!(
            m,
            ReproMetrics {
                reproduced: 1,
                ..Default::default()
            }
        );
        // target hash 0 -> reproduced.
        let m = compare_trees(
            &sim_with(&[("d", 10, 99)]),
            &target_with(&[("d", 10, 0)]),
            &excluded(&[]),
        );
        assert_eq!(
            m,
            ReproMetrics {
                reproduced: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_zero_size_either_side_falls_through_when_hashes_agree() {
        // sim size 0 -> size branch skipped; hashes equal -> reproduced.
        let m = compare_trees(
            &sim_with(&[("d", 0, 77)]),
            &target_with(&[("d", 20, 77)]),
            &excluded(&[]),
        );
        assert_eq!(
            m,
            ReproMetrics {
                reproduced: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_counts_missing_and_extra() {
        let sim = sim_with(&[("only_sim", 5, 0)]);
        let target = target_with(&[("only_target", 5, 0)]);
        let m = compare_trees(&sim, &target, &excluded(&[]));
        assert_eq!(
            m,
            ReproMetrics {
                missing: 1,
                extra: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_excluded_skipped_in_both_loops() {
        // "gone" would be missing (target-only), "surplus" would be extra
        // (sim-only); both excluded -> neither counted. "keep" reproduced.
        let sim = sim_with(&[("keep", 5, 0), ("surplus", 9, 0)]);
        let target = target_with(&[("keep", 5, 0), ("gone", 3, 0)]);
        let m = compare_trees(&sim, &target, &excluded(&["gone", "surplus"]));
        assert_eq!(
            m,
            ReproMetrics {
                reproduced: 1,
                ..Default::default()
            }
        );
    }

    // ------------------------------------------------------------------
    // collect_mismatched_dests parity with compare_trees.
    // ------------------------------------------------------------------

    #[test]
    fn collect_mismatched_dests_categories_match_compare_trees() {
        // One of each: missing, size mismatch, hash mismatch, extra, plus a
        // reproduced and an excluded dest that must NOT appear.
        let sim = sim_with(&[
            ("size_bad", 10, 0),  // vs target 20 -> size mismatch
            ("hash_bad", 10, 99), // vs target hash 88 -> hash mismatch
            ("repro_ok", 10, 0),  // vs target 10 -> reproduced
            ("extra_one", 7, 0),  // not in target -> extra
            ("skip_extra", 7, 0), // extra but excluded
        ]);
        let target = target_with(&[
            ("size_bad", 20, 0),
            ("hash_bad", 10, 88),
            ("repro_ok", 10, 0),
            ("missing_one", 3, 0),  // not in sim -> missing
            ("skip_missing", 3, 0), // missing but excluded
        ]);
        let excl = excluded(&["skip_extra", "skip_missing"]);

        let mut got = collect_mismatched_dests(&sim, &target, &excl);
        got.sort();
        assert_eq!(
            got,
            vec![
                "extra_one".to_string(),
                "hash_bad".to_string(),
                "missing_one".to_string(),
                "size_bad".to_string(),
            ]
        );

        // Cross-check: the collected count equals the sum of the four error
        // counters compare_trees reports on the same tree.
        let m = compare_trees(&sim, &target, &excl);
        let error_sum = (m.missing + m.extra + m.size_mismatch + m.hash_mismatch) as usize;
        assert_eq!(got.len(), error_sum);
        assert_eq!(m.reproduced, 1);
    }

    #[test]
    fn compare_trees_impl_predicate_gate_can_suppress_counting() {
        // Prove the generic predicate variant is honored: gating on_missing to
        // false suppresses the missing count that compare_trees would report.
        let sim = sim_with(&[]);
        let target = target_with(&[("a", 1, 0), ("b", 1, 0)]);
        // Count only dest "a" as missing.
        let m = compare_trees_impl(
            &sim,
            &target,
            &excluded(&[]),
            |d| d == "a",
            |_| true,
            |_| true,
        );
        assert_eq!(m.missing, 1);
        // The unfiltered wrapper counts both.
        assert_eq!(compare_trees(&sim, &target, &excluded(&[])).missing, 2);
    }
}
