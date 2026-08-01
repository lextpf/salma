//! Fixture-driven CSP precompute + option-enumeration tests over the committed
//! golden cases in `rust/tests/golden/cases/` (Task 8).
//!
//! ## Oracle limitation
//!
//! Task 8's outputs (the `Precompute` reverse indices and per-group option
//! lists) are NOT recorded in the fixtures' `expected.json` - byte-parity
//! against recorded C++ intermediates is not available (that resumes at Task 9,
//! which produces the final selection grid). So these fixture tests are
//! STRUCTURAL + DETERMINISM only; the behavioral parity lives in the
//! hand-derived micro-tests inside `src/fomod_csp_precompute.rs` and
//! `src/fomod_csp_options.rs`. See `PARITY-NOTES.md` "Task 8".
//!
//! Each fixture reuses the shared [`common`] harness (Task 5 input-prep) to
//! build installer/atoms/index/excluded/target, then builds the flat GroupRef
//! list and per-plugin evidence exactly as the real caller (`solve_fomod_csp` in
//! `src/FomodCSPSolver.cpp`) does before `build_precompute`.

mod common;

use std::collections::HashMap;

use mo2_salma_rs::fomod_csp_options::get_options_for_group;
use mo2_salma_rs::fomod_csp_precompute::{build_precompute, compute_evidence};
use mo2_salma_rs::fomod_csp_types::{
    CachedOptions, GroupRef, OptionCacheKey, Precompute, SELECT_ANY_CAP_NARROW, SolverStats,
};
use mo2_salma_rs::fomod_ir::{FomodGroupType, FomodInstaller, total_flat_plugins};

/// Group ordering priority, mirror of the `group_priority` lambda in
/// `solve_fomod_csp` (`src/FomodCSPSolver.cpp`).
fn group_priority(t: FomodGroupType) -> i32 {
    match t {
        FomodGroupType::SelectAll => 4,
        FomodGroupType::SelectExactlyOne => 3,
        FomodGroupType::SelectAtMostOne => 2,
        FomodGroupType::SelectAtLeastOne => 1,
        FomodGroupType::SelectAny => 0,
    }
}

/// Build the flat `GroupRef` list the CSP solver entry point passes to
/// `build_precompute`: groups in document order (with document-order
/// `flat_start`), then sorted WITHIN each step by group priority DESC then
/// plugin_count ASC.
///
/// The C++ per-step sort is an UNSTABLE `std::sort`; equal (priority,
/// plugin_count) ties have unspecified order there. This port uses a STABLE
/// sort, so ties keep document order - deterministic run-to-run (the tests below
/// rely only on determinism, not on matching a specific C++ tie order).
fn build_group_refs(installer: &FomodInstaller) -> Vec<GroupRef> {
    let mut groups: Vec<GroupRef> = Vec::new();
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

    for si in 0..installer.steps.len() as i32 {
        let begin = groups.iter().position(|g| g.step_idx == si);
        let Some(begin) = begin else {
            continue;
        };
        let end = groups.iter().rposition(|g| g.step_idx == si).unwrap() + 1;
        groups[begin..end].sort_by(|a, b| {
            let pa =
                group_priority(installer.steps[si as usize].groups[a.group_idx as usize].r#type);
            let pb =
                group_priority(installer.steps[si as usize].groups[b.group_idx as usize].r#type);
            pb.cmp(&pa).then(a.plugin_count.cmp(&b.plugin_count))
        });
    }

    groups
}

/// Build a `Precompute` for a prepared fixture case, mirroring the real caller.
fn build_case_precompute<'a>(run: &'a common::CaseRun) -> Precompute<'a> {
    let groups = build_group_refs(&run.installer);
    let evidence = compute_evidence(
        &run.installer,
        &run.atoms,
        &run.index,
        &run.target,
        &run.excluded,
    );
    build_precompute(
        &run.installer,
        &run.atoms,
        &run.index,
        &run.target,
        &run.excluded,
        None,
        None,
        groups,
        evidence,
    )
}

/// A vector is strictly ascending (sorted AND deduped).
fn is_sorted_dedup(v: &[i32]) -> bool {
    v.windows(2).all(|w| w[0] < w[1])
}

fn assert_reverse_indices_sorted(pre: &Precompute<'_>, case: &str) {
    for (dest, v) in &pre.dest_to_groups {
        assert!(
            is_sorted_dedup(v),
            "{case}: dest_to_groups[{dest}] not sorted/deduped"
        );
    }
    for (dest, v) in &pre.dest_to_plugins {
        assert!(
            is_sorted_dedup(v),
            "{case}: dest_to_plugins[{dest}] not sorted/deduped"
        );
    }
    for (dest, v) in &pre.dest_to_size_match_groups {
        assert!(
            is_sorted_dedup(v),
            "{case}: dest_to_size_match_groups[{dest}] not sorted/deduped"
        );
    }
    for (dest, v) in &pre.dest_to_hash_capable_groups {
        assert!(
            is_sorted_dedup(v),
            "{case}: dest_to_hash_capable_groups[{dest}] not sorted/deduped"
        );
    }
    for (flag, v) in &pre.flag_to_setter_groups {
        assert!(
            is_sorted_dedup(v),
            "{case}: flag_to_setter_groups[{flag}] not sorted/deduped"
        );
    }
    assert!(
        is_sorted_dedup(&pre.contested_plugins),
        "{case}: contested_plugins not sorted/deduped"
    );
}

/// Structural invariants: shapes match the installer hierarchy and every
/// reverse-index vector is sorted-ascending and deduped. Exercises every
/// committed fixture; the count is pinned so a fixture appearing/disappearing is
/// caught.
#[test]
fn precompute_shapes_and_sorted_indices_over_fixtures() {
    let cases = common::committed_cases();
    assert!(!cases.is_empty(), "expected committed fixtures on disk");
    let mut exercised = 0usize;

    for case in &cases {
        let run = common::run_case(case);
        let pre = build_case_precompute(&run);

        let total_plugins = total_flat_plugins(&run.installer) as usize;
        let total_groups: usize = run.installer.steps.iter().map(|s| s.groups.len()).sum();

        assert_eq!(pre.groups.len(), total_groups, "{case}: group count");
        assert_eq!(pre.evidence.len(), total_plugins, "{case}: evidence len");
        assert_eq!(
            pre.plugin_to_group.len(),
            total_plugins,
            "{case}: plugin_to_group len"
        );
        assert_eq!(
            pre.plugin_unique_support.len(),
            total_plugins,
            "{case}: plugin_unique_support len"
        );

        // Per-group derived vectors are sized to the group count.
        assert_eq!(pre.group_sets_flags.len(), total_groups);
        assert_eq!(pre.group_reads_flags.len(), total_groups);
        assert_eq!(pre.group_cache_flags.len(), total_groups);
        assert_eq!(pre.group_dests.len(), total_groups);

        // plugin_to_group maps every flat plugin to a valid group index.
        for (flat, &gidx) in pre.plugin_to_group.iter().enumerate() {
            assert!(
                gidx >= 0 && (gidx as usize) < total_groups,
                "{case}: plugin_to_group[{flat}] = {gidx} out of range"
            );
        }

        // Every group_cache_flags list is byte-ascending (the hash key list).
        for (gidx, keys) in pre.group_cache_flags.iter().enumerate() {
            assert!(
                keys.windows(2).all(|w| w[0] < w[1]),
                "{case}: group_cache_flags[{gidx}] not byte-ascending"
            );
        }
        // memo_flags is byte-ascending.
        assert!(
            pre.memo_flags.windows(2).all(|w| w[0] < w[1]),
            "{case}: memo_flags not byte-ascending"
        );

        assert_reverse_indices_sorted(&pre, case);

        // Component decomposition partitions exactly [0, total_groups).
        let mut members: Vec<i32> = pre.components.iter().flatten().copied().collect();
        members.sort_unstable();
        let expected: Vec<i32> = (0..total_groups as i32).collect();
        assert_eq!(
            members, expected,
            "{case}: components do not partition groups"
        );
        // Components are ordered size-descending.
        assert!(
            pre.components.windows(2).all(|w| w[0].len() >= w[1].len()),
            "{case}: components not size-descending"
        );

        exercised += 1;
    }

    // 15 committed fixtures ship a ModuleConfig.xml (one empty case does not).
    assert_eq!(exercised, 15, "fixtures exercised");
    assert_eq!(cases.len(), 15);
}

/// Determinism: building the Precompute twice from the same inputs yields an
/// identical structure (all deterministic fields, including the component
/// decomposition and its ordering).
#[test]
fn precompute_is_deterministic_over_fixtures() {
    for case in common::committed_cases() {
        let run = common::run_case(&case);
        let pre_a = build_case_precompute(&run);
        let pre_b = build_case_precompute(&run);
        // Precompute derives PartialEq: compares the borrowed inputs (same
        // pointees) and every owned reverse index, flag graph, and component.
        assert_eq!(
            pre_a, pre_b,
            "{case}: build_precompute is not deterministic"
        );
        assert_eq!(
            pre_a.components, pre_b.components,
            "{case}: components differ"
        );
    }
}

/// For a couple of groups per fixture, `get_options_for_group` returns a stable
/// option count across two independent Precompute builds, and every option is a
/// valid mask whose length equals the group's plugin count.
#[test]
fn get_options_for_group_is_stable_and_masks_are_valid() {
    for case in common::committed_cases() {
        let run = common::run_case(&case);
        let pre_a = build_case_precompute(&run);
        let pre_b = build_case_precompute(&run);

        // Probe up to the first two groups.
        let probe: Vec<i32> = (0..pre_a.groups.len().min(2) as i32).collect();
        for &gidx in &probe {
            let count_a = {
                let mut cache: HashMap<OptionCacheKey, CachedOptions> = HashMap::new();
                let mut stats = SolverStats {
                    logged_group_options: vec![false; pre_a.groups.len()],
                    ..SolverStats::default()
                };
                let opts = get_options_for_group(
                    gidx,
                    &pre_a,
                    &HashMap::new(),
                    SELECT_ANY_CAP_NARROW,
                    None,
                    &mut cache,
                    &mut stats,
                );
                // Every option is a valid mask of the group's plugin count.
                let pc = pre_a.groups[gidx as usize].plugin_count as usize;
                for opt in &opts.options {
                    assert_eq!(
                        opt.len(),
                        pc,
                        "{case}: group {gidx} option length != plugin count"
                    );
                }
                // profiles parallel the options.
                assert_eq!(opts.options.len(), opts.profiles.len());
                assert!(
                    !opts.options.is_empty(),
                    "{case}: group {gidx} has no options"
                );
                opts.options.len()
            };

            let count_b = {
                let mut cache: HashMap<OptionCacheKey, CachedOptions> = HashMap::new();
                let mut stats = SolverStats {
                    logged_group_options: vec![false; pre_b.groups.len()],
                    ..SolverStats::default()
                };
                let opts = get_options_for_group(
                    gidx,
                    &pre_b,
                    &HashMap::new(),
                    SELECT_ANY_CAP_NARROW,
                    None,
                    &mut cache,
                    &mut stats,
                );
                opts.options.len()
            };

            assert_eq!(
                count_a, count_b,
                "{case}: group {gidx} option count not stable across runs"
            );
        }
    }
}
