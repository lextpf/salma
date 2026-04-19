//! Fixture-driven forward-simulator tests over the committed golden cases in
//! `rust/tests/golden/cases/` (Task 6).
//!
//! For every committed fixture this reuses the Task 5 input-prep harness (via
//! the shared [`common`] module) to parse the IR, expand atoms, and build the
//! atom index / excluded set / target tree exactly as the engine does. It then
//! reconstructs the C++ solver's winning `[step][group][plugin]` selection grid
//! from `expected.json`, replays it through [`simulate`], and validates the
//! result against the authoritative C++ output recorded in each `expected.json`.
//!
//! ## Oracle: overrides = None, context = None (trap (n))
//!
//! The C++ golden runs scored candidates with `simulate(..., context=nullptr,
//! overrides=real)` where the overrides came from `compute_overrides`
//! (`FomodInferenceService.cpp`, Task 12 - deliberately NOT ported yet). Those
//! overrides ONLY affect step-visibility conditions and conditional-install
//! patterns, and ONLY when those conditions contain external-dependency leaves
//! (game/file/plugin/fomod/fomm/fose). A scan of all 15 committed
//! ModuleConfig.xml files found ZERO external-dependency leaves inside any
//! `<visible>` block or `<conditionalFileInstalls>` pattern (every such
//! condition is flag-only). For a flag leaf, normal and inferred evaluation are
//! identical regardless of override, so running here with `overrides=None`
//! (normal evaluation) reproduces the C++ golden simulate. See PARITY-NOTES
//! "Task 6" for the per-fixture scan.
//!
//! ## What is asserted (and why not a blanket "exact" on the fixture atoms)
//!
//! The golden inference run recorded `atom.file_size = 0` for many output atoms
//! (its live archive listing did not populate uncompressed sizes for those
//! entries), whereas the committed `archive_entries.json` snapshots carry
//! populated sizes. For 3 fixtures (`rar_7step_sos`, `sevenz_2step_nec_feet`,
//! `zip_11step_cbbe_3ba`) some of those populated sizes differ from the
//! installed target size, so a faithful end-to-end run over the FIXTURE atoms
//! reports `size_mismatch` where the golden run (size 0 -> size check skipped)
//! reported `reproduced`. This is an archive-listing input discrepancy in the
//! Task 2 fixture data, NOT a simulator defect. See PARITY-NOTES "Task 6".
//!
//! The tests therefore validate the simulator and metrics against C++ ground
//! truth in a way that is immune to that input discrepancy:
//!   1. [`simulate`] reproduces the C++ `outputTree` winning-atom SOURCE for
//!      every destination (conflict resolution is size-independent).
//!   2. [`compare_trees`] fed the reconstructed C++ `outputTree` (with the
//!      golden atom sizes) reproduces each fixture's `diagnostics.repro`
//!      exactly, including the `exact_match` flag.
//!   3. An end-to-end run over the fixture atoms matches every size-INDEPENDENT
//!      repro counter (missing, extra, hash_mismatch) and the size-independent
//!      coverage total `size_mismatch + reproduced`.

mod common;

use std::collections::{HashMap, HashSet};

use common::build_selection_grid;
use common::minijson;
use mo2_salma_rs::fomod_atom::FomodAtom;
use mo2_salma_rs::fomod_csp_types::ReproMetrics;
use mo2_salma_rs::fomod_forward_simulator::{SimulatedTree, compare_trees, simulate};

/// Read a fixture's `diagnostics.repro` block into a [`ReproMetrics`].
fn expected_metrics(expected: &minijson::Value) -> ReproMetrics {
    let repro = expected
        .member("diagnostics")
        .expect("diagnostics")
        .member("repro")
        .expect("repro");
    let get = |k: &str| {
        repro
            .member(k)
            .unwrap_or_else(|| panic!("repro.{k}"))
            .as_u64() as i32
    };
    ReproMetrics {
        missing: get("missing"),
        extra: get("extra"),
        size_mismatch: get("size_mismatch"),
        hash_mismatch: get("hash_mismatch"),
        reproduced: get("reproduced"),
    }
}

/// Read a fixture's `diagnostics.exact_match` flag.
fn expected_exact_match(expected: &minijson::Value) -> bool {
    expected
        .member("diagnostics")
        .expect("diagnostics")
        .member("exact_match")
        .expect("exact_match")
        .as_bool()
}

/// Reconstruct the C++ simulated tree from a fixture's `outputTree`
/// ({path, size, source}), carrying the GOLDEN atom sizes the C++ run recorded.
/// `content_hash` is left 0 (the golden repro has `hash_mismatch == 0` for
/// every fixture, so hash comparison never changes the outcome).
fn reconstruct_cpp_tree(expected: &minijson::Value) -> SimulatedTree {
    // These fixtures never truncate (all well under the 5000-entry cap).
    assert!(
        expected.member("outputTreeTruncated").is_none(),
        "outputTree unexpectedly truncated"
    );
    let mut files = HashMap::new();
    for e in expected
        .member("outputTree")
        .expect("outputTree")
        .as_array()
    {
        let dest = e.member("path").expect("path").as_str().to_string();
        let atom = FomodAtom {
            dest_path: dest.clone(),
            source_path: e.member("source").expect("source").as_str().to_string(),
            file_size: e.member("size").expect("size").as_u64(),
            ..FomodAtom::default()
        };
        files.insert(dest, atom);
    }
    SimulatedTree { files }
}

/// Map each simulated destination to its winning atom's source path.
fn dest_to_source(sim: &SimulatedTree) -> HashMap<String, String> {
    sim.files
        .iter()
        .map(|(dest, atom)| (dest.clone(), atom.source_path.clone()))
        .collect()
}

/// (1) The Rust simulator reproduces the C++ `outputTree` winning-atom SOURCE
/// for every destination, over ALL 15 fixtures. This validates the simulator's
/// full four-phase conflict resolution directly against C++ ground truth and is
/// independent of the archive-listing size discrepancy (conflict resolution
/// uses only priority/document-order/phase, never file size).
#[test]
fn simulate_reproduces_cpp_output_tree() {
    let cases = common::committed_cases();
    assert!(!cases.is_empty(), "expected committed fixtures on disk");

    for case in cases {
        let run = common::run_case(&case);
        let expected = common::load_expected(&case);
        let grid = build_selection_grid(&run.installer, &expected, &case);

        // Faithful to the C++ golden runs: context = None, overrides = None.
        let sim = simulate(&run.installer, &run.atoms, &grid, None, None);

        let cpp = dest_to_source(&reconstruct_cpp_tree(&expected));
        let mine = dest_to_source(&sim);
        assert_eq!(
            mine, cpp,
            "{case}: simulated dest->source winning-atom map vs C++ outputTree"
        );
    }
}

/// (2) The Rust [`compare_trees`], fed the reconstructed C++ `outputTree`
/// (carrying the golden atom sizes), reproduces each fixture's
/// `diagnostics.repro` counters AND `exact_match` flag exactly. This validates
/// the metrics accounting (the else-chain, missing/extra/reproduced) against
/// real C++ output for all 15 fixtures.
#[test]
fn compare_trees_reproduces_cpp_repro_metrics() {
    for case in common::committed_cases() {
        let run = common::run_case(&case);
        let expected = common::load_expected(&case);
        let cpp_tree = reconstruct_cpp_tree(&expected);

        let metrics = compare_trees(&cpp_tree, &run.target, &run.excluded);
        assert_eq!(
            metrics,
            expected_metrics(&expected),
            "{case}: compare_trees(cpp_tree) vs diagnostics.repro"
        );
        assert_eq!(
            metrics.exact(),
            expected_exact_match(&expected),
            "{case}: metrics.exact() vs diagnostics.exact_match"
        );
    }
}

/// (3) End-to-end (Rust simulate + Rust compare_trees over the FIXTURE atoms):
/// every size-INDEPENDENT repro counter matches the C++ `diagnostics.repro` -
/// `missing`, `extra`, `hash_mismatch`, and the coverage total
/// `size_mismatch + reproduced` (target dests the simulation produced) - for ALL
/// 15 fixtures.
///
/// The FULL metrics (and `exact_match` flag) additionally match for every
/// fixture EXCEPT the archive-listing-inconsistent ones (see the module doc).
/// This test does not hardcode which those are: it compares `m == d` directly,
/// and for any fixture that diverges it asserts the divergence is confined to
/// the size_mismatch/reproduced split (the size-independent invariants above
/// already pin everything else). The 12/3 split is guarded so a fixture silently
/// flipping is caught.
#[test]
fn end_to_end_matches_repro_metrics() {
    let mut full_match = 0usize;
    let mut split_divergent = 0usize;
    for case in common::committed_cases() {
        let run = common::run_case(&case);
        let expected = common::load_expected(&case);
        let grid = build_selection_grid(&run.installer, &expected, &case);
        let sim = simulate(&run.installer, &run.atoms, &grid, None, None);
        let m = compare_trees(&sim, &run.target, &run.excluded);
        let d = expected_metrics(&expected);

        // Size-independent invariants hold for every fixture.
        assert_eq!(m.missing, d.missing, "{case}: missing");
        assert_eq!(m.extra, d.extra, "{case}: extra");
        assert_eq!(m.hash_mismatch, d.hash_mismatch, "{case}: hash_mismatch");
        assert_eq!(
            m.size_mismatch + m.reproduced,
            d.size_mismatch + d.reproduced,
            "{case}: size_mismatch + reproduced coverage total"
        );

        if m == d {
            full_match += 1;
            assert_eq!(
                m.exact(),
                expected_exact_match(&expected),
                "{case}: exact_match"
            );
        } else {
            // The only permitted divergence is the size/reproduced split; the
            // invariants above guarantee everything else already matches.
            assert_ne!(
                m.size_mismatch, d.size_mismatch,
                "{case}: unexpected metric divergence outside the size split"
            );
            split_divergent += 1;
        }
    }
    // 12 fixtures reproduce diagnostics.repro exactly; 3 diverge only on the
    // archive-listing size split documented in PARITY-NOTES "Task 6".
    assert_eq!(full_match, 12, "fixtures fully matching diagnostics.repro");
    assert_eq!(split_divergent, 3, "fixtures diverging on the size split");
}

/// (4) For two diverse fixtures that are exact over the fixture atoms - one
/// driven entirely by conditionalFileInstalls (`sevenz_4step_lewdmarks`, 0
/// plugin atoms / 36 conditional atoms) and one plain SelectExactlyOne fixture
/// (`zip_exactlyone_mu_joint_fix`) - assert the simulated destination SET equals
/// the target set (not just counters), and every simulated atom's file_size
/// matches the target size wherever both are nonzero.
#[test]
fn exact_fixtures_reproduce_the_full_dest_set_and_sizes() {
    for case in ["sevenz_4step_lewdmarks", "zip_exactlyone_mu_joint_fix"] {
        let run = common::run_case(case);
        let expected = common::load_expected(case);

        let grid = build_selection_grid(&run.installer, &expected, case);
        let sim = simulate(&run.installer, &run.atoms, &grid, None, None);

        // These two are exact end to end over the fixture atoms.
        let m = compare_trees(&sim, &run.target, &run.excluded);
        assert!(
            m.exact(),
            "{case}: expected exact over fixture atoms, got {m:?}"
        );

        let sim_dests: HashSet<&String> = sim.files.keys().collect();
        let target_dests: HashSet<&String> = run.target.keys().collect();
        assert_eq!(
            sim_dests, target_dests,
            "{case}: simulated dest set must equal target dest set"
        );

        let mut checked_nonzero = 0usize;
        for (dest, atom) in &sim.files {
            let tf = run
                .target
                .get(dest)
                .expect("simulated dest present in target");
            if atom.file_size != 0 && tf.size != 0 {
                assert_eq!(atom.file_size, tf.size, "{case}: file_size of {dest}");
                checked_nonzero += 1;
            }
        }
        assert!(
            checked_nonzero > 0,
            "{case}: expected at least one nonzero-size file to compare"
        );
    }
}
