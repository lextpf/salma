//! Byte-parity fixture tests for the schema-v2 inference output (Task 10).
//!
//! The golden `expected.json` bytes ARE the C++ DLL's
//! `assemble_json -> add_output_tree -> dump(2)` output, so this suite replays
//! the same chain in Rust and compares byte for byte.
//!
//! ## Two injected values (both come from stages this task does NOT own)
//!
//! - `timings_ms` is wall-clock (Task 12 orchestration) and non-reproducible;
//!   the four integers are parsed from `expected.json` and fed to
//!   `set_run_timings`, exactly as the task spec prescribes.
//! - The per-step visibility reason is produced by `compute_overrides` (Task 12
//!   orchestration, not ported here). Each step's visibility reason CODE and the
//!   `visible` flag are read from `expected.json` and fed to
//!   `set_step_visibility`; the Rust builder then produces the reason MESSAGE and
//!   ordering, so the serializer/message mapping is still validated end to end.
//!   Only the compute_overrides OUTPUT (a 3-valued code per step) is borrowed.
//!
//! ## The outputTree size discrepancy (documented fixture-data issue)
//!
//! `outputTree[].size` is `atom.file_size`, which the golden inference run took
//! from a LIVE archive listing that returned 0 for many entries (Task 6
//! PARITY-NOTES). The committed `archive_entries.json` snapshots carry populated
//! sizes, so a Rust run over the fixture atoms produces different `size` values
//! wherever the golden listing had 0. This is NOT a serializer bug, so:
//!
//! - [`full_byte_parity_over_consistent_fixtures`] asserts the ENTIRE document
//!   byte-matches after collapsing every `outputTree` `"size"` value (the only
//!   bare `"size"` key in the schema) - proving confidence, reasons,
//!   resolved_by, timings, cache, step visibility, key ordering, indentation,
//!   escaping, and the float format all match exactly. It also counts the
//!   fixtures whose UNnormalized document is byte-identical (sizes and all) and
//!   pins that count.
//! - [`skeleton_and_output_tree_over_all_fixtures`] asserts, for every fixture,
//!   that the step/group/plugin skeleton (names + selected/deselected split),
//!   `schema_version`, and the `outputTree` path/source pairs (sizes excluded)
//!   byte-match.

mod common;

use std::collections::HashSet;

use mo2_salma_rs::fomod_csp_solver::solve_fomod_csp;
use mo2_salma_rs::fomod_csp_types::{InferenceOverrides, ReproMetrics, SolverResult};
use mo2_salma_rs::fomod_forward_simulator::{compare_trees, simulate};
use mo2_salma_rs::fomod_inference_atoms::{add_output_tree, assemble_json};
use mo2_salma_rs::fomod_ir::FomodInstaller;
use mo2_salma_rs::fomod_propagator::{PropagationResult, propagate};
use mo2_salma_rs::inference_diagnostics::{InferenceDiagnosticsBuilder, ReasonCode};

use common::minijson::Value;

/// Consistent fixtures whose full document (INCLUDING both documented C++
/// nondeterministic regions) is byte-identical to `expected.json`. Only
/// `rar_exactlyone_heel_volume` qualifies: it has no `outputTree` size-0
/// discrepancy AND no multi-hit `UNIQUE_FILE_EVIDENCE` example list, so nothing
/// nondeterministic appears. Every other consistent fixture matches
/// byte-for-byte OUTSIDE those two regions (proven by
/// [`full_byte_parity_over_consistent_fixtures`]'s normalized assertion) but
/// carries at least one size-0 atom or a `std::unordered_set`-ordered `files`
/// example array. Pinned so drift in either direction fails the test.
const BYTE_EXACT_COUNT: usize = 1;

/// Number of archive-consistent fixtures (expected grid metrics == recorded
/// `diagnostics.repro`). Mirror of the pin in `fomod_csp_solver_fixtures.rs`.
const CONSISTENT_COUNT: usize = 12;

/// The single inconsistent fixture whose 100-plugin solve does not short-circuit
/// within a test-time budget; excluded from any solver-driving path.
const NON_TERMINATING: &str = "zip_11step_cbbe_3ba";

// --- consistent/inconsistent partition (mirror of the CSP fixtures) --------

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

/// A fixture is archive-consistent when its expected grid, simulated with the
/// fixture atoms, reproduces `diagnostics.repro` exactly.
fn is_consistent(case: &str) -> bool {
    let run = common::run_case(case);
    let expected = common::load_expected(case);
    let grid = common::build_selection_grid(&run.installer, &expected, case);
    grid_metrics(&run, &grid) == expected_repro(&expected)
}

fn consistent_cases() -> Vec<String> {
    common::committed_cases()
        .into_iter()
        .filter(|c| is_consistent(c))
        .collect()
}

// --- fixture-derived injection (timings + step visibility) -----------------

/// Feed the four `timings_ms` integers recorded in `expected.json`.
fn inject_timings(builder: &mut InferenceDiagnosticsBuilder, expected: &Value) {
    let t = expected
        .member("diagnostics")
        .expect("diagnostics")
        .member("timings_ms")
        .expect("timings_ms");
    let get = |k: &str| t.member(k).expect(k).as_u64() as i64;
    builder.set_run_timings(get("list"), get("scan"), get("solve"), get("total"));
}

fn visibility_code(name: &str) -> ReasonCode {
    match name {
        "STEP_VISIBILITY_FORCED" => ReasonCode::StepVisibilityForced,
        "STEP_VISIBILITY_UNKNOWN" => ReasonCode::StepVisibilityUnknown,
        "STEP_NOT_VISIBLE" => ReasonCode::StepNotVisible,
        other => panic!("unexpected step visibility code {other:?}"),
    }
}

/// Feed each step's visibility decision from `expected.json` (the code recorded
/// in the single step-level reason, plus the `visible` flag). The
/// `compute_overrides` stage that produces these is Task 12; only its per-step
/// output is borrowed here.
fn inject_step_visibility(builder: &mut InferenceDiagnosticsBuilder, expected: &Value) {
    let steps = expected.member("steps").expect("steps").as_array();
    for (si, step) in steps.iter().enumerate() {
        let reasons = step.member("reasons").expect("reasons").as_array();
        let Some(first) = reasons.first() else {
            continue;
        };
        let code = visibility_code(first.member("code").expect("code").as_str());
        let visible = step.member("visible").expect("visible").as_bool();
        builder.set_step_visibility(si as i32, visible, code);
    }
}

/// Replay the full C++ diagnostics chain for a solved fixture and return the
/// `dump(2)` string. `grid`/`res` supply the selection state; timings and step
/// visibility are injected from `expected`.
fn build_document(
    run: &common::CaseRun,
    res: &SolverResult,
    expected: &Value,
    installer: &FomodInstaller,
) -> String {
    let prop = propagate_case(run);
    let mut builder = InferenceDiagnosticsBuilder::new(installer);
    inject_step_visibility(&mut builder, expected);
    builder.absorb_propagation(&prop);
    builder.absorb_solver(res);
    inject_timings(&mut builder, expected);
    builder.set_target_file_count(run.target.len() as i32);
    builder.finalize(res, &prop, installer);

    let mut out = assemble_json(installer, res, builder.diagnostics());
    let overrides = InferenceOverrides::default();
    let sim = simulate(
        installer,
        &run.atoms,
        &res.selections,
        None,
        Some(&overrides),
    );
    add_output_tree(&mut out, &sim);
    out.dump(2)
}

/// Collapse the two documented C++ nondeterministic regions so the rest of the
/// document can be byte-compared:
///
/// 1. `outputTree[].size` (bare `"size"`, the only such key in the schema): the
///    golden `atom.file_size` came from a live listing that returned 0 for many
///    entries, while the fixture snapshots carry populated sizes (Task 6
///    PARITY-NOTES).
/// 2. `UNIQUE_FILE_EVIDENCE` `files` example arrays: the C++ builds these by
///    iterating a `std::unordered_set` (MSVC hash order), so both the ORDER and,
///    when `count > 4`, the chosen 4-of-N SUBSET are unreproducible. The Rust
///    propagator sorts for determinism.
///
/// Everything else (confidence, reasons, codes, messages, `count`, resolved_by,
/// timings, cache, `nodes`, structure, key order, indentation, float format) is
/// left intact for an exact byte comparison.
fn normalize_nondeterministic(doc: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut lines = doc.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        if let Some(rest) = trimmed.strip_prefix("\"size\": ") {
            let comma = if rest.ends_with(',') { "," } else { "" };
            out.push(format!("{indent}\"size\": N{comma}"));
        } else if trimmed == "\"files\": [" {
            // Consume the array body up to the closing bracket at this indent.
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

/// First differing line index + the two lines, for a readable failure message.
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

// --- primary: full byte parity over the consistent set ---------------------

#[test]
fn full_byte_parity_over_consistent_fixtures() {
    let cases = consistent_cases();
    assert_eq!(
        cases.len(),
        CONSISTENT_COUNT,
        "archive-consistent fixture count (consistent: {cases:?})"
    );

    let mut byte_exact: Vec<String> = Vec::new();
    for case in &cases {
        let run = common::run_case(case);
        let expected_text =
            std::fs::read_to_string(common::golden_cases_dir().join(case).join("expected.json"))
                .expect("expected.json readable");
        let expected = common::load_expected(case);
        let res = solve_case(&run);
        let mine = build_document(&run, &res, &expected, &run.installer);

        // STRONG: the whole document matches once outputTree sizes are collapsed.
        // Any diff here is a real serializer/confidence/accumulation bug.
        let mine_n = normalize_nondeterministic(&mine);
        let expected_n = normalize_nondeterministic(&expected_text);
        assert!(
            mine_n == expected_n,
            "{case}: normalized document differs (serializer/confidence bug)\n{}",
            first_diff(&mine_n, &expected_n).unwrap_or_default()
        );

        if mine == expected_text {
            byte_exact.push(case.clone());
        }
    }

    byte_exact.sort();
    assert_eq!(
        byte_exact.len(),
        BYTE_EXACT_COUNT,
        "fully byte-exact fixtures (incl. outputTree sizes): {byte_exact:?}"
    );
}

// --- secondary: skeleton + outputTree structure over ALL fixtures ----------

/// Per-group `(selected names, deselected names)` split.
type GroupSplit = (Vec<String>, Vec<String>);
/// Size-independent projection: `(schema_version, per-step group splits,
/// outputTree (path, source) pairs)`.
type Skeleton = (i64, Vec<Vec<GroupSplit>>, Vec<(String, String)>);

/// A minimal, size-independent projection of a document: schema_version, the
/// per-step/group selected+deselected plugin-name split, and the outputTree
/// (path, source) pairs. Byte-independent of confidence floats and sizes.
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

#[test]
fn skeleton_and_output_tree_over_all_fixtures() {
    for case in common::committed_cases() {
        let run = common::run_case(&case);
        let expected = common::load_expected(&case);

        // Drive assemble_json from the KNOWN expected grid so this covers the
        // inconsistent set too (whose solver grid may differ on the size split)
        // and the non-terminating fixture (never solved here).
        let grid = common::build_selection_grid(&run.installer, &expected, &case);
        let res = SolverResult {
            selections: grid,
            ..SolverResult::default()
        };

        let mut builder = InferenceDiagnosticsBuilder::new(&run.installer);
        inject_step_visibility(&mut builder, &expected);
        let prop = propagate_case(&run);
        builder.absorb_propagation(&prop);
        builder.absorb_solver(&res);
        builder.finalize(&res, &prop, &run.installer);

        let mut out = assemble_json(&run.installer, &res, builder.diagnostics());
        let overrides = InferenceOverrides::default();
        let sim = simulate(
            &run.installer,
            &run.atoms,
            &res.selections,
            None,
            Some(&overrides),
        );
        add_output_tree(&mut out, &sim);

        // Parse my dump back through the same reader the expected uses, then
        // compare the size-independent projections.
        let mine_text = out.dump(2);
        let mine = common::minijson::parse(&mine_text);
        assert_eq!(
            skeleton(&mine),
            skeleton(&expected),
            "{case}: skeleton (names + selected split) / schema_version / outputTree path+source"
        );
    }
}

/// Walk a document in `(step, group, plugins-then-deselected, reason)` order and
/// collect every `UNIQUE_FILE_EVIDENCE` detail's `(count, files)`. Used to check
/// the propagator's file CONTENT (normalized away in the byte comparison).
fn unique_evidence_details(doc: &Value) -> Vec<(i64, Vec<String>)> {
    let mut out = Vec::new();
    for step in doc.member("steps").expect("steps").as_array() {
        for group in step.member("groups").expect("groups").as_array() {
            for key in ["plugins", "deselected"] {
                let Some(arr) = group.member(key) else {
                    continue;
                };
                for plugin in arr.as_array() {
                    let Some(reasons) = plugin.member("reasons") else {
                        continue;
                    };
                    for r in reasons.as_array() {
                        if r.member("code").map(|c| c.as_str()) != Some("UNIQUE_FILE_EVIDENCE") {
                            continue;
                        }
                        let detail = r.member("detail").expect("detail");
                        let count = detail.member("count").expect("count").as_u64() as i64;
                        let files = detail
                            .member("files")
                            .expect("files")
                            .as_array()
                            .iter()
                            .map(|f| f.as_str().to_string())
                            .collect();
                        out.push((count, files));
                    }
                }
            }
        }
    }
    out
}

/// The `UNIQUE_FILE_EVIDENCE` `files` example array is emitted from a
/// `std::unordered_set` in the C++ (MSVC hash order) and sorted in the Rust
/// propagator, so the byte comparison normalizes it. This test recovers the lost
/// coverage: the `count` (deterministic) must match at every position, and when
/// `count <= 4` (the full hit set fits, so the choice of examples is not
/// truncation-dependent) the file SET must match too. `count > 4` details are a
/// 4-of-N subset that both sides pick nondeterministically, so only the count is
/// checked there.
#[test]
fn unique_file_evidence_content_matches_over_consistent_fixtures() {
    for case in consistent_cases() {
        let run = common::run_case(&case);
        let expected = common::load_expected(&case);
        let res = solve_case(&run);
        let mine_text = build_document(&run, &res, &expected, &run.installer);
        let mine = common::minijson::parse(&mine_text);

        let mine_ev = unique_evidence_details(&mine);
        let exp_ev = unique_evidence_details(&expected);
        assert_eq!(
            mine_ev.len(),
            exp_ev.len(),
            "{case}: UNIQUE_FILE_EVIDENCE detail count"
        );
        for (i, ((mc, mf), (ec, ef))) in mine_ev.iter().zip(exp_ev.iter()).enumerate() {
            assert_eq!(mc, ec, "{case}: detail {i} count");
            if *ec <= 4 {
                let mut ms = mf.clone();
                ms.sort();
                let mut es = ef.clone();
                es.sort();
                assert_eq!(ms, es, "{case}: detail {i} file set (count <= 4)");
            }
        }
    }
}

/// The `NON_TERMINATING` fixture is inconsistent and must never enter the
/// consistent (solver-driven) set.
#[test]
fn non_terminating_fixture_is_not_consistent() {
    let consistent: HashSet<String> = consistent_cases().into_iter().collect();
    assert!(
        !consistent.contains(NON_TERMINATING),
        "{NON_TERMINATING} must be excluded from the solver-driven set"
    );
}
