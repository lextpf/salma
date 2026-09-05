/*!
 * @brief narrows FOMOD plugin domains before CSP search.
 * @author Alex (https://github.com/lextpf)
 *
 * each fixpoint pass applies effective plugin type, unique-file evidence, and group cardinality
 * in that order. the narrowed domains seed the solver and do not replace it.
 */

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::fomod_atom::{AtomIndex, ExpandedAtoms, TargetTree};
use crate::fomod_csp_types::InferenceOverrides;
use crate::fomod_dependency_evaluator::evaluate_plugin_type;
use crate::fomod_ir::{FomodGroupType, FomodInstaller, compute_flat_starts};
use crate::inference_diagnostics::{ReasonCode, ReasonDetail};
use crate::logger::Logger;
use crate::types::{FomodDependencyContext, PluginType};

/**
 * @struct PropagationResult
 * @brief hold narrowed domains and groups resolved before backtracking.
 * @author Alex (https://github.com/lextpf)
 *
 * `narrowed_domains` filters each group's option list during the CSP solve, and `resolved_groups`
 * blanks the matching `phase_per_group` entries in the solver's result.
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PropagationResult {
    pub narrowed_domains: Vec<Vec<Vec<bool>>>,
    /**
     * @brief list groups in propagation-resolution order.
     * @author Alex (https://github.com/lextpf)
     *
     * a group is pushed at most once, because a resolved group is skipped by every later iteration.
     */
    pub resolved_groups: Vec<(i32, i32)>,
    /**
     * @brief true when propagation resolved every group in the installer.
     * @author Alex (https://github.com/lextpf)
     *
     * no caller skips or shortens the CSP solve on this flag: the inference service reads it once,
     * into a log line, and the solver never reads it at all.
     */
    pub fully_resolved: bool,
    /**
     * @brief reason code per plugin, indexed by step, group and plugin.
     * @author Alex (https://github.com/lextpf)
     */
    pub plugin_reasons: Vec<Vec<Vec<ReasonCode>>>,
    pub plugin_reason_details: Vec<Vec<Vec<Option<ReasonDetail>>>>,
    /**
     * @brief per-group identifier for the rule that completed the group's resolution.
     * @author Alex (https://github.com/lextpf)
     *
     * `propagation.cardinality` otherwise.
     */
    pub resolved_by: Vec<Vec<String>>,
}

// record a plugin reason, first-wins.
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

// malformed or contradictory input weakens narrowing instead of failing.
pub fn propagate(
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    atom_index: &AtomIndex,
    target: &TargetTree,
    excluded_dests: &HashSet<String>,
    overrides: &InferenceOverrides,
    context: Option<&FomodDependencyContext>,
) -> PropagationResult {
    // keep these parameters in the shared solver interface; this phase does not use them.
    let _ = (atom_index, overrides);

    let mut result = PropagationResult::default();

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

    // track which groups are resolved (only mark once).
    let mut resolved: Vec<Vec<bool>> = installer
        .steps
        .iter()
        .map(|s| vec![false; s.groups.len()])
        .collect();

    let flat_starts = compute_flat_starts(installer);

    let mut flags: HashMap<String, String> = HashMap::new();

    // fixpoint iteration: repeat until no domain changes occur. FOMOD flag dependencies can form
    // cycles (group A sets a flag group B reads, and the reverse), so a single forward pass misses
    // the backward direction and a topological order does not exist. iterating to a fixpoint
    // handles arbitrary dependency shapes. the cap of 16 guards malformed installers, not
    // non-termination: every `changed = true` site either clears a domain bit or marks a group
    // resolved once, so the loop is bounded by plugins eliminated plus groups resolved with or
    // without it. typical installers settle in 2 to 3 iterations. reaching the cap is not an error
    // and logs no warning: the narrowing returned is still sound, only possibly weaker.
    const MAX_ITERATIONS: i32 = 16;
    let mut total_resolved: i32 = 0;

    for _iteration in 0..MAX_ITERATIONS {
        let mut changed = false;

        for si in 0..installer.steps.len() {
            let step = &installer.steps[si];

            // every step counts as visible: the visibility the user saw at install time cannot be
            // recovered, so `step.visible` is not read.

            for gi in 0..step.groups.len() {
                // a resolved group is frozen for the rest of this pre-pass: its domain, reasons and
                // resolved_by are never revisited, even if a later group writes a flag that would
                // change its plugin types.
                if resolved[si][gi] {
                    continue;
                }

                let group = &step.groups[gi];
                let n = group.plugins.len();
                let flat_start = flat_starts[si][gi];

                let mut domain = std::mem::take(&mut result.narrowed_domains[si][gi]);

                // 1. evaluate plugin types, eliminate NotUsable.
                for (pi, dom) in domain.iter_mut().enumerate() {
                    if !*dom {
                        continue;
                    }
                    let plugin = &group.plugins[pi];
                    let eff = evaluate_plugin_type(plugin, &flags, context);

                    // during inference (no external context), dynamic dependencyType outcomes are
                    // not definitive enough for hard domain pruning.
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

                // 2. filter by file evidence. for each usable plugin, collect its non-auto-install
                // atoms that are unique within this group (no other usable plugin in this group
                // produces the same dest). eliminate the plugin when every one of those unique
                // atoms misses the target.
                {
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

                        // find dests unique to this plugin within the group. has_any_unique /
                        // all_unique_miss are order-independent boolean reductions, so the
                        // elimination decision is deterministic regardless of iteration order.
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
                            // positive evidence: the plugin uniquely produces at least one target
                            // file. record the full hit count plus up to 4 examples, sorted
                            // byte-ascending so the diagnostic is stable across runs.
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

                // 3. enforce cardinality constraints.
                let usable_count = domain.iter().filter(|&&b| b).count();

                let group_resolved = match group.r#type {
                    // every usable plugin is selected.
                    FomodGroupType::SelectAll => true,
                    FomodGroupType::SelectExactlyOne => usable_count == 1,
                    // resolve only when forced to zero. at usable_count == 1 "select zero" is still
                    // valid, so leave it for the CSP.
                    FomodGroupType::SelectAtMostOne => usable_count == 0,
                    FomodGroupType::SelectAtLeastOne => usable_count == 1,
                    FomodGroupType::SelectAny => usable_count == 0,
                };

                if group_resolved {
                    resolved[si][gi] = true;
                    result.resolved_groups.push((si as i32, gi as i32));
                    total_resolved += 1;
                    changed = true;

                    // attribute the resolution to a stable string, most informative cause first:
                    // SelectAll, then unique file evidence, then plain cardinality.
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

                    // emit per-plugin reasons explaining the kept selections.
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
                            // SelectAtMostOne and SelectAny resolve only at usable_count == 0,
                            // where every plugin already carries a ForcedNotUsable or
                            // NoFileEvidence reason.
                            _ => None,
                        };
                        if let Some(kr) = kept_reason {
                            record_plugin_reason(&mut result, si, gi, pi, kr, None);
                        }
                    }

                    // accumulate flags from the selected plugins of the resolved group. inside this
                    // block `domain[pi] == true` means the plugin is definitively selected, so its
                    // condition_flags become visible to every group visited afterwards.
                    for (pi, &dom) in domain.iter().enumerate() {
                        if dom {
                            for (name, value) in &group.plugins[pi].condition_flags {
                                flags.insert(name.clone(), value.clone());
                            }
                        }
                    }
                }

                // persist the narrowed domain for the next iteration and callers.
                result.narrowed_domains[si][gi] = domain;
            }
        }

        if !changed {
            break;
        }
    }

    // diagnostic tally: no caller changes behavior on `fully_resolved`.
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

    fn plugin(name: &str, ptype: PluginType) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            r#type: ptype,
            ..FomodPlugin::default()
        }
    }

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

    #[test]
    fn rule1_notusable_base_type_eliminated_without_patterns() {
        // p0 NotUsable (no type_patterns) is eliminated even with context=None; p1 Optional
        // survives. SelectAny never resolves at usable_count==1, so the domain is inspectable.
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
        // mutation sensitivity: if the elimination were skipped, usable_count would be 2 and p0
        // would still be true.
        assert!(!r.narrowed_domains[0][0][0]);
    }

    #[test]
    fn rule1_dynamic_notusable_guarded_without_context_eliminated_with_context() {
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

        let none = run_no_atoms(&make(), None);
        assert_eq!(none.narrowed_domains[0][0], vec![true, true]);
        assert_eq!(none.plugin_reasons[0][0][0], ReasonCode::ImplicitDefault);

        let ctx = FomodDependencyContext::default();
        let some = run_no_atoms(&make(), Some(&ctx));
        assert_eq!(some.narrowed_domains[0][0], vec![false, true]);
        assert_eq!(some.plugin_reasons[0][0][0], ReasonCode::ForcedNotUsable);
    }

    #[test]
    fn rule1_required_records_reason_but_does_not_pin_or_eliminate_sibling() {
        // SelectExactlyOne with a Required p0 and an Optional p1. if Required were pinned (siblings
        // eliminated), usable_count would drop to 1 and the group would resolve. rule 1 does
        // neither: usable_count stays 2, the group stays unresolved, and only a reason is recorded.
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

    #[test]
    fn rule2_unique_miss_eliminates_and_unique_hit_records_evidence() {
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
        // p0 has only always_install atoms, p1 only install_if_usable atoms, both missing target.
        // auto atoms never enter plugin_dests, so neither plugin is eliminated.
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
        // usable_count == 1 -> not resolved, because select-zero stays valid.
        let one = installer_one_step(vec![group(
            FomodGroupType::SelectAtMostOne,
            vec![plugin("p0", PluginType::Optional)],
        )]);
        let r1 = run_no_atoms(&one, None);
        assert!(!r1.fully_resolved);
        assert!(r1.resolved_groups.is_empty());
        assert_eq!(r1.narrowed_domains[0][0], vec![true]);

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

    #[test]
    fn resolved_by_attribution_select_all_evidence_and_cardinality() {
        let sa = installer_one_step(vec![group(
            FomodGroupType::SelectAll,
            vec![plugin("p0", PluginType::Optional)],
        )]);
        assert_eq!(
            run_no_atoms(&sa, None).resolved_by[0][0],
            "propagation.select_all"
        );

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

    #[test]
    fn fixpoint_flag_set_by_later_group_resolves_earlier_group_next_iteration() {
        // document order: group 0 = B (reads flag F), group 1 = A (sets F). in iteration 0, B is
        // visited before A with the flag unset and cannot resolve; A then resolves and sets F. b
        // can only resolve when revisited in iteration 1, which proves the fixpoint did more than
        // one pass. a context is supplied because B's resolution relies on a type_pattern flipping
        // a plugin to NotUsable, which only prunes with a context (the dynamic-without-context
        // guard blocks it during context-free runs).
        let make = |a_flag_value: &str| {
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
            let mut pa = plugin("pA", PluginType::Optional);
            pa.condition_flags = vec![("F".to_string(), a_flag_value.to_string())];
            let a = group(FomodGroupType::SelectAll, vec![pa]);
            installer_one_step(vec![b, a])
        };

        let ctx = FomodDependencyContext::default();

        let inst = make("on");
        let r = run_no_atoms(&inst, Some(&ctx));
        assert!(
            r.fully_resolved,
            "flag propagation must resolve B on a later pass"
        );
        assert_eq!(r.narrowed_domains[0][0], vec![false, true]);
        assert!(r.resolved_groups.contains(&(0, 0)));
        assert!(r.resolved_groups.contains(&(0, 1)));

        // mutation: A sets F = "off" -> B's pattern never matches -> pB0 stays Optional, B keeps
        // usable_count 2 and never resolves.
        let inst_off = make("off");
        let r_off = run_no_atoms(&inst_off, Some(&ctx));
        assert!(!r_off.fully_resolved);
        assert_eq!(r_off.narrowed_domains[0][0], vec![true, true]);
        assert!(!r_off.resolved_groups.contains(&(0, 0)));
    }

    #[test]
    fn record_plugin_reason_keeps_first_code() {
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
        assert_eq!(r.plugin_reasons[0][0][0], ReasonCode::UniqueFileEvidence);
        assert_eq!(
            r.plugin_reason_details[0][0][0],
            Some(ReasonDetail::UniqueFileEvidence {
                files: vec!["hit.dds".to_string()],
                count: 1,
            })
        );
    }

    #[test]
    fn fully_resolved_true_when_all_groups_resolve_false_otherwise() {
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
