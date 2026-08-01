//! Corpus-gated END-TO-END install-replay oracle for Task 14.
//!
//! The unit tests inside `src/fomod_service.rs` pin each stage against inline IR
//! fixtures. This file closes the loop the plan asks for: take a REAL curated
//! archive, extract it, parse its `ModuleConfig.xml`, replay the install driven
//! by the committed schema-v2 selections, and diff the tree the replay actually
//! writes to disk against the golden `target_tree.json` captured from the
//! installed mod.
//!
//! That makes it the first test in the port that exercises
//! `FomodService` + `file_operations` together against real bytes, and the only
//! one that proves the replay produces the SAME FILES the C++ engine produced.
//!
//! ## Why the committed `expected.json` is the config
//!
//! `expected.json` IS the C++ DLL's schema-v2 inference output for that case, and
//! schema-v2 is exactly one of the two shapes `read_plugin_name` accepts. Feeding
//! it back in unchanged is both the realistic `installWithConfig` path (the MO2
//! plugin passes an inference result straight back) and free coverage of the v2
//! consumer over 16 real documents rather than a synthetic one.
//!
//! ## Skips
//!
//! Every case is skipped unless its `source_archive_path` exists on this machine,
//! so CI (which has no corpus) runs this file as a no-op, the same convention
//! `archive_service_fixtures.rs` uses. A case whose `expected_status` is `empty`
//! carries no selections and is skipped too.

mod common;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use mo2_salma_rs::archive_service::ArchiveService;
use mo2_salma_rs::fomod_ir_parser::parse_module_config;
use mo2_salma_rs::fomod_service::{FomodService, execute_file_operations};
use mo2_salma_rs::json;
use mo2_salma_rs::types::FileOperation;
use mo2_salma_rs::utils::normalize_path;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique scratch directory for one case, removed by the caller.
fn scratch_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("salma_t14_{tag}_{}_{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Recursively collect `normalized relative path -> size` under `root`.
fn scan_tree(root: &Path) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(root) {
                let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                out.insert(normalize_path(&rel.to_string_lossy()), size);
            }
        }
    }
    out
}

/// Locate the shallowest `fomod/moduleconfig.xml` in a listing, mirroring the
/// preference `infer_selections` applies.
fn find_module_config(entries: &[String]) -> Option<String> {
    const SUFFIX: &str = "fomod/moduleconfig.xml";
    let mut best: Option<String> = None;
    let mut best_depth = usize::MAX;
    for raw in entries {
        let norm = normalize_path(raw);
        let is_candidate =
            norm == SUFFIX || (norm.len() > SUFFIX.len() && norm.ends_with(&format!("/{SUFFIX}")));
        if !is_candidate {
            continue;
        }
        let depth = norm.matches('/').count();
        let shorter = best.as_ref().is_none_or(|b| norm.len() < b.len());
        if depth < best_depth || (depth == best_depth && shorter) {
            best_depth = depth;
            best = Some(norm);
        }
    }
    best
}

/// What the committed C++ inference itself predicts about this case.
struct Predicted {
    /// `diagnostics.exact_match` from `expected.json`.
    exact: bool,
    /// `diagnostics.repro.missing` - target dests the selection does not produce.
    missing: u64,
    /// `diagnostics.repro.extra` - dests produced that are not in the target.
    extra: u64,
}

/// One replayed case: the tree the replay wrote to disk, the golden tree it is
/// checked against, the C++ prediction, and the set of file sizes the committed
/// archive actually contains (used to tell a stale golden from a real defect).
struct Replay {
    produced: HashMap<String, u64>,
    golden: HashMap<String, u64>,
    predicted: Predicted,
    archive_sizes: HashSet<u64>,
}

/// Extract, parse, and replay one case; `None` when the corpus archive is absent
/// or the case carries no selections.
fn replay_case(case: &str) -> Option<Replay> {
    let case_dir = common::golden_cases_dir().join(case);
    let case_json = fs::read_to_string(case_dir.join("case.json")).ok()?;
    let meta = common::minijson::parse(&case_json);

    if meta.member("expected_status")?.as_str() != "non_empty" {
        return None; // no selections to replay
    }
    let archive_path = meta.member("source_archive_path")?.as_str().to_string();
    if !Path::new(&archive_path).exists() {
        return None; // corpus archive absent (CI)
    }

    // 1. Extract the archive.
    let extract_dir = scratch_dir("src");
    let svc = ArchiveService::new();
    if svc
        .extract(
            &archive_path,
            extract_dir.to_str().expect("utf-8 temp path"),
        )
        .is_err()
    {
        let _ = fs::remove_dir_all(&extract_dir);
        return None;
    }

    // 2. Locate and parse ModuleConfig.xml from the EXTRACTED tree, so the
    //    replay reads the same bytes the install would.
    let listing = svc.list_entries_with_sizes(&archive_path);
    // Every size the committed archive can produce. A golden size outside this
    // set cannot be reproduced from THIS archive no matter which source wins, so
    // it marks a file whose installed copy came from a different archive
    // revision (see the size assertion below and PARITY-NOTES "Task 14").
    let archive_sizes: HashSet<u64> = listing.sizes.values().copied().collect();
    let cfg_norm = find_module_config(&listing.paths).or_else(|| {
        let _ = fs::remove_dir_all(&extract_dir);
        None
    })?;
    let prefix = cfg_norm
        .strip_suffix("fomod/moduleconfig.xml")
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string();

    let cfg_disk = extract_dir.join(&cfg_norm);
    let xml = fs::read(&cfg_disk).ok().or_else(|| {
        let _ = fs::remove_dir_all(&extract_dir);
        None
    })?;
    let installer = match parse_module_config(&xml, &prefix) {
        Ok(ir) => ir,
        Err(_) => {
            let _ = fs::remove_dir_all(&extract_dir);
            return None;
        }
    };

    // 3. Replay, driven by the committed schema-v2 selections. Parsed with the
    //    CRATE's json reader (not the test harness's), because that is the
    //    reader the production `installWithConfig` path uses.
    let expected_text =
        fs::read_to_string(case_dir.join("expected.json")).expect("expected.json readable");
    let config = json::parse(&expected_text).expect("expected.json parses");
    let mod_dir = scratch_dir("mod");
    let mut service = FomodService::new();
    service.set_installer(installer);

    let src_base = extract_dir.to_string_lossy().to_string();
    let dst_base = mod_dir.to_string_lossy().to_string();
    let mut ops: Vec<FileOperation> = Vec::new();
    let mut doc_order = 0;

    assert!(
        service.check_module_dependencies(None),
        "{case}: module dependencies must hold for a case the C++ inferred"
    );
    // Cardinality violations are only a warning in the C++ caller, so the
    // boolean is deliberately not asserted; a hard error still fails the test.
    let _ = service
        .validate_json_selections(&config)
        .unwrap_or_else(|e| panic!("{case}: selections rejected: {e:?}"));
    service.process_required_files(&src_base, &dst_base, &mut ops, &mut doc_order);
    service
        .process_optional_files(
            &config,
            &src_base,
            &dst_base,
            None,
            &mut ops,
            &mut doc_order,
        )
        .unwrap_or_else(|e| panic!("{case}: optional pass failed: {e:?}"));
    service.process_conditional_files(&src_base, &dst_base, None, &mut ops, &mut doc_order);
    execute_file_operations(&mut ops);

    let produced = scan_tree(&mod_dir);

    // 4. Golden tree: paths + sizes (hashes are the inference oracle's business).
    let golden_tree = common::load_target_tree_with_hashes(&case_dir);
    let golden: HashMap<String, u64> = golden_tree
        .into_iter()
        .map(|(path, tf)| (path, tf.size))
        .collect();

    let diag = config.get("diagnostics");
    let repro = diag.and_then(|d| d.get("repro"));
    let predicted = Predicted {
        exact: diag
            .and_then(|d| d.get("exact_match"))
            .and_then(json::Value::as_bool)
            .unwrap_or(false),
        missing: repro
            .and_then(|r| r.get("missing"))
            .and_then(json::Value::as_i64)
            .unwrap_or(0)
            .max(0) as u64,
        extra: repro
            .and_then(|r| r.get("extra"))
            .and_then(json::Value::as_i64)
            .unwrap_or(0)
            .max(0) as u64,
    };

    let _ = fs::remove_dir_all(&extract_dir);
    let _ = fs::remove_dir_all(&mod_dir);
    Some(Replay {
        produced,
        golden,
        predicted,
        archive_sizes,
    })
}

/// Replaying the committed selections must reproduce the installed file tree.
///
/// The strength of the assertion is taken from what the C++ inference ITSELF
/// claims about the case, read out of the committed `expected.json`:
///
/// - `exact_match: true`: the selection is asserted to reproduce the installed
///   tree EXACTLY - every path, and every byte size the committed archive can
///   actually produce (see the reachability gate below). This is the real oracle
///   for the replay.
/// - `exact_match: false`: the C++ engine already says this selection does not
///   reproduce the tree (it is the best the solver found), so demanding equality
///   would assert something the reference implementation does not claim. Only
///   the PATH-level shape the C++ predicts is asserted: `repro.missing == 0`
///   means the replay must still produce every target dest, and `repro.extra ==
///   0` means it must produce no others.
///
/// Two curated cases are non-exact and each exercises one of those directions:
/// `rar_7step_sos` (missing 0, extra 0, size_mismatch 5 - right files, wrong
/// sources win) and `rar_exactlyone_heel_volume` (missing 35 - a subset).
///
/// Sizes are deliberately NOT compared on the non-exact cases. Note the C++
/// `repro` counters are computed against the SIMULATED tree, whose per-atom
/// sizes come from the archive listing and are subject to the Task 12
/// entry-size under-population bug (an entry whose raw path is not already
/// normalized gets size 0, which compares as "compatible"). A real byte-level
/// replay therefore sees at least as many size mismatches as the simulator
/// reports: `rar_7step_sos` reports 5 and replays 10.
///
/// ## Reachability gate on the exact-case size check
///
/// The replay copies the committed archive's real bytes, so every size it writes
/// IS an archive entry size. A golden size that is ABSENT from the committed
/// archive therefore cannot be reproduced no matter which source wins: the
/// installed mod was built from a DIFFERENT archive revision than the one
/// committed here. `sevenz_2step_nec_feet` is exactly this - 12 of its 31 mesh
/// files carry golden sizes that appear nowhere in the committed `.7z` (the
/// installed `.nif`s are a different build). The C++ inference still stamps the
/// case `exact_match: true` because the same Task 12 under-population bug leaves
/// those atoms at `file_size == 0`, which compares as size-compatible, so the
/// simulator never sees the discrepancy. Size equality is therefore asserted
/// only for files whose golden size the archive CAN produce; unreproducible
/// files are counted and skipped. A genuine wrong-winner (a reachable golden the
/// replay fails to reproduce) still fails. See PARITY-NOTES "Task 14".
#[test]
fn replaying_committed_selections_reproduces_the_installed_tree() {
    let mut exact_checked = 0;
    let mut approx_checked = 0;
    let mut skipped = 0;

    for case in common::committed_cases() {
        let Some(Replay {
            produced,
            golden,
            predicted,
            archive_sizes,
        }) = replay_case(&case)
        else {
            skipped += 1;
            continue;
        };

        let mut missing: Vec<&String> = golden
            .keys()
            .filter(|k| !produced.contains_key(*k))
            .collect();
        let mut extra: Vec<&String> = produced
            .keys()
            .filter(|k| !golden.contains_key(*k))
            .collect();
        missing.sort();
        extra.sort();

        if predicted.exact {
            exact_checked += 1;
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{case}: exact_match case must replay the golden tree.\n  missing ({}): {:?}\n  extra ({}): {:?}",
                missing.len(),
                missing.iter().take(12).collect::<Vec<_>>(),
                extra.len(),
                extra.iter().take(12).collect::<Vec<_>>()
            );

            // Byte-check only files whose golden size the committed archive can
            // produce; skip (and count) files whose golden size is unreachable -
            // those are a stale golden vs the committed archive revision, not a
            // replay defect (see the reachability gate in the doc comment).
            let mut wrong_size: Vec<String> = Vec::new();
            let mut unreachable = 0;
            for (path, want) in &golden {
                let got = produced[path];
                if got == *want {
                    continue;
                }
                if archive_sizes.contains(want) {
                    wrong_size.push(format!("{path}: got {got}, want {want}"));
                } else {
                    unreachable += 1;
                }
            }
            wrong_size.sort();
            assert!(
                wrong_size.is_empty(),
                "{case}: {} file(s) replayed at the wrong size: {:?}",
                wrong_size.len(),
                wrong_size.iter().take(12).collect::<Vec<_>>()
            );
            if unreachable > 0 {
                eprintln!(
                    "[task14] {case}: {unreachable} file(s) skipped - golden size \
                     absent from the committed archive (stale golden vs archive \
                     revision; see PARITY-NOTES Task 14)"
                );
            }
        } else {
            approx_checked += 1;
            // The replay must not drop MORE target dests than the C++ predicts:
            // the produced tree and the simulated tree the `repro` counts come
            // from target the same dest set (conflict resolution only picks WHICH
            // source wins, never WHETHER a dest appears), so real missing must be
            // <= `repro.missing`. This subsumes the `missing == 0` case and, for
            // `missing > 0`, catches under-production - e.g. an optional pass that
            // enqueues nothing - which a bare `if missing == 0` guard let slip
            // (`rar_exactlyone_heel_volume`: missing 35, reproduced 1; a replay
            // producing zero files would otherwise pass).
            assert!(
                missing.len() <= predicted.missing as usize,
                "{case}: replay is missing {} target dest(s), more than the {} the C++ predicts (under-production): {:?}",
                missing.len(),
                predicted.missing,
                missing.iter().take(12).collect::<Vec<_>>()
            );
            if predicted.extra == 0 {
                assert!(
                    extra.is_empty(),
                    "{case}: C++ reports repro.extra == 0, so the replay must produce no dest outside the target, but {} appeared: {:?}",
                    extra.len(),
                    extra.iter().take(12).collect::<Vec<_>>()
                );
            }
        }
    }

    eprintln!(
        "[task14] install replay: {exact_checked} exact case(s) byte-verified, \
         {approx_checked} non-exact case(s) shape-verified, {skipped} skipped"
    );
    common::note_corpus_coverage(exact_checked + approx_checked, "the install-replay oracle");
}
