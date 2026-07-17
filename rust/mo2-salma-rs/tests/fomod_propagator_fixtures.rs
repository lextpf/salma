//! Fixture-driven constraint-propagator tests over the committed golden cases
//! in `rust/tests/golden/cases/` (Task 7).
//!
//! For every committed fixture this reuses the shared [`common`] harness (Task
//! 5 input-prep + the Task 6 selection-grid reconstructor) to parse the IR,
//! expand atoms, and build the atom index / excluded set / target tree exactly
//! as the engine does, then runs [`propagate`] with `overrides = default` and
//! `context = None` (the standalone-inference call the orchestrator uses:
//! `FomodInferenceService.cpp` passes `nullptr` context).
//!
//! The central correctness invariant is SOUNDNESS: propagation must never prune
//! the plugin the C++ solver actually selected. Every fixture is checked against
//! its `expected.json` selection grid. For the fixtures propagation fully
//! resolves, the narrowed domain IS the selection, so it must equal the grid
//! exactly.

mod common;

use mo2_salma_rs::fomod_csp_types::InferenceOverrides;
use mo2_salma_rs::fomod_propagator::propagate;

/// Number of committed fixtures for which propagation fully resolves every
/// group. Pinned so a silent flip (a fixture that starts or stops fully
/// resolving) is caught, mirroring the Task 6 12/3 split guard. The WHICH is
/// derived at runtime (never hardcoded); only the count is asserted. Observed
/// resolvers: `sevenz_selectall_hh_walk` (a `SelectAll` "Base Files" group plus
/// a 3-plugin `SelectExactlyOne` "Version" group that resolves by file-evidence
/// elimination, keeping A while pruning B and mixed as NO_FILE_EVIDENCE) and
/// `sevenz_3step_tk_dodge` (two `SelectAll` groups plus a single-plugin
/// `SelectExactlyOne` group that resolves trivially at usable_count == 1 by
/// cardinality alone, with no file evidence).
const FULLY_RESOLVED_COUNT: usize = 2;

/// Run `propagate` for a prepared fixture case with the standalone-inference
/// arguments (overrides = default, context = None).
fn propagate_case(run: &common::CaseRun) -> mo2_salma_rs::fomod_propagator::PropagationResult {
    propagate(
        &run.installer,
        &run.atoms,
        &run.index,
        &run.target,
        &run.excluded,
        &InferenceOverrides::default(),
        None,
    )
}

/// The result's nested vectors must mirror the installer hierarchy exactly.
#[test]
fn result_shapes_match_installer_hierarchy() {
    let cases = common::committed_cases();
    assert!(!cases.is_empty(), "expected committed fixtures on disk");

    for case in cases {
        let run = common::run_case(&case);
        let r = propagate_case(&run);

        assert_eq!(
            r.narrowed_domains.len(),
            run.installer.steps.len(),
            "{case}: step count"
        );
        assert_eq!(r.plugin_reasons.len(), run.installer.steps.len());
        assert_eq!(r.plugin_reason_details.len(), run.installer.steps.len());
        assert_eq!(r.resolved_by.len(), run.installer.steps.len());

        for (si, step) in run.installer.steps.iter().enumerate() {
            assert_eq!(
                r.narrowed_domains[si].len(),
                step.groups.len(),
                "{case}: step {si} groups"
            );
            assert_eq!(r.plugin_reasons[si].len(), step.groups.len());
            assert_eq!(r.plugin_reason_details[si].len(), step.groups.len());
            assert_eq!(r.resolved_by[si].len(), step.groups.len());
            for (gi, group) in step.groups.iter().enumerate() {
                let n = group.plugins.len();
                assert_eq!(
                    r.narrowed_domains[si][gi].len(),
                    n,
                    "{case}: step {si} group {gi} plugins"
                );
                assert_eq!(r.plugin_reasons[si][gi].len(), n);
                assert_eq!(r.plugin_reason_details[si][gi].len(), n);
            }
        }
    }
}

/// SOUNDNESS (the key correctness invariant): propagation never eliminates a
/// truly-selected plugin. For every `(si, gi, pi)` selected in `expected.json`,
/// the narrowed domain must still be usable.
#[test]
fn propagation_never_eliminates_a_selected_plugin() {
    for case in common::committed_cases() {
        let run = common::run_case(&case);
        let expected = common::load_expected(&case);
        let grid = common::build_selection_grid(&run.installer, &expected, &case);
        let r = propagate_case(&run);

        for (si, step_grid) in grid.iter().enumerate() {
            for (gi, group_grid) in step_grid.iter().enumerate() {
                for (pi, &selected) in group_grid.iter().enumerate() {
                    if selected {
                        assert!(
                            r.narrowed_domains[si][gi][pi],
                            "{case}: propagation pruned selected plugin at \
                             step {si} group {gi} plugin {pi}"
                        );
                    }
                }
            }
        }
    }
}

/// For every fixture propagation fully resolves, the narrowed domain equals the
/// C++ selection grid exactly (a resolved group's narrowed domain IS the
/// selection). The count of fully-resolved fixtures is pinned; the identities
/// are derived at runtime.
#[test]
fn fully_resolved_fixtures_match_the_selection_grid() {
    let mut fully_resolved = 0usize;
    let mut names: Vec<String> = Vec::new();

    for case in common::committed_cases() {
        let run = common::run_case(&case);
        let expected = common::load_expected(&case);
        let grid = common::build_selection_grid(&run.installer, &expected, &case);
        let r = propagate_case(&run);

        if r.fully_resolved {
            fully_resolved += 1;
            names.push(case.clone());
            assert_eq!(
                r.narrowed_domains, grid,
                "{case}: fully-resolved narrowed_domains must equal the \
                 expected.json selection grid"
            );
        }
    }

    assert_eq!(
        fully_resolved, FULLY_RESOLVED_COUNT,
        "number of committed fixtures propagation fully resolves (resolved: {names:?})"
    );
}

/// Propagation is deterministic: two runs over the same fixture produce
/// identical domains, resolved groups, reasons, and detail file lists (proving
/// the unordered-set handling in rule 2 is pinned to a stable order).
#[test]
fn propagation_is_deterministic() {
    for case in common::committed_cases() {
        let run = common::run_case(&case);
        let a = propagate_case(&run);
        let b = propagate_case(&run);
        // PropagationResult derives PartialEq, so this compares narrowed_domains,
        // resolved_groups, fully_resolved, plugin_reasons, plugin_reason_details
        // (including the UniqueFileEvidence file lists), and resolved_by.
        assert_eq!(a, b, "{case}: propagate is not deterministic");
    }
}
