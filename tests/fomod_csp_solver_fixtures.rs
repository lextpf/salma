//! Fixture-driven CSP-solver tests over the committed golden cases in
//! `rust/tests/golden/cases/` (Task 9).
//!
//! The golden `expected.json` is the ORACLE: it records the final selection
//! grid, `diagnostics.repro` (four error counters + `reproduced`),
//! `diagnostics.exact_match`, and `diagnostics.phase_reached` from the
//! authoritative C++ DLL run.
//!
//! ## The archive-listing size discrepancy (Task 6 PARITY-NOTES)
//!
//! The golden inference run recorded `atom.file_size = 0` for many output atoms
//! (its live archive listing did not populate uncompressed sizes for those
//! entries), whereas the committed `archive_entries.json` snapshots carry
//! populated sizes. For 3 fixtures - `rar_7step_sos`, `sevenz_2step_nec_feet`,
//! `zip_11step_cbbe_3ba` - some populated sizes differ from the installed target
//! size, so an end-to-end run over the FIXTURE atoms reports `size_mismatch`
//! where the golden run (size 0 -> size check skipped) reported `reproduced`.
//! This is a Task 2 fixture-data discrepancy, NOT a solver defect (see
//! `PARITY-NOTES.md` "Task 6" / "Task 9").
//!
//! Consequences for the SOLVER, which optimizes against the fixture atoms:
//! - The 12 "consistent" fixtures (expected-grid metrics == `diagnostics.repro`)
//!   solve to full parity - grid, all five repro counters, `exact_match`, and
//!   `phase_reached`.
//! - `rar_7step_sos` and `sevenz_2step_nec_feet` terminate quickly but score the
//!   extra size mismatches, so `exact_match`/`phase_reached`/`size_mismatch`
//!   diverge; only the size-INDEPENDENT invariants match.
//! - `zip_11step_cbbe_3ba` would (correctly) reach `exact` in the greedy phase
//!   with the golden size-0 atoms, but with the fixture atoms it can never reach
//!   exact and searches the full 100-plugin space up to the wall-clock deadline.
//!   It is excluded from the solver-driving tests (documented, pinned by name).
//!
//! `nodes_explored` is never asserted (visitation-order dependent).

mod common;

use std::collections::HashSet;

use mo2_salma_rs::fomod_csp_solver::solve_fomod_csp;
use mo2_salma_rs::fomod_csp_types::{InferenceOverrides, ReproMetrics, SolverResult};
use mo2_salma_rs::fomod_forward_simulator::{compare_trees, simulate};
use mo2_salma_rs::fomod_propagator::{PropagationResult, propagate};

use common::minijson::Value;

/// Number of committed fixtures whose expected grid, simulated with the fixture
/// atoms, reproduces `diagnostics.repro` exactly (the archive-consistent set).
/// Pinned so a fixture silently entering or leaving the set fails the test.
const CONSISTENT_COUNT: usize = 12;

/// Fixtures whose committed archive sizes diverge from the golden run's (see the
/// module doc). Pinned by name; the runtime-derived inconsistent set must equal
/// this exactly.
const INCONSISTENT: [&str; 3] = [
    "rar_7step_sos",
    "sevenz_2step_nec_feet",
    "zip_11step_cbbe_3ba",
];

/// The single inconsistent fixture whose solver run does not terminate within a
/// test-time budget (the 100-plugin search never short-circuits on exact).
const NON_TERMINATING: &str = "zip_11step_cbbe_3ba";

/// Number of consistent fixtures whose returned grid equals the C++ grid
/// byte-for-byte (the exact-grid deterministic subset). Every consistent fixture
/// matches, so this equals `CONSISTENT_COUNT`; pinned separately so a grid
/// regression on any single fixture is caught even if metrics still match.
const EXACT_GRID_COUNT: usize = 12;

fn propagate_case(run: &common::CaseRun) -> PropagationResult {
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

/// Run the full solve for a prepared fixture with the standalone-inference
/// arguments (default overrides - behaviorally identical to `None` since the
/// vectors are empty - and propagation from `propagate_case`).
fn solve_case(run: &common::CaseRun) -> SolverResult {
    let prop = propagate_case(run);
    let overrides = InferenceOverrides::default();
    solve_fomod_csp(
        &run.installer,
        &run.atoms,
        &run.index,
        &run.target,
        &run.excluded,
        Some(&overrides),
        Some(&prop),
    )
}

/// Recompute a selection grid's metrics exactly as the C++ oracle does
/// (`simulate` -> `compare_trees`).
fn grid_metrics(run: &common::CaseRun, grid: &[Vec<Vec<bool>>]) -> ReproMetrics {
    let overrides = InferenceOverrides::default();
    let sim = simulate(&run.installer, &run.atoms, grid, None, Some(&overrides));
    compare_trees(&sim, &run.target, &run.excluded)
}

fn expected_repro(expected: &Value) -> ReproMetrics {
    let repro = expected
        .member("diagnostics")
        .expect("diagnostics")
        .member("repro")
        .expect("repro");
    let get = |k: &str| repro.member(k).expect(k).as_u64() as i32;
    ReproMetrics {
        missing: get("missing"),
        extra: get("extra"),
        size_mismatch: get("size_mismatch"),
        hash_mismatch: get("hash_mismatch"),
        reproduced: get("reproduced"),
    }
}

fn expected_exact(expected: &Value) -> bool {
    expected
        .member("diagnostics")
        .expect("diagnostics")
        .member("exact_match")
        .expect("exact_match")
        .as_bool()
}

fn expected_phase(expected: &Value) -> String {
    expected
        .member("diagnostics")
        .expect("diagnostics")
        .member("phase_reached")
        .expect("phase_reached")
        .as_str()
        .to_string()
}

/// A fixture is archive-consistent when its expected grid, simulated with the
/// fixture atoms, reproduces `diagnostics.repro` exactly. Derived at runtime (no
/// hardcoded membership) so a fixture-data change is caught by the pinned counts.
fn is_consistent(case: &str) -> bool {
    let run = common::run_case(case);
    let expected = common::load_expected(case);
    let grid = common::build_selection_grid(&run.installer, &expected, case);
    grid_metrics(&run, &grid) == expected_repro(&expected)
}

/// The archive-consistent fixtures, in sorted order.
fn consistent_cases() -> Vec<String> {
    common::committed_cases()
        .into_iter()
        .filter(|c| is_consistent(c))
        .collect()
}

/// PRIMARY parity: every archive-consistent fixture reproduces the C++ oracle's
/// `diagnostics.repro`, `exact_match`, and `phase_reached`. Robust to grid ties
/// (metrics are recomputed on the returned grid). The consistent-set size is
/// pinned.
#[test]
fn metrics_parity_over_consistent_fixtures() {
    let cases = consistent_cases();
    assert_eq!(
        cases.len(),
        CONSISTENT_COUNT,
        "archive-consistent fixture count (consistent: {cases:?})"
    );

    for case in cases {
        let run = common::run_case(&case);
        let expected = common::load_expected(&case);
        let res = solve_case(&run);

        let m = grid_metrics(&run, &res.selections);
        assert_eq!(m, expected_repro(&expected), "{case}: repro metrics");
        assert_eq!(
            res.exact_match,
            expected_exact(&expected),
            "{case}: exact_match"
        );
        assert_eq!(
            res.phase_reached,
            expected_phase(&expected),
            "{case}: phase_reached"
        );
    }
}

/// EXACT-GRID parity for the deterministic subset: every archive-consistent
/// fixture returns the C++ grid byte-for-byte. The two propagation-fully-resolved
/// fixtures are guaranteed members (asserted by name); the count is pinned so a
/// grid regression on any single fixture surfaces even when metrics still match.
#[test]
fn exact_grid_parity_for_consistent_subset() {
    let mut matched: Vec<String> = Vec::new();
    let mut metrics_only: Vec<String> = Vec::new();

    for case in consistent_cases() {
        let run = common::run_case(&case);
        let expected = common::load_expected(&case);
        let res = solve_case(&run);
        let grid = common::build_selection_grid(&run.installer, &expected, &case);

        if res.selections == grid {
            matched.push(case);
        } else {
            metrics_only.push(case);
        }
    }

    for guaranteed in ["sevenz_selectall_hh_walk", "sevenz_3step_tk_dodge"] {
        assert!(
            matched.iter().any(|c| c == guaranteed),
            "expected {guaranteed} to be an exact-grid member; matched = {matched:?}"
        );
    }

    assert_eq!(
        matched.len(),
        EXACT_GRID_COUNT,
        "exact-grid subset size (matched: {matched:?}, metrics-only: {metrics_only:?})"
    );
}

/// The inconsistent set is exactly the documented 3 fixtures, and each diverges
/// from `diagnostics.repro` ONLY on the size_mismatch/reproduced split (the
/// size-INDEPENDENT counters still match). Simulates the KNOWN expected grid, so
/// it never drives the solver - safe for the non-terminating fixture.
#[test]
fn inconsistent_fixtures_diverge_only_on_size_split() {
    let inconsistent: Vec<String> = common::committed_cases()
        .into_iter()
        .filter(|c| !is_consistent(c))
        .collect();

    let want: HashSet<&str> = INCONSISTENT.into_iter().collect();
    let got: HashSet<&str> = inconsistent.iter().map(|s| s.as_str()).collect();
    assert_eq!(got, want, "inconsistent fixture set");

    for case in &inconsistent {
        let run = common::run_case(case);
        let expected = common::load_expected(case);
        let grid = common::build_selection_grid(&run.installer, &expected, case);
        let m = grid_metrics(&run, &grid);
        let d = expected_repro(&expected);

        // Size-independent invariants hold for every fixture.
        assert_eq!(m.missing, d.missing, "{case}: missing");
        assert_eq!(m.extra, d.extra, "{case}: extra");
        assert_eq!(m.hash_mismatch, d.hash_mismatch, "{case}: hash_mismatch");
        assert_eq!(
            m.size_mismatch + m.reproduced,
            d.size_mismatch + d.reproduced,
            "{case}: size+reproduced coverage total"
        );
        // ... and the divergence is confined to the size split.
        assert_ne!(
            m.size_mismatch, d.size_mismatch,
            "{case}: expected a size-split divergence"
        );
    }
}

/// Solver coverage on the two FAST inconsistent fixtures: the returned grid's
/// size-INDEPENDENT metrics (missing, extra, hash_mismatch) still match the C++
/// oracle, and the solve terminates with a valid best. `size_mismatch`,
/// `exact_match`, and `phase_reached` are NOT asserted (the fixture-size
/// discrepancy shifts them). The non-terminating fixture is excluded by name.
#[test]
fn solver_on_fast_inconsistent_fixtures() {
    for case in INCONSISTENT {
        if case == NON_TERMINATING {
            continue;
        }
        let run = common::run_case(case);
        let expected = common::load_expected(case);
        let res = solve_case(&run);

        let m = grid_metrics(&run, &res.selections);
        let d = expected_repro(&expected);
        assert_eq!(m.missing, d.missing, "{case}: missing");
        assert_eq!(m.extra, d.extra, "{case}: extra");
        assert_eq!(m.hash_mismatch, d.hash_mismatch, "{case}: hash_mismatch");

        // A best was recorded and the reported counters agree with the grid.
        assert_eq!(res.missing, m.missing, "{case}: reported missing vs grid");
        assert_eq!(res.extra, m.extra, "{case}: reported extra vs grid");
    }
}

/// Determinism: two solves of one non-trivial consistent fixture and one fast
/// all-phases fixture each produce identical grids and metrics, proving the
/// total-order tiebreaks removed the HashMap-iteration / unstable-sort
/// nondeterminism.
#[test]
fn solve_is_deterministic() {
    // Consistent, exact, 85 reproduced files (a non-trivial SelectExactlyOne).
    let run = common::run_case("zip_exactlyone_racecompat");
    let a = solve_case(&run);
    let b = solve_case(&run);
    assert_eq!(a.selections, b.selections, "racecompat: selections differ");
    assert_eq!(a.exact_match, b.exact_match);
    assert_eq!(a.phase_reached, b.phase_reached);
    assert_eq!(
        (a.missing, a.extra, a.size_mismatch, a.hash_mismatch),
        (b.missing, b.extra, b.size_mismatch, b.hash_mismatch)
    );

    // Fast all-five-phases fixture (drives the whole pipeline, incl. backtrack).
    let run2 = common::run_case("rar_7step_sos");
    let a2 = solve_case(&run2);
    let b2 = solve_case(&run2);
    assert_eq!(
        a2.selections, b2.selections,
        "rar_7step_sos: selections differ"
    );
    assert_eq!(a2.phase_reached, b2.phase_reached);
    assert_eq!(
        (a2.missing, a2.extra, a2.size_mismatch, a2.hash_mismatch),
        (b2.missing, b2.extra, b2.size_mismatch, b2.hash_mismatch)
    );
}
