//! FOMOD constraint propagator - Rust port of `src/FomodPropagator.hpp` /
//! `.cpp`.
//!
//! A deterministic constraint-propagation pre-pass for FOMOD inference. It
//! walks the installer steps in document order and narrows each group's plugin
//! domain using three rules applied in sequence per group, iterated to a
//! fixpoint. When every group is resolved ([`PropagationResult::fully_resolved`])
//! the CSP solver can be skipped entirely.
//!
//! The three rules (the composed narrowing operator, applied to each group's
//! domain in this order so rule 2 sees rule 1's eliminations and rule 3 sees
//! both):
//!
//! 1. **Plugin type** - a plugin whose effective type is `NotUsable` is
//!    eliminated (except a dynamic `dependencyType` outcome during
//!    context-free inference, which is not definitive enough to prune); a
//!    `Required` plugin records a reason but is NOT pinned (see the doc-vs-code
//!    note below).
//! 2. **File evidence** - a plugin whose group-unique non-auto files are
//!    entirely absent from the target tree is eliminated; a plugin that
//!    uniquely produces a target file records positive evidence.
//! 3. **Cardinality** - a group whose remaining plugin set admits exactly one
//!    valid combination under its [`FomodGroupType`] is resolved.
//!
//! All steps are treated as visible: original step visibility at install time
//! cannot be determined during inference, so the propagator does not read
//! `FomodStep::visible`.
//!
//! ## Doc-vs-code gap on `Required` "pinning"
//!
//! The C++ `FomodPropagator.hpp` Doxygen says "`Required` ... plugins are
//! pinned", but the C++ CODE only records a `FORCED_REQUIRED` reason for a
//! Required plugin - it never pins it and never eliminates the other plugins.
//! This port faithfully reproduces the CODE (record-only). See PARITY-NOTES.md
//! "Task 7".
//!
//! ## Unused parameters
//!
//! `propagate` accepts `atom_index` and `overrides` to match the call signature
//! the CSP solver and inference orchestrator (Tasks 8/12) use, but the C++ body
//! never reads either (verified: they appear only in the parameter list). They
//! are kept un-prefixed so the signature matches; `context` IS used. See
//! PARITY-NOTES.md "Task 7" for why.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::fomod_atom::{AtomIndex, ExpandedAtoms, TargetTree};
use crate::fomod_csp_types::InferenceOverrides;
use crate::fomod_dependency_evaluator::evaluate_plugin_type;
use crate::fomod_ir::{FomodGroupType, FomodInstaller, compute_flat_starts};
use crate::inference_diagnostics::{ReasonCode, ReasonDetail};
use crate::logger::Logger;
use crate::types::{FomodDependencyContext, PluginType};

/// Output of the constraint propagation pre-pass. Mirror of
/// `mo2core::PropagationResult`.
///
/// Holds narrowed plugin domains and a list of groups fully resolved without
/// backtracking. When [`fully_resolved`](Self::fully_resolved) is true the CSP
/// solver can be skipped. The diagnostic fields are sized to match the
/// installer hierarchy.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PropagationResult {
    /// Per-group remaining candidate selections after propagation.
    /// `[step][group][plugin] = usable`.
    pub narrowed_domains: Vec<Vec<Vec<bool>>>,
    /// Groups fully resolved by propagation, as `(step_idx, group_idx)`.
    pub resolved_groups: Vec<(i32, i32)>,
    /// Whether all groups were resolved (no CSP needed).
    pub fully_resolved: bool,
    /// Per-plugin reason code, indexed `[step][group][plugin]`. Default value
    /// is [`ReasonCode::ImplicitDefault`]; each rule firing overwrites with a
    /// more specific code (first-wins). The C++ stores this as `int` only to
    /// dodge an include cycle; the Rust port stores the enum directly.
    pub plugin_reasons: Vec<Vec<Vec<ReasonCode>>>,
    /// Optional structured payload accompanying each plugin reason, indexed
    /// identically to `plugin_reasons`. `None` when the reason is
    /// self-explanatory. The C++ uses a nullable `nlohmann::json`; the port
    /// uses a typed [`ReasonDetail`] (Task 10 maps it to schema-v2 JSON).
    pub plugin_reason_details: Vec<Vec<Vec<Option<ReasonDetail>>>>,
    /// Per-group identifier for the rule that completed the group's
    /// resolution, from `{"propagation.unique_evidence",
    /// "propagation.cardinality", "propagation.select_all"}`. Empty when the
    /// group was not fully resolved by propagation. Indexed `[step][group]`.
    pub resolved_by: Vec<Vec<String>>,
}

/// Set the plugin reason only if it has not already been set to a more specific
/// code (first-wins: treats [`ReasonCode::ImplicitDefault`] as "unset"). When a
/// detail is supplied it is stored at the same index. Mirror of the C++
/// file-scoped `record_plugin_reason`.
///
/// The C++ helper also guards `s`/`g`/`p` against negative values; here the
/// indices are always non-negative (unsigned loop counters), so only the
/// upper-bound guard is meaningful. Both guards are kept as defensive coding
/// exactly as the C++ has them.
fn record_plugin_reason(
    result: &mut PropagationResult,
    s: usize,
    g: usize,
    p: usize,
    code: ReasonCode,
    detail: Option<ReasonDetail>,
) {
    if s >= result.plugin_reasons.len() {
        return;
    }
    if g >= result.plugin_reasons[s].len() {
        return;
    }
    if p >= result.plugin_reasons[s][g].len() {
        return;
    }
    if result.plugin_reasons[s][g][p] != ReasonCode::ImplicitDefault {
        return; // first reason wins
    }
    result.plugin_reasons[s][g][p] = code;
    if s < result.plugin_reason_details.len()
        && g < result.plugin_reason_details[s].len()
        && p < result.plugin_reason_details[s][g].len()
    {
        result.plugin_reason_details[s][g][p] = detail;
    }
}

/// Deterministic constraint propagation pre-pass for FOMOD inference. Mirror of
/// `mo2core::propagate`.
///
/// Returns a [`PropagationResult`] with narrowed plugin domains, the list of
/// fully resolved groups, and a `fully_resolved` flag indicating whether all
/// groups were determined without the CSP solver.
///
/// `atom_index` and `overrides` are accepted for call-site parity but never
/// read (the C++ body ignores them too). `context` is the optional external
/// dependency context for plugin-type evaluation; pass `None` during standalone
/// inference.
pub fn propagate(
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    atom_index: &AtomIndex,
    target: &TargetTree,
    excluded_dests: &HashSet<String>,
    overrides: &InferenceOverrides,
    context: Option<&FomodDependencyContext>,
) -> PropagationResult {
    // `atom_index` and `overrides` are unused by design (see the module doc and
    // PARITY-NOTES.md "Task 7"); the bindings below silence nothing (Rust does
    // not warn on unused function parameters) and document the intent.
    let _ = (atom_index, overrides);

    let mut result = PropagationResult::default();

    // Initialize narrowed_domains: [step][group][plugin] = true, all reasons
    // ImplicitDefault, all details None, all resolved_by empty.
    for step in &installer.steps {
        let ng = step.groups.len();
        let mut nd_step: Vec<Vec<bool>> = Vec::with_capacity(ng);
        let mut pr_step: Vec<Vec<ReasonCode>> = Vec::with_capacity(ng);
        let mut prd_step: Vec<Vec<Option<ReasonDetail>>> = Vec::with_capacity(ng);
        let mut rb_step: Vec<String> = Vec::with_capacity(ng);
        for group in &step.groups {
            let pcount = group.plugins.len();
            nd_step.push(vec![true; pcount]);
            pr_step.push(vec![ReasonCode::ImplicitDefault; pcount]);
            prd_step.push(vec![None; pcount]);
            rb_step.push(String::new());
        }
        result.narrowed_domains.push(nd_step);
        result.plugin_reasons.push(pr_step);
        result.plugin_reason_details.push(prd_step);
        result.resolved_by.push(rb_step);
    }

    // Track which groups are resolved (only mark once).
    let mut resolved: Vec<Vec<bool>> = installer
        .steps
        .iter()
        .map(|s| vec![false; s.groups.len()])
        .collect();

    // Build flat_start index: (step_idx, group_idx) -> flat plugin start index.
    let flat_starts = compute_flat_starts(installer);

    let mut flags: HashMap<String, String> = HashMap::new();

    // Fixpoint iteration: repeat until no domain changes occur.
    //
    // FOMOD flag dependencies can form cycles (group A sets a flag group B
    // reads, and vice versa). A single forward pass misses backward
    // propagation; topological ordering requires acyclicity. Fixpoint iteration
    // handles arbitrary dependency shapes. The cap (16) is a wide safety margin
    // over the empirically-observed 2-3 iterations, guarding against
    // pathological circular dependencies without risking an unbounded loop.
    const MAX_ITERATIONS: i32 = 16;
    let mut total_resolved: i32 = 0;

    for _iteration in 0..MAX_ITERATIONS {
        let mut changed = false;

        for si in 0..installer.steps.len() {
            let step = &installer.steps[si];

            // All steps treated as visible - we cannot determine original
            // visibility, so `step.visible` is intentionally not read.

            for gi in 0..step.groups.len() {
                if resolved[si][gi] {
                    continue;
                }

                let group = &step.groups[gi];
                let n = group.plugins.len();
                let flat_start = flat_starts[si][gi];

                // Work on a moved-out copy of the domain so the reason-recording
                // helper can borrow `result` mutably (the reason arrays are
                // disjoint from `narrowed_domains`); write it back afterwards.
                let mut domain = std::mem::take(&mut result.narrowed_domains[si][gi]);

                // 1. Evaluate plugin types, eliminate NotUsable.
                for (pi, dom) in domain.iter_mut().enumerate() {
                    if !*dom {
                        continue;
                    }
                    let plugin = &group.plugins[pi];
                    let eff = evaluate_plugin_type(plugin, &flags, context);

                    // During inference (no external context), dynamic
                    // dependencyType outcomes are not definitive enough for hard
                    // domain pruning.
                    let dynamic_without_context =
                        context.is_none() && !plugin.type_patterns.is_empty();
                    if eff == PluginType::NotUsable {
                        if !dynamic_without_context {
                            *dom = false;
                            changed = true;
                            record_plugin_reason(
                                &mut result,
                                si,
                                gi,
                                pi,
                                ReasonCode::ForcedNotUsable,
                                None,
                            );
                        }
                    } else if eff == PluginType::Required {
                        record_plugin_reason(
                            &mut result,
                            si,
                            gi,
                            pi,
                            ReasonCode::ForcedRequired,
                            None,
                        );
                    }
                }

                // 2. Filter by file evidence.
                // For each usable plugin, collect its non-auto-install atoms
                // that are unique within this group (no other usable plugin in
                // this group produces the same dest). If ALL such unique atoms
                // miss the target, eliminate the plugin.
                {
                    // Collect per-plugin dest sets (non-auto atoms only). A
                    // BTreeSet gives deterministic iteration (the C++ iterates
                    // an unordered_set, which only affects the diagnostic
                    // "files" example ordering, never selection - see
                    // PARITY-NOTES.md "Task 7").
                    let mut plugin_dests: Vec<BTreeSet<String>> = vec![BTreeSet::new(); n];
                    for (pi, dests) in plugin_dests.iter_mut().enumerate() {
                        if !domain[pi] {
                            continue;
                        }
                        let fi = flat_start + pi as i32;
                        if fi >= 0 && (fi as usize) < atoms.per_plugin.len() {
                            for atom in &atoms.per_plugin[fi as usize] {
                                if !atom.always_install
                                    && !atom.install_if_usable
                                    && !excluded_dests.contains(&atom.dest_path)
                                {
                                    dests.insert(atom.dest_path.clone());
                                }
                            }
                        }
                    }

                    // Build map: dest -> usable plugin indices within this group.
                    let mut dest_to_plugins_local: HashMap<&str, Vec<usize>> = HashMap::new();
                    for pi in 0..n {
                        if !domain[pi] {
                            continue;
                        }
                        for d in &plugin_dests[pi] {
                            dest_to_plugins_local
                                .entry(d.as_str())
                                .or_default()
                                .push(pi);
                        }
                    }

                    for pi in 0..n {
                        if !domain[pi] || plugin_dests[pi].is_empty() {
                            continue;
                        }

                        // Find dests unique to this plugin within the group.
                        // has_any_unique / all_unique_miss are order-independent
                        // boolean reductions, so the elimination decision is
                        // deterministic regardless of iteration order.
                        let mut has_any_unique = false;
                        let mut all_unique_miss = true;
                        let mut unique_target_hits: Vec<String> = Vec::new();
                        for d in &plugin_dests[pi] {
                            if let Some(v) = dest_to_plugins_local.get(d.as_str()) {
                                if v.len() == 1 {
                                    has_any_unique = true;
                                    if target.contains_key(d) {
                                        all_unique_miss = false;
                                        unique_target_hits.push(d.clone());
                                    }
                                }
                            }
                        }

                        if has_any_unique && all_unique_miss {
                            domain[pi] = false;
                            changed = true;
                            record_plugin_reason(
                                &mut result,
                                si,
                                gi,
                                pi,
                                ReasonCode::NoFileEvidence,
                                None,
                            );
                        } else if !unique_target_hits.is_empty() {
                            // Positive evidence: the plugin uniquely produces at
                            // least one target file. Record up to 4 examples
                            // (sorted byte-ascending - a diagnostic-only pin) and
                            // the full hit count.
                            unique_target_hits.sort();
                            let count = unique_target_hits.len() as i32;
                            let files: Vec<String> =
                                unique_target_hits.into_iter().take(4).collect();
                            record_plugin_reason(
                                &mut result,
                                si,
                                gi,
                                pi,
                                ReasonCode::UniqueFileEvidence,
                                Some(ReasonDetail::UniqueFileEvidence { files, count }),
                            );
                        }
                    }
                }

                // 3. Enforce cardinality constraints.
                let usable_count = domain.iter().filter(|&&b| b).count();

                let group_resolved = match group.r#type {
                    // All usable plugins are selected.
                    FomodGroupType::SelectAll => true,
                    FomodGroupType::SelectExactlyOne => usable_count == 1,
                    // Only resolve when forced to zero. When usable_count == 1,
                    // "select zero" is still valid, so leave it for the CSP.
                    FomodGroupType::SelectAtMostOne => usable_count == 0,
                    FomodGroupType::SelectAtLeastOne => usable_count == 1,
                    FomodGroupType::SelectAny => usable_count == 0,
                };

                if group_resolved {
                    resolved[si][gi] = true;
                    result.resolved_groups.push((si as i32, gi as i32));
                    total_resolved += 1;
                    changed = true;

                    // Attribute the resolution to a stable string. Prefer the
                    // most informative cause: SelectAll (spec) > unique evidence
                    // > generic cardinality.
                    let resolved_by_str = if group.r#type == FomodGroupType::SelectAll {
                        "propagation.select_all".to_string()
                    } else {
                        let mut saw_evidence = false;
                        for pi in 0..n {
                            let code = result.plugin_reasons[si][gi][pi];
                            if code == ReasonCode::UniqueFileEvidence
                                || code == ReasonCode::NoFileEvidence
                            {
                                saw_evidence = true;
                                break;
                            }
                        }
                        if saw_evidence {
                            "propagation.unique_evidence".to_string()
                        } else {
                            "propagation.cardinality".to_string()
                        }
                    };
                    result.resolved_by[si][gi] = resolved_by_str;

                    // Emit per-plugin reasons explaining the kept selections.
                    for (pi, &dom) in domain.iter().enumerate() {
                        if !dom {
                            continue;
                        }
                        let kept_reason = match group.r#type {
                            FomodGroupType::SelectAll => Some(ReasonCode::ForcedSelectAll),
                            FomodGroupType::SelectExactlyOne if usable_count == 1 => {
                                Some(ReasonCode::ForcedExactlyOne)
                            }
                            FomodGroupType::SelectAtLeastOne if usable_count == 1 => {
                                Some(ReasonCode::ForcedAtLeastOne)
                            }
                            // SelectAtMostOne / SelectAny (usable_count == 0
                            // path): deselected plugins already carry
                            // FORCED_NOT_USABLE / NO_FILE_EVIDENCE.
                            _ => None,
                        };
                        if let Some(kr) = kept_reason {
                            record_plugin_reason(&mut result, si, gi, pi, kr, None);
                        }
                    }

                    // Accumulate flags from selected plugins in the resolved
                    // group. domain[pi] == true here means the plugin is
                    // definitively selected (the C++ debug assert that
                    // group_resolved holds is trivially true inside this block).
                    for (pi, &dom) in domain.iter().enumerate() {
                        if dom {
                            for (name, value) in &group.plugins[pi].condition_flags {
                                flags.insert(name.clone(), value.clone());
                            }
                        }
                    }
                }

                // Persist the narrowed domain for the next iteration and callers.
                result.narrowed_domains[si][gi] = domain;
            }
        }

        if !changed {
            break;
        }
    }

    // Check if all groups are resolved.
    let total_groups: i32 = installer.steps.iter().map(|s| s.groups.len() as i32).sum();

    result.fully_resolved = total_resolved == total_groups;

    let fully_resolved = result.fully_resolved;
    Logger::instance().log(&format!(
        "[propagate] resolved {total_resolved}/{total_groups} groups, fully_resolved={fully_resolved}"
    ));

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_atom::{FomodAtom, TargetFile};
    use crate::fomod_ir::{
        FomodCondition, FomodConditionType, FomodGroup, FomodPlugin, FomodStep, FomodTypePattern,
    };

    // --- synthetic-IR builders --------------------------------------------

    fn plugin(name: &str, ptype: PluginType) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            r#type: ptype,
            ..FomodPlugin::default()
        }
    }

    /// A plugin whose type flips via a single flag-condition type_pattern.
    fn plugin_with_flag_pattern(
        name: &str,
        base: PluginType,
        flag_name: &str,
        flag_value: &str,
        result_type: PluginType,
    ) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            r#type: base,
            type_patterns: vec![FomodTypePattern {
                condition: FomodCondition {
                    r#type: FomodConditionType::Flag,
                    flag_name: flag_name.to_string(),
                    flag_value: flag_value.to_string(),
                    ..FomodCondition::default()
                },
                result_type,
            }],
            ..FomodPlugin::default()
        }
    }

    fn group(gtype: FomodGroupType, plugins: Vec<FomodPlugin>) -> FomodGroup {
        FomodGroup {
            name: "g".to_string(),
            r#type: gtype,
            plugins,
        }
    }

    fn installer_one_step(groups: Vec<FomodGroup>) -> FomodInstaller {
        FomodInstaller {
            steps: vec![FomodStep {
                groups,
                ..FomodStep::default()
            }],
            ..FomodInstaller::default()
        }
    }

    fn atom(dest: &str, always: bool, if_usable: bool) -> FomodAtom {
        FomodAtom {
            dest_path: dest.to_string(),
            always_install: always,
            install_if_usable: if_usable,
            ..FomodAtom::default()
        }
    }

    fn target_of(dests: &[&str]) -> TargetTree {
        dests
            .iter()
            .map(|d| (d.to_string(), TargetFile::default()))
            .collect()
    }

    fn excluded_of(dests: &[&str]) -> HashSet<String> {
        dests.iter().map(|s| s.to_string()).collect()
    }

    /// Run `propagate` on a one-step installer with no atoms.
    fn run_no_atoms(
        installer: &FomodInstaller,
        context: Option<&FomodDependencyContext>,
    ) -> PropagationResult {
        propagate(
            installer,
            &ExpandedAtoms::default(),
            &AtomIndex::default(),
            &TargetTree::default(),
            &HashSet::default(),
            &InferenceOverrides::default(),
            context,
        )
    }

    /// Run `propagate` with an explicit per-plugin atom layout + target +
    /// excluded set (single step, flat plugin indices in document order).
    fn run_with_atoms(
        installer: &FomodInstaller,
        per_plugin: Vec<Vec<FomodAtom>>,
        target: &TargetTree,
        excluded: &HashSet<String>,
    ) -> PropagationResult {
        let atoms = ExpandedAtoms {
            required: Vec::new(),
            per_plugin,
            per_conditional: Vec::new(),
        };
        propagate(
            installer,
            &atoms,
            &AtomIndex::default(),
            target,
            excluded,
            &InferenceOverrides::default(),
            None,
        )
    }

    // --- Rule 1: plugin type ----------------------------------------------

    #[test]
    fn rule1_notusable_base_type_eliminated_without_patterns() {
        // p0 NotUsable (no type_patterns) is eliminated even with context=None;
        // p1 Optional survives. SelectAny never resolves at usable_count==1, so
        // the domain is inspectable.
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectAny,
            vec![
                plugin("p0", PluginType::NotUsable),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r = run_no_atoms(&inst, None);
        assert_eq!(r.narrowed_domains[0][0], vec![false, true]);
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::ForcedNotUsable);
        assert_eq!(r.plugin_reasons[0][0][1], ReasonCode::ImplicitDefault);
        // Mutation sensitivity: if the elimination were skipped, usable_count
        // would be 2 and p0 would still be true.
        assert!(!r.narrowed_domains[0][0][0]);
    }

    #[test]
    fn rule1_dynamic_notusable_guarded_without_context_eliminated_with_context() {
        // p0's flag-only type_pattern yields NotUsable (flag "f" missing, empty
        // expected value -> pattern matches in both modes). p1 keeps the group
        // from resolving so we can inspect the domain in both runs.
        let make = || {
            installer_one_step(vec![group(
                FomodGroupType::SelectAny,
                vec![
                    plugin_with_flag_pattern(
                        "p0",
                        PluginType::Optional,
                        "f",
                        "",
                        PluginType::NotUsable,
                    ),
                    plugin("p1", PluginType::Optional),
                ],
            )])
        };

        // context = None: dynamic_without_context is true -> NOT eliminated.
        let none = run_no_atoms(&make(), None);
        assert_eq!(none.narrowed_domains[0][0], vec![true, true]);
        assert_eq!(none.plugin_reasons[0][0][0], ReasonCode::ImplicitDefault);

        // context = Some(empty): dynamic_without_context is false -> eliminated.
        let ctx = FomodDependencyContext::default();
        let some = run_no_atoms(&make(), Some(&ctx));
        assert_eq!(some.narrowed_domains[0][0], vec![false, true]);
        assert_eq!(some.plugin_reasons[0][0][0], ReasonCode::ForcedNotUsable);
    }

    #[test]
    fn rule1_required_records_reason_but_does_not_pin_or_eliminate_sibling() {
        // SelectExactlyOne with a Required p0 and an Optional p1. If Required
        // were pinned (siblings eliminated), usable_count would drop to 1 and
        // the group would resolve. The CODE does neither: usable_count stays 2,
        // the group does NOT resolve, and only a reason is recorded.
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectExactlyOne,
            vec![
                plugin("p0", PluginType::Required),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r = run_no_atoms(&inst, None);
        assert_eq!(r.narrowed_domains[0][0], vec![true, true]);
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::ForcedRequired);
        assert_eq!(r.plugin_reasons[0][0][1], ReasonCode::ImplicitDefault);
        assert!(r.resolved_groups.is_empty());
        assert!(!r.fully_resolved);
        assert!(r.resolved_by[0][0].is_empty());
    }

    // --- Rule 2: file evidence --------------------------------------------

    #[test]
    fn rule2_unique_miss_eliminates_and_unique_hit_records_evidence() {
        // p0's unique dest is absent from target -> eliminated (NO_FILE_EVIDENCE).
        // p1's unique dest is present -> UNIQUE_FILE_EVIDENCE with that file.
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectAny,
            vec![
                plugin("p0", PluginType::Optional),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let target = target_of(&["b/y.dds"]);
        let r = run_with_atoms(
            &inst,
            vec![
                vec![atom("a/x.dds", false, false)],
                vec![atom("b/y.dds", false, false)],
            ],
            &target,
            &HashSet::default(),
        );
        assert_eq!(r.narrowed_domains[0][0], vec![false, true]);
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::NoFileEvidence);
        assert_eq!(r.plugin_reasons[0][0][1], ReasonCode::UniqueFileEvidence);
        assert_eq!(
            r.plugin_reason_details[0][0][1],
            Some(ReasonDetail::UniqueFileEvidence {
                files: vec!["b/y.dds".to_string()],
                count: 1,
            })
        );
        assert_eq!(r.plugin_reason_details[0][0][0], None);
    }

    #[test]
    fn rule2_shared_dest_is_not_unique_so_neither_eliminated() {
        // Both plugins produce the same dest (absent from target). A shared dest
        // is not unique, so has_any_unique is false and neither is eliminated.
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectAny,
            vec![
                plugin("p0", PluginType::Optional),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r = run_with_atoms(
            &inst,
            vec![
                vec![atom("shared.dds", false, false)],
                vec![atom("shared.dds", false, false)],
            ],
            &TargetTree::default(),
            &HashSet::default(),
        );
        assert_eq!(r.narrowed_domains[0][0], vec![true, true]);
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::ImplicitDefault);
        assert_eq!(r.plugin_reasons[0][0][1], ReasonCode::ImplicitDefault);
    }

    #[test]
    fn rule2_auto_atoms_are_excluded_from_evidence() {
        // p0 has only always_install atoms, p1 only install_if_usable atoms,
        // both missing target. Auto atoms never enter plugin_dests, so neither
        // plugin is eliminated.
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectAny,
            vec![
                plugin("p0", PluginType::Optional),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r = run_with_atoms(
            &inst,
            vec![
                vec![atom("auto1.dds", true, false)],
                vec![atom("auto2.dds", false, true)],
            ],
            &TargetTree::default(),
            &HashSet::default(),
        );
        assert_eq!(r.narrowed_domains[0][0], vec![true, true]);
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::ImplicitDefault);
        assert_eq!(r.plugin_reasons[0][0][1], ReasonCode::ImplicitDefault);
    }

    #[test]
    fn rule2_excluded_dests_are_ignored() {
        // p0's only unique dest is excluded -> ignored -> not eliminated.
        // p1's unique dest is not excluded and missing -> eliminated (control).
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectAny,
            vec![
                plugin("p0", PluginType::Optional),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r = run_with_atoms(
            &inst,
            vec![
                vec![atom("excl.dds", false, false)],
                vec![atom("keep.dds", false, false)],
            ],
            &TargetTree::default(),
            &excluded_of(&["excl.dds"]),
        );
        assert_eq!(r.narrowed_domains[0][0], vec![true, false]);
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::ImplicitDefault);
        assert_eq!(r.plugin_reasons[0][0][1], ReasonCode::NoFileEvidence);
    }

    #[test]
    fn rule2_unique_hit_detail_is_sorted_first_four_with_full_count() {
        // A single plugin uniquely produces five target files. The detail lists
        // the first four SORTED byte-ascending, but count is the full five.
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectAny,
            vec![plugin("p0", PluginType::Optional)],
        )]);
        let dests = ["z.dds", "a.dds", "m.dds", "c.dds", "q.dds"];
        let target = target_of(&dests);
        let r = run_with_atoms(
            &inst,
            vec![dests.iter().map(|d| atom(d, false, false)).collect()],
            &target,
            &HashSet::default(),
        );
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::UniqueFileEvidence);
        assert_eq!(
            r.plugin_reason_details[0][0][0],
            Some(ReasonDetail::UniqueFileEvidence {
                files: vec![
                    "a.dds".to_string(),
                    "c.dds".to_string(),
                    "m.dds".to_string(),
                    "q.dds".to_string(),
                ],
                count: 5,
            })
        );
    }

    // --- Rule 3: cardinality per group type -------------------------------

    #[test]
    fn rule3_select_all_always_resolves_with_forced_select_all() {
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectAll,
            vec![
                plugin("p0", PluginType::Optional),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r = run_no_atoms(&inst, None);
        assert!(r.fully_resolved);
        assert_eq!(r.resolved_groups, vec![(0, 0)]);
        assert_eq!(r.resolved_by[0][0], "propagation.select_all");
        assert_eq!(r.narrowed_domains[0][0], vec![true, true]);
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::ForcedSelectAll);
        assert_eq!(r.plugin_reasons[0][0][1], ReasonCode::ForcedSelectAll);
    }

    #[test]
    fn rule3_select_exactly_one_resolves_at_one_not_two() {
        // Resolve: p0 NotUsable eliminated -> usable_count 1 -> resolves,
        // FORCED_EXACTLY_ONE on the survivor, cardinality attribution.
        let resolve = installer_one_step(vec![group(
            FomodGroupType::SelectExactlyOne,
            vec![
                plugin("p0", PluginType::NotUsable),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r = run_no_atoms(&resolve, None);
        assert!(r.fully_resolved);
        assert_eq!(r.narrowed_domains[0][0], vec![false, true]);
        assert_eq!(r.plugin_reasons[0][0][1], ReasonCode::ForcedExactlyOne);
        assert_eq!(r.resolved_by[0][0], "propagation.cardinality");

        // No resolve: two Optional plugins -> usable_count 2 -> not resolved.
        let stay = installer_one_step(vec![group(
            FomodGroupType::SelectExactlyOne,
            vec![
                plugin("p0", PluginType::Optional),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r2 = run_no_atoms(&stay, None);
        assert!(!r2.fully_resolved);
        assert!(r2.resolved_groups.is_empty());
    }

    #[test]
    fn rule3_select_at_least_one_resolves_at_one_not_two() {
        let resolve = installer_one_step(vec![group(
            FomodGroupType::SelectAtLeastOne,
            vec![
                plugin("p0", PluginType::NotUsable),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r = run_no_atoms(&resolve, None);
        assert!(r.fully_resolved);
        assert_eq!(r.plugin_reasons[0][0][1], ReasonCode::ForcedAtLeastOne);

        let stay = installer_one_step(vec![group(
            FomodGroupType::SelectAtLeastOne,
            vec![
                plugin("p0", PluginType::Optional),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        assert!(!run_no_atoms(&stay, None).fully_resolved);
    }

    #[test]
    fn rule3_select_at_most_one_resolves_only_at_zero() {
        // usable_count == 1 -> NOT resolved (select-zero remains valid).
        let one = installer_one_step(vec![group(
            FomodGroupType::SelectAtMostOne,
            vec![plugin("p0", PluginType::Optional)],
        )]);
        let r1 = run_no_atoms(&one, None);
        assert!(!r1.fully_resolved);
        assert!(r1.resolved_groups.is_empty());
        assert_eq!(r1.narrowed_domains[0][0], vec![true]);

        // usable_count == 0 -> resolved.
        let zero = installer_one_step(vec![group(
            FomodGroupType::SelectAtMostOne,
            vec![plugin("p0", PluginType::NotUsable)],
        )]);
        let r0 = run_no_atoms(&zero, None);
        assert!(r0.fully_resolved);
        assert_eq!(r0.narrowed_domains[0][0], vec![false]);
        assert_eq!(r0.resolved_by[0][0], "propagation.cardinality");
    }

    #[test]
    fn rule3_select_any_resolves_only_at_zero() {
        let some = installer_one_step(vec![group(
            FomodGroupType::SelectAny,
            vec![plugin("p0", PluginType::Optional)],
        )]);
        assert!(!run_no_atoms(&some, None).fully_resolved);

        let zero = installer_one_step(vec![group(
            FomodGroupType::SelectAny,
            vec![plugin("p0", PluginType::NotUsable)],
        )]);
        let r0 = run_no_atoms(&zero, None);
        assert!(r0.fully_resolved);
        assert_eq!(r0.narrowed_domains[0][0], vec![false]);
    }

    // --- resolved_by attribution ------------------------------------------

    #[test]
    fn resolved_by_attribution_select_all_evidence_and_cardinality() {
        // select_all
        let sa = installer_one_step(vec![group(
            FomodGroupType::SelectAll,
            vec![plugin("p0", PluginType::Optional)],
        )]);
        assert_eq!(
            run_no_atoms(&sa, None).resolved_by[0][0],
            "propagation.select_all"
        );

        // unique_evidence: SelectExactlyOne resolved after a file-evidence
        // elimination (p0 unique miss -> NO_FILE_EVIDENCE, p1 unique hit).
        let ev = installer_one_step(vec![group(
            FomodGroupType::SelectExactlyOne,
            vec![
                plugin("p0", PluginType::Optional),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let rev = run_with_atoms(
            &ev,
            vec![
                vec![atom("miss.dds", false, false)],
                vec![atom("hit.dds", false, false)],
            ],
            &target_of(&["hit.dds"]),
            &HashSet::default(),
        );
        assert!(rev.fully_resolved);
        assert_eq!(rev.resolved_by[0][0], "propagation.unique_evidence");

        // cardinality: SelectExactlyOne resolved via a NotUsable elimination
        // (no file-evidence reasons in the group).
        let card = installer_one_step(vec![group(
            FomodGroupType::SelectExactlyOne,
            vec![
                plugin("p0", PluginType::NotUsable),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        assert_eq!(
            run_no_atoms(&card, None).resolved_by[0][0],
            "propagation.cardinality"
        );
    }

    // --- fixpoint flag propagation ----------------------------------------

    #[test]
    fn fixpoint_flag_set_by_later_group_resolves_earlier_group_next_iteration() {
        // Document order: group 0 = B (reads flag F), group 1 = A (sets F).
        // In iteration 0, B is visited BEFORE A (flag unset) and cannot resolve;
        // A then resolves and sets F. B can only resolve when revisited in
        // iteration 1 - proving the fixpoint did more than a single pass.
        //
        // A context is supplied because B's resolution relies on a type_pattern
        // flipping a plugin to NotUsable, which only prunes with a context
        // (the dynamic-without-context guard blocks it during context-free runs).
        let make = |a_flag_value: &str| {
            // B: SelectExactlyOne. pB0 flips to NotUsable when F == "on".
            let b = group(
                FomodGroupType::SelectExactlyOne,
                vec![
                    plugin_with_flag_pattern(
                        "pB0",
                        PluginType::Optional,
                        "F",
                        "on",
                        PluginType::NotUsable,
                    ),
                    plugin("pB1", PluginType::Optional),
                ],
            );
            // A: SelectAll, single plugin that sets F.
            let mut pa = plugin("pA", PluginType::Optional);
            pa.condition_flags = vec![("F".to_string(), a_flag_value.to_string())];
            let a = group(FomodGroupType::SelectAll, vec![pa]);
            installer_one_step(vec![b, a])
        };

        let ctx = FomodDependencyContext::default();

        // A sets F = "on": B's pattern matches on iteration 1, pB0 eliminated,
        // B resolves. Both groups resolved -> fully_resolved.
        let inst = make("on");
        let r = run_no_atoms(&inst, Some(&ctx));
        assert!(
            r.fully_resolved,
            "flag propagation must resolve B on a later pass"
        );
        assert_eq!(r.narrowed_domains[0][0], vec![false, true]);
        assert!(r.resolved_groups.contains(&(0, 0)));
        assert!(r.resolved_groups.contains(&(0, 1)));

        // Mutation: A sets F = "off" -> B's pattern never matches -> pB0 stays
        // Optional, B keeps usable_count 2 and never resolves.
        let inst_off = make("off");
        let r_off = run_no_atoms(&inst_off, Some(&ctx));
        assert!(!r_off.fully_resolved);
        assert_eq!(r_off.narrowed_domains[0][0], vec![true, true]);
        assert!(!r_off.resolved_groups.contains(&(0, 0)));
    }

    // --- record_plugin_reason first-wins ----------------------------------

    #[test]
    fn record_plugin_reason_keeps_first_code() {
        // p0 earns UNIQUE_FILE_EVIDENCE in rule 2 (a unique target hit) and is
        // the sole survivor of a SelectExactlyOne after p1 is eliminated. Rule 3
        // then tries to stamp FORCED_EXACTLY_ONE on p0, but first-wins keeps the
        // earlier UNIQUE_FILE_EVIDENCE.
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectExactlyOne,
            vec![
                plugin("p0", PluginType::Optional),
                plugin("p1", PluginType::Optional),
            ],
        )]);
        let r = run_with_atoms(
            &inst,
            vec![
                vec![atom("hit.dds", false, false)],
                vec![atom("miss.dds", false, false)],
            ],
            &target_of(&["hit.dds"]),
            &HashSet::default(),
        );
        assert!(r.fully_resolved);
        assert_eq!(r.narrowed_domains[0][0], vec![true, false]);
        // First code wins: UNIQUE_FILE_EVIDENCE, NOT FORCED_EXACTLY_ONE.
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::UniqueFileEvidence);
        assert_eq!(
            r.plugin_reason_details[0][0][0],
            Some(ReasonDetail::UniqueFileEvidence {
                files: vec!["hit.dds".to_string()],
                count: 1,
            })
        );
    }

    // --- fully_resolved predicate -----------------------------------------

    #[test]
    fn fully_resolved_true_when_all_groups_resolve_false_otherwise() {
        // Two groups both resolve (SelectAll + SelectExactlyOne narrowed to 1).
        let all_resolve = installer_one_step(vec![
            group(
                FomodGroupType::SelectAll,
                vec![plugin("a", PluginType::Optional)],
            ),
            group(
                FomodGroupType::SelectExactlyOne,
                vec![
                    plugin("b0", PluginType::NotUsable),
                    plugin("b1", PluginType::Optional),
                ],
            ),
        ]);
        let r = run_no_atoms(&all_resolve, None);
        assert!(r.fully_resolved);
        assert_eq!(r.resolved_groups.len(), 2);

        // One group left ambiguous -> not fully resolved.
        let one_ambiguous = installer_one_step(vec![
            group(
                FomodGroupType::SelectAll,
                vec![plugin("a", PluginType::Optional)],
            ),
            group(
                FomodGroupType::SelectExactlyOne,
                vec![
                    plugin("b0", PluginType::Optional),
                    plugin("b1", PluginType::Optional),
                ],
            ),
        ]);
        let r2 = run_no_atoms(&one_ambiguous, None);
        assert!(!r2.fully_resolved);
        assert_eq!(r2.resolved_groups, vec![(0, 0)]);
    }

    // --- shapes + determinism ---------------------------------------------

    #[test]
    fn result_shapes_match_installer_hierarchy() {
        let inst = installer_one_step(vec![
            group(
                FomodGroupType::SelectAny,
                vec![
                    plugin("a", PluginType::Optional),
                    plugin("b", PluginType::Optional),
                ],
            ),
            group(
                FomodGroupType::SelectAny,
                vec![plugin("c", PluginType::Optional)],
            ),
        ]);
        let r = run_no_atoms(&inst, None);
        assert_eq!(r.narrowed_domains.len(), 1);
        assert_eq!(r.narrowed_domains[0].len(), 2);
        assert_eq!(r.narrowed_domains[0][0].len(), 2);
        assert_eq!(r.narrowed_domains[0][1].len(), 1);
        assert_eq!(r.plugin_reasons[0][0].len(), 2);
        assert_eq!(r.plugin_reason_details[0][1].len(), 1);
        assert_eq!(r.resolved_by[0].len(), 2);
    }

    #[test]
    fn propagate_is_deterministic_across_runs() {
        let inst = installer_one_step(vec![group(
            FomodGroupType::SelectAny,
            vec![plugin("p0", PluginType::Optional)],
        )]);
        let dests = ["z.dds", "a.dds", "m.dds", "c.dds", "q.dds"];
        let target = target_of(&dests);
        let per_plugin: Vec<Vec<FomodAtom>> =
            vec![dests.iter().map(|d| atom(d, false, false)).collect()];
        let a = run_with_atoms(&inst, per_plugin.clone(), &target, &HashSet::default());
        let b = run_with_atoms(&inst, per_plugin, &target, &HashSet::default());
        assert_eq!(a, b);
    }
}
