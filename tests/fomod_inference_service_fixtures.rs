//! End-to-end byte-parity fixture tests for the inference orchestration (Task 12).
//!
//! This suite is the integration capstone's offline gate. For each committed
//! golden case it drives the FULL pipeline from committed inputs -
//! `archive_entries.json` (prep + prefix), `ModuleConfig.xml`
//! (`parse_module_config`), the atom expansion, and `target_tree.json` read WITH
//! its FNV-1a hashes (substituting for the absent mod-dir scan + contested
//! hashing) - then runs the REAL [`compute_overrides`], `propagate`, `solve`,
//! diagnostics, `assemble_json`, and `add_output_tree`, and byte-compares the
//! `dump(2)` against `expected.json`.
//!
//! ## What is NEW vs Task 10
//!
//! Task 10 INJECTED the per-step visibility code from `expected.json` and ran
//! `solve`/`simulate` with DEFAULT (empty) overrides. Task 12 instead computes
//! the overrides with [`compute_overrides`] and threads them through `solve`,
//! `simulate`, AND the diagnostics step-visibility switch - exactly as the real
//! `infer_selections` does. [`computed_step_visibility_matches_expected`] proves
//! `compute_overrides` reproduces the visibility code recorded in every
//! `expected.json`; if any disagrees the pipeline byte-parity would break too.
//!
//! ## The two documented non-reproducible regions (same as Task 10)
//!
//! - `outputTree[].size` is `atom.file_size`; the golden live run under-populated
//!   sizes for many entries (Task 6 / Task 12 PARITY-NOTES), while the committed
//!   `archive_entries.json` snapshots carry populated sizes. Collapsed for the
//!   byte comparison.
//! - `UNIQUE_FILE_EVIDENCE` `files` example arrays come from a `std::unordered_set`
//!   in the C++ (unreproducible order/subset); the Rust propagator sorts them.
//!   Collapsed for the byte comparison.
//!
//! `timings_ms` (wall-clock) is normalized to 0 on BOTH sides.

mod common;

use std::collections::HashSet;

use mo2_salma_rs::fomod_csp_solver::solve_fomod_csp;
use mo2_salma_rs::fomod_csp_types::{InferenceOverrides, ReproMetrics, SolverResult};
use mo2_salma_rs::fomod_dependency_evaluator::ExternalConditionOverride;
use mo2_salma_rs::fomod_forward_simulator::{compare_trees, simulate};
use mo2_salma_rs::fomod_inference_atoms::{add_output_tree, assemble_json};
use mo2_salma_rs::fomod_inference_service::{compute_overrides, try_tier1_cache};
use mo2_salma_rs::fomod_ir::FomodInstaller;
use mo2_salma_rs::fomod_propagator::propagate;
use mo2_salma_rs::inference_diagnostics::{InferenceDiagnosticsBuilder, ReasonCode};
use mo2_salma_rs::json::Value as JVal;

use common::minijson::Value;

/// Number of archive-consistent fixtures (mirror of the pins in the Task 9/10
/// fixture suites). The consistent partition is computed identically here.
const CONSISTENT_COUNT: usize = 12;

/// The single inconsistent fixture whose 100-plugin solve does not short-circuit
/// within a test-time budget; excluded from any solver-driving path.
const NON_TERMINATING: &str = "zip_11step_cbbe_3ba";

/// Consistent fixtures whose full document (with timings normalized to 0) is
/// byte-identical to `expected.json`. Same single member as Task 10
/// (`rar_exactlyone_heel_volume`): the only consistent fixture with neither an
/// `outputTree` size-0 discrepancy nor a multi-hit `UNIQUE_FILE_EVIDENCE` list.
/// Pinned so drift in either direction fails.
const BYTE_EXACT_COUNT: usize = 1;

// --- consistency partition (identical to fomod_csp_solver_fixtures) ----------

fn grid_metrics_default(run: &common::CaseRun, grid: &[Vec<Vec<bool>>]) -> ReproMetrics {
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

/// A fixture is archive-consistent when its expected grid, simulated with the
/// fixture atoms and DEFAULT overrides, reproduces `diagnostics.repro` exactly.
/// Identical definition to `fomod_csp_solver_fixtures.rs` so the partition is the
/// same 12.
fn is_consistent(case: &str) -> bool {
    let run = common::run_case(case);
    let expected = common::load_expected(case);
    let grid = common::build_selection_grid(&run.installer, &expected, case);
    grid_metrics_default(&run, &grid) == expected_repro(&expected)
}

fn consistent_cases() -> Vec<String> {
    common::committed_cases()
        .into_iter()
        .filter(|c| is_consistent(c))
        .collect()
}

// --- real-overrides pipeline (what infer_selections runs) --------------------

/// Prepare a case with the HASHED target tree (substituting for the mod-dir scan
/// + contested hashing that the offline corpus cannot run).
fn run_case_hashed(case: &str) -> common::CaseRun {
    let mut run = common::run_case(case);
    run.target = common::load_target_tree_with_hashes(&common::golden_cases_dir().join(case));
    run
}

/// Feed each step's visibility decision from the COMPUTED overrides through the
/// same switch `infer_selections` uses (SVC 963-982).
fn feed_step_visibility(builder: &mut InferenceDiagnosticsBuilder, overrides: &InferenceOverrides) {
    for (si, mode) in overrides.step_visible.iter().enumerate() {
        match mode {
            ExternalConditionOverride::ForceTrue => {
                builder.set_step_visibility(si as i32, true, ReasonCode::StepVisibilityForced)
            }
            ExternalConditionOverride::ForceFalse => {
                builder.set_step_visibility(si as i32, false, ReasonCode::StepNotVisible)
            }
            ExternalConditionOverride::Unknown => {
                builder.set_step_visibility(si as i32, true, ReasonCode::StepVisibilityUnknown)
            }
        }
    }
}

/// Replay the full diagnostics + assembly chain with real overrides, returning
/// the `dump(2)` string.
fn build_document(run: &common::CaseRun) -> String {
    let overrides = compute_overrides(
        &run.installer,
        &run.atoms,
        &run.index,
        &run.target,
        &run.excluded,
    );
    let prop = propagate(
        &run.installer,
        &run.atoms,
        &run.index,
        &run.target,
        &run.excluded,
        &overrides,
        None,
    );
    let res = solve_fomod_csp(
        &run.installer,
        &run.atoms,
        &run.index,
        &run.target,
        &run.excluded,
        Some(&overrides),
        if prop.resolved_groups.is_empty() {
            None
        } else {
            Some(&prop)
        },
    );

    let mut builder = InferenceDiagnosticsBuilder::new(&run.installer);
    feed_step_visibility(&mut builder, &overrides);
    builder.absorb_propagation(&prop);
    builder.absorb_solver(&res);
    // Timings are normalized away; feed zeros.
    builder.set_run_timings(0, 0, 0, 0);
    builder.set_target_file_count(run.target.len() as i32);
    builder.finalize(&res, &prop, &run.installer);

    let mut out = assemble_json(&run.installer, &res, builder.diagnostics());
    let sim = simulate(
        &run.installer,
        &run.atoms,
        &res.selections,
        None,
        Some(&overrides),
    );
    add_output_tree(&mut out, &sim);
    out.dump(2)
}

/// Collapse the non-reproducible regions of a `dump(2)` document.
///
/// Always collapses the four `timings_ms` members to 0 - CONTEXT-SCOPED to the
/// `timings_ms` object so the identically-named `groups.total` is NOT touched.
/// When `collapse_sizes`/`collapse_files` are set, also collapses
/// `outputTree[].size` (to `N`) and `UNIQUE_FILE_EVIDENCE` `files` arrays.
fn normalize(doc: &str, collapse_sizes: bool, collapse_files: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut lines = doc.lines().peekable();
    // Depth of the `timings_ms` object relative to its own brace (0 = outside).
    let mut in_timings = false;
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];

        if in_timings {
            if trimmed.starts_with('}') {
                in_timings = false;
                out.push(line.to_string());
                continue;
            }
            // A scalar member of timings_ms: "<key>": <n>[,] -> zero it.
            if let Some((key, comma)) = scalar_member(trimmed) {
                out.push(format!("{indent}{key}: 0{comma}"));
                continue;
            }
            out.push(line.to_string());
            continue;
        }

        if trimmed == "\"timings_ms\": {" {
            in_timings = true;
            out.push(line.to_string());
        } else if collapse_sizes && trimmed.starts_with("\"size\": ") {
            let comma = if trimmed.ends_with(',') { "," } else { "" };
            out.push(format!("{indent}\"size\": N{comma}"));
        } else if collapse_files && trimmed == "\"files\": [" {
            let mut saw_close_comma = false;
            for inner in lines.by_ref() {
                let t = inner.trim_start();
                if t == "]" || t == "]," {
                    saw_close_comma = t == "],";
                    break;
                }
            }
            let comma = if saw_close_comma { "," } else { "" };
            out.push(format!("{indent}\"files\": [ ... ]{comma}"));
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\n")
}

/// Split a `"key": value[,]` scalar member line into `("\"key\"", ","|"")`.
fn scalar_member(trimmed: &str) -> Option<(&str, &str)> {
    let key_end = trimmed[1..].find('"').map(|i| i + 2)?; // include both quotes
    let key = &trimmed[..key_end];
    let comma = if trimmed.ends_with(',') { "," } else { "" };
    Some((key, comma))
}

fn first_diff(a: &str, b: &str) -> Option<String> {
    for (i, (la, lb)) in a.lines().zip(b.lines()).enumerate() {
        if la != lb {
            return Some(format!("line {i}:\n  mine:     {la:?}\n  expected: {lb:?}"));
        }
    }
    if a.lines().count() != b.lines().count() {
        return Some(format!(
            "line count differs: mine={} expected={}",
            a.lines().count(),
            b.lines().count()
        ));
    }
    None
}

// --- primary: full byte parity with real compute_overrides -------------------

#[test]
fn full_byte_parity_with_real_overrides() {
    let cases = consistent_cases();
    assert_eq!(
        cases.len(),
        CONSISTENT_COUNT,
        "archive-consistent fixture count (consistent: {cases:?})"
    );

    let mut byte_exact: Vec<String> = Vec::new();
    for case in &cases {
        let run = run_case_hashed(case);
        let expected_text =
            std::fs::read_to_string(common::golden_cases_dir().join(case).join("expected.json"))
                .expect("expected.json readable");
        let mine = build_document(&run);

        // STRONG: the whole document matches once timings, outputTree sizes, and
        // the unordered files arrays are collapsed. Any diff here is a real
        // overrides/serializer/accumulation bug.
        let mine_n = normalize(&mine, true, true);
        let expected_n = normalize(&expected_text, true, true);
        assert!(
            mine_n == expected_n,
            "{case}: normalized document differs (overrides/serializer/accumulation bug)\n{}",
            first_diff(&mine_n, &expected_n).unwrap_or_default()
        );

        // BYTE-EXACT: collapse ONLY timings (context-scoped) on both sides. A
        // fixture qualifies iff it also has no outputTree size-0 discrepancy and
        // no multi-hit UNIQUE_FILE_EVIDENCE list.
        if normalize(&mine, false, false) == normalize(&expected_text, false, false) {
            byte_exact.push(case.clone());
        }
    }

    byte_exact.sort();
    assert_eq!(
        byte_exact.len(),
        BYTE_EXACT_COUNT,
        "fully byte-exact fixtures (timings-normalized): {byte_exact:?}"
    );
}

// --- compute_overrides reproduces the expected step visibility ---------------

fn expected_step_visibility_code(step: &Value) -> Option<&str> {
    let reasons = step.member("reasons").expect("reasons").as_array();
    reasons
        .first()
        .map(|r| r.member("code").expect("code").as_str())
}

fn override_code_name(mode: ExternalConditionOverride) -> &'static str {
    match mode {
        ExternalConditionOverride::ForceTrue => "STEP_VISIBILITY_FORCED",
        ExternalConditionOverride::ForceFalse => "STEP_NOT_VISIBLE",
        ExternalConditionOverride::Unknown => "STEP_VISIBILITY_UNKNOWN",
    }
}

/// For EVERY committed case (consistent or not, terminating or not - no solve is
/// run here), `compute_overrides` must produce the exact per-step visibility code
/// recorded in `expected.json`. This is the load-bearing claim that lets the
/// pipeline byte-parity test drop Task 10's injection.
#[test]
fn computed_step_visibility_matches_expected() {
    for case in common::committed_cases() {
        let run = run_case_hashed(&case);
        let expected = common::load_expected(&case);
        let overrides = compute_overrides(
            &run.installer,
            &run.atoms,
            &run.index,
            &run.target,
            &run.excluded,
        );

        let steps = expected.member("steps").expect("steps").as_array();
        assert_eq!(
            steps.len(),
            overrides.step_visible.len(),
            "{case}: step count vs computed step_visible"
        );
        for (si, step) in steps.iter().enumerate() {
            let Some(want) = expected_step_visibility_code(step) else {
                continue; // no step-level reason recorded (nothing to check)
            };
            let got = override_code_name(overrides.step_visible[si]);
            assert_eq!(got, want, "{case}: step {si} visibility code");
        }
    }
}

// --- skeleton + outputTree over ALL fixtures (covers the size-split subset) ---

type GroupSplit = (Vec<String>, Vec<String>);
type Skeleton = (i64, Vec<Vec<GroupSplit>>, Vec<(String, String)>);

fn skeleton(doc: &Value) -> Skeleton {
    let schema = doc
        .member("schema_version")
        .expect("schema_version")
        .as_u64() as i64;
    let mut steps_out = Vec::new();
    for step in doc.member("steps").expect("steps").as_array() {
        let mut groups_out = Vec::new();
        for group in step.member("groups").expect("groups").as_array() {
            let names = |key: &str| -> Vec<String> {
                group
                    .member(key)
                    .map(|v| {
                        v.as_array()
                            .iter()
                            .map(|p| p.member("name").expect("name").as_str().to_string())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            groups_out.push((names("plugins"), names("deselected")));
        }
        steps_out.push(groups_out);
    }
    let output_tree = doc
        .member("outputTree")
        .map(|v| {
            v.as_array()
                .iter()
                .map(|e| {
                    (
                        e.member("path").expect("path").as_str().to_string(),
                        e.member("source").expect("source").as_str().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    (schema, steps_out, output_tree)
}

/// Drive `assemble_json` from the KNOWN expected grid (so the inconsistent and
/// non-terminating fixtures are covered without solving) with real overrides,
/// then compare the size-independent skeleton + outputTree (path, source) pairs.
#[test]
fn skeleton_and_output_tree_over_all_fixtures() {
    for case in common::committed_cases() {
        let run = run_case_hashed(&case);
        let expected = common::load_expected(&case);
        let overrides = compute_overrides(
            &run.installer,
            &run.atoms,
            &run.index,
            &run.target,
            &run.excluded,
        );

        let grid = common::build_selection_grid(&run.installer, &expected, &case);
        let res = SolverResult {
            selections: grid,
            ..SolverResult::default()
        };

        let mut builder = InferenceDiagnosticsBuilder::new(&run.installer);
        feed_step_visibility(&mut builder, &overrides);
        let prop = propagate(
            &run.installer,
            &run.atoms,
            &run.index,
            &run.target,
            &run.excluded,
            &overrides,
            None,
        );
        builder.absorb_propagation(&prop);
        builder.absorb_solver(&res);
        builder.finalize(&res, &prop, &run.installer);

        let mut out = assemble_json(&run.installer, &res, builder.diagnostics());
        let sim = simulate(
            &run.installer,
            &run.atoms,
            &res.selections,
            None,
            Some(&overrides),
        );
        add_output_tree(&mut out, &sim);

        let mine = common::minijson::parse(&out.dump(2));
        assert_eq!(
            skeleton(&mine),
            skeleton(&expected),
            "{case}: skeleton / schema_version / outputTree path+source"
        );
    }
}

// --- the empty (non-FOMOD) case returns nothing ------------------------------

/// `sevenz_empty_no_moduleconfig` has no `fomod/ModuleConfig.xml`, so
/// `infer_selections` returns "" via the "Not a FOMOD" branch. Offline, the
/// mechanism is that the prefix derivation finds no candidate, and its committed
/// `expected.json` is empty.
#[test]
fn empty_case_is_not_a_fomod() {
    let case = "sevenz_empty_no_moduleconfig";
    let case_dir = common::golden_cases_dir().join(case);
    let raw = common::load_archive_entries(&case_dir);
    assert!(
        common::derive_prefix(&raw).is_none(),
        "{case}: expected no fomod prefix (Not a FOMOD)"
    );
    let expected =
        std::fs::read_to_string(case_dir.join("expected.json")).expect("expected.json readable");
    assert!(expected.is_empty(), "{case}: expected empty output");
}

/// The non-terminating fixture is inconsistent and must never enter the
/// solver-driven consistent set.
#[test]
fn non_terminating_fixture_is_not_consistent() {
    let consistent: HashSet<String> = consistent_cases().into_iter().collect();
    assert!(!consistent.contains(NON_TERMINATING));
}

// --- Tier-1 cache: accept (reproduces) + reject (does not) -------------------

/// Build a fomod-plus blob (`json::Value`) from a `[step][group][plugin]` grid:
/// one `{name, groups:[{name, plugins:[{name}]}]}` entry per IR step/group, with
/// the SELECTED plugins listed under `plugins`.
fn blob_from_grid(installer: &FomodInstaller, grid: &[Vec<Vec<bool>>]) -> JVal {
    let mut steps = JVal::array();
    for (si, step) in installer.steps.iter().enumerate() {
        let mut out_step = JVal::object();
        out_step.insert("name", JVal::string(&step.name));
        let mut groups = JVal::array();
        for (gi, group) in step.groups.iter().enumerate() {
            let mut out_group = JVal::object();
            out_group.insert("name", JVal::string(&group.name));
            let mut plugins = JVal::array();
            for (pi, plugin) in group.plugins.iter().enumerate() {
                if grid[si][gi][pi] {
                    let mut p = JVal::object();
                    p.insert("name", JVal::string(&plugin.name));
                    plugins.push(p);
                }
            }
            out_group.insert("plugins", plugins);
            groups.push(out_group);
        }
        out_step.insert("groups", groups);
        steps.push(out_step);
    }
    let mut blob = JVal::object();
    blob.insert("steps", steps);
    blob
}

/// A consistent fixture that reproduces via Tier-1: `(case, run, grid, overrides)`.
type Tier1Reproducer = (
    String,
    common::CaseRun,
    Vec<Vec<Vec<bool>>>,
    InferenceOverrides,
);

/// Pick the first consistent fixture whose expected grid, run through
/// `try_tier1_cache` with real overrides + hashed target, reproduces exactly.
fn first_tier1_reproducer() -> Option<Tier1Reproducer> {
    for case in consistent_cases() {
        if case == NON_TERMINATING {
            continue;
        }
        let run = run_case_hashed(&case);
        let expected = common::load_expected(&case);
        let grid = common::build_selection_grid(&run.installer, &expected, &case);
        let overrides = compute_overrides(
            &run.installer,
            &run.atoms,
            &run.index,
            &run.target,
            &run.excluded,
        );
        let blob = blob_from_grid(&run.installer, &grid);
        if try_tier1_cache(
            &blob,
            &run.installer,
            &run.atoms,
            &run.target,
            &run.excluded,
            &overrides,
            0,
        )
        .is_hit()
        {
            return Some((case, run, grid, overrides));
        }
    }
    None
}

/// Walk the bespoke document's plugins collecting every reason code, to prove the
/// FOMOD_PLUS_CACHE stamp is present on a selected plugin.
fn any_plugin_reason_code(out: &JVal, want: &str) -> bool {
    let Some(steps) = out.get("steps") else {
        return false;
    };
    for si in 0..steps.array_len().unwrap_or(0) {
        let step = steps.get_index(si).unwrap();
        let Some(groups) = step.get("groups") else {
            continue;
        };
        for gi in 0..groups.array_len().unwrap_or(0) {
            let group = groups.get_index(gi).unwrap();
            for key in ["plugins", "deselected"] {
                let Some(arr) = group.get(key) else { continue };
                for pi in 0..arr.array_len().unwrap_or(0) {
                    let plugin = arr.get_index(pi).unwrap();
                    let Some(reasons) = plugin.get("reasons") else {
                        continue;
                    };
                    for ri in 0..reasons.array_len().unwrap_or(0) {
                        if reasons
                            .get_index(ri)
                            .unwrap()
                            .get("code")
                            .and_then(JVal::as_str)
                            == Some(want)
                        {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

#[test]
fn tier1_cache_accept_emits_bespoke_json() {
    let (case, run, _grid, overrides) =
        first_tier1_reproducer().expect("at least one consistent fixture reproduces via Tier-1");
    let expected = common::load_expected(&case);
    let grid = common::build_selection_grid(&run.installer, &expected, &case);
    let blob = blob_from_grid(&run.installer, &grid);

    let out = try_tier1_cache(
        &blob,
        &run.installer,
        &run.atoms,
        &run.target,
        &run.excluded,
        &overrides,
        7,
    )
    .hit()
    .expect("Tier-1 reproduces -> bespoke JSON");

    assert_eq!(out.get("schema_version").and_then(JVal::as_i64), Some(2));
    let diag = out.get("diagnostics").expect("diagnostics");
    assert_eq!(
        diag.get("phase_reached").and_then(JVal::as_str),
        Some("tier1_cache")
    );
    assert_eq!(diag.get("exact_match").and_then(JVal::as_bool), Some(true));
    let cache = diag.get("cache").expect("cache");
    assert_eq!(cache.get("hit").and_then(JVal::as_bool), Some(true));
    assert_eq!(
        cache.get("source").and_then(JVal::as_str),
        Some("fomod-plus")
    );
    // total_ms is threaded through from the caller.
    assert_eq!(
        diag.get("timings_ms")
            .and_then(|t| t.get("total"))
            .and_then(JVal::as_i64),
        Some(7)
    );
    assert!(
        any_plugin_reason_code(&out, "FOMOD_PLUS_CACHE"),
        "{case}: a selected plugin must carry the FOMOD_PLUS_CACHE reason"
    );
    // outputTree is embedded from the validation simulation.
    assert!(out.get("outputTree").is_some_and(JVal::is_array));
}

#[test]
fn tier1_cache_reject_falls_through() {
    // Reuse a fixture that DOES reproduce with the correct selection, then feed a
    // blob that name-resolves but selects NOTHING - the empty install cannot
    // reproduce, so Tier-1 returns None (fall through to the normal solve).
    let (case, run, _grid, overrides) =
        first_tier1_reproducer().expect("a reproducing fixture to derive a reject from");

    // Empty every group's plugin selection.
    let empty_grid: Vec<Vec<Vec<bool>>> = run
        .installer
        .steps
        .iter()
        .map(|s| {
            s.groups
                .iter()
                .map(|g| vec![false; g.plugins.len()])
                .collect()
        })
        .collect();
    let blob = blob_from_grid(&run.installer, &empty_grid);

    let out = try_tier1_cache(
        &blob,
        &run.installer,
        &run.atoms,
        &run.target,
        &run.excluded,
        &overrides,
        0,
    );
    assert!(
        out.is_miss(),
        "{case}: an all-deselected cache must not reproduce -> Miss"
    );
}

/// A blob naming a step absent from the installer is stale -> None (fall through).
#[test]
fn tier1_cache_stale_step_name_falls_through() {
    let run = run_case_hashed("zip_exactlyone_racecompat");
    let overrides = compute_overrides(
        &run.installer,
        &run.atoms,
        &run.index,
        &run.target,
        &run.excluded,
    );
    let mut steps = JVal::array();
    let mut step = JVal::object();
    step.insert("name", JVal::string("NoSuchStepName"));
    step.insert("groups", JVal::array());
    steps.push(step);
    let mut blob = JVal::object();
    blob.insert("steps", steps);

    assert!(
        try_tier1_cache(
            &blob,
            &run.installer,
            &run.atoms,
            &run.target,
            &run.excluded,
            &overrides,
            0,
        )
        .is_miss()
    );
}
