//! Corpus byte-parity test for `ArchiveService::list_entries_with_sizes`
//! (Task 11).
//!
//! This is the primary oracle for the archive layer: for each committed golden
//! case it opens the REAL source archive (recorded in `case.json` as
//! `source_archive_path`) and asserts the produced `[{path, size}]` listing
//! equals the committed `archive_entries.json` exactly - same order, same path
//! bytes, same sizes - across all three formats (zip / 7z / rar).
//!
//! The real archives are NOT committed (they are large third-party mod files
//! living at machine-specific paths). The test is therefore GATED ON
//! AVAILABILITY: a case whose archive is absent is skipped with a printed note
//! so the suite stays green on CI, where none of the archives exist. On the
//! development machine all 16 archives are present and must all match. The test
//! counts how many cases actually ran and, when any did, asserts every one of
//! them matched (never a subset or set-compare - the ordered, byte-exact list).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use mo2_salma_rs::archive_service::ArchiveService;
use mo2_salma_rs::utils::{normalize_path, random_hex_string};

mod common;
use common::minijson;

/// The C++ `EntryListing` serialization contract: pair each `paths` entry
/// (original casing, listing order) with `sizes[normalize_path(path)]`.
fn produced_pairs(archive_path: &str) -> Vec<(String, u64)> {
    let svc = ArchiveService::new();
    let listing = svc.list_entries_with_sizes(archive_path);
    listing
        .paths
        .iter()
        .map(|p| {
            let size = *listing
                .sizes
                .get(&normalize_path(p))
                .unwrap_or_else(|| panic!("no size for listed path {p:?}"));
            (p.clone(), size)
        })
        .collect()
}

/// Parse a committed `archive_entries.json` into an ordered (path, size) list.
fn golden_pairs(case_dir: &Path) -> Vec<(String, u64)> {
    let text = fs::read_to_string(case_dir.join("archive_entries.json"))
        .expect("archive_entries.json readable");
    minijson::parse(&text)
        .as_array()
        .iter()
        .map(|e| {
            (
                e.member("path").expect("path").as_str().to_string(),
                e.member("size").expect("size").as_u64(),
            )
        })
        .collect()
}

/// Read `source_archive_path` from a case's `case.json` (backslash-unescaped by
/// the JSON string parser).
fn source_archive_path(case_dir: &Path) -> String {
    let text = fs::read_to_string(case_dir.join("case.json")).expect("case.json readable");
    minijson::parse(&text)
        .member("source_archive_path")
        .expect("source_archive_path")
        .as_str()
        .to_string()
}

/// Read the archive's stored `fomod/ModuleConfig.xml` entry path (original
/// casing / separators) from a case's `case.json`.
fn module_config_entry(case_dir: &Path) -> String {
    let text = fs::read_to_string(case_dir.join("case.json")).expect("case.json readable");
    minijson::parse(&text)
        .member("module_config_entry")
        .expect("module_config_entry")
        .as_str()
        .to_string()
}

/// Read (archive_format, archive_size) from a case's `case.json`.
fn archive_format_and_size(case_dir: &Path) -> (String, u64) {
    let text = fs::read_to_string(case_dir.join("case.json")).expect("case.json readable");
    let v = minijson::parse(&text);
    (
        v.member("archive_format")
            .expect("archive_format")
            .as_str()
            .to_string(),
        v.member("archive_size").expect("archive_size").as_u64(),
    )
}

/// A throwaway scratch directory under the OS temp root.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "salma_rs_archive_corpus_{}_{}",
        tag,
        random_hex_string(12)
    ));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Recursively collect regular-file paths under `root`.
fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

/// Corpus read/extract parity for the 7z / rar / zip backends (Task 11 review
/// finding 2: the only read/extract unit tests use in-memory ZIP, so the whole
/// solid-block read strategy shipped unverified against real data).
///
/// For every committed case that ships a `ModuleConfig.xml` AND whose real
/// source archive is present, this reads `fomod/ModuleConfig.xml` back three
/// ways and byte-compares each to the committed golden bytes:
///
/// - [`ArchiveService::read_entry`] - the exact call Task 12 makes to load the
///   config. On a solid 7z the config is preceded in its block by `info.xml`, so
///   this exercises the solid-block skip-drain path that regressed (the pre-fix
///   symptom was 0 bytes returned here).
/// - [`ArchiveService::read_entries_batch`] requesting ONLY the config (any
///   preceding non-requested entry in the block must be drained to stay aligned;
///   the pre-fix symptom was the entry absent from the result map).
/// - [`ArchiveService::extract_prefix`] with the config's full path as the
///   prefix, extracting exactly that one entry while skipping every other entry
///   in its block (pre-fix symptom: `ChecksumVerificationFailed`). Skipped for
///   solid 7z archives above 50 MiB, where draining the remainder of the block
///   would decode hundreds of MiB for a single-file check; the read paths above
///   still cover those cases (they stop at the target).
#[test]
fn read_and_extract_match_committed_module_config_over_real_corpus() {
    let cases_dir = common::golden_cases_dir();
    let mut ran = 0usize;
    let mut skipped = 0usize;
    let mut extract_ran = 0usize;
    let mut failures: Vec<String> = Vec::new();

    let mut case_names: Vec<String> = fs::read_dir(&cases_dir)
        .expect("golden cases dir")
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.join("case.json").exists() && p.join("ModuleConfig.xml").exists())
                .then(|| p.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    case_names.sort();

    let svc = ArchiveService::new();
    for case in &case_names {
        let case_dir = cases_dir.join(case);
        let archive = source_archive_path(&case_dir);
        if !Path::new(&archive).exists() {
            eprintln!("[skip] {case}: source archive not present ({archive})");
            skipped += 1;
            continue;
        }
        ran += 1;

        let entry = module_config_entry(&case_dir);
        let norm = normalize_path(&entry);
        let golden =
            fs::read(case_dir.join("ModuleConfig.xml")).expect("committed ModuleConfig.xml");

        // read_entry
        let via_read_entry = svc.read_entry(&archive, &entry);
        if via_read_entry != golden {
            failures.push(format!(
                "{case}: read_entry({entry:?}) = {} bytes, golden = {} bytes",
                via_read_entry.len(),
                golden.len()
            ));
        }

        // read_entries_batch requesting only the module config
        let want: HashSet<String> = std::iter::once(norm.clone()).collect();
        let batch = svc.read_entries_batch(&archive, &want);
        match batch.get(&norm) {
            Some(bytes) if *bytes == golden => {}
            Some(bytes) => failures.push(format!(
                "{case}: read_entries_batch returned {} bytes, golden {} bytes",
                bytes.len(),
                golden.len()
            )),
            None => failures.push(format!(
                "{case}: read_entries_batch did not return {norm:?} (got {} of 1)",
                batch.len()
            )),
        }

        // extract_prefix round-trip (size-gated for solid 7z, see the doc note).
        let (fmt, size) = archive_format_and_size(&case_dir);
        if fmt == "7z" && size > 50 * 1024 * 1024 {
            continue;
        }
        extract_ran += 1;
        let out = scratch_dir(case);
        match svc.extract_prefix(&archive, out.to_str().unwrap(), &norm) {
            Ok(()) => {
                let files = walk_files(&out);
                if files.len() != 1 {
                    failures.push(format!(
                        "{case}: extract_prefix wrote {} files, expected exactly 1: {files:?}",
                        files.len()
                    ));
                } else {
                    let got = fs::read(&files[0]).expect("read extracted file");
                    if got != golden {
                        failures.push(format!(
                            "{case}: extract_prefix file = {} bytes, golden = {} bytes",
                            got.len(),
                            golden.len()
                        ));
                    }
                }
            }
            Err(e) => failures.push(format!("{case}: extract_prefix errored: {e}")),
        }
        fs::remove_dir_all(&out).ok();
    }

    eprintln!(
        "[corpus] archive read/extract parity: {ran} ran ({extract_ran} with extract_prefix), \
         {skipped} skipped"
    );
    assert!(
        failures.is_empty(),
        "archive read/extract failures:\n{}",
        failures.join("\n")
    );
    common::note_corpus_coverage(ran, "archive read/extract parity");
}

#[test]
fn list_entries_with_sizes_matches_golden_over_real_corpus() {
    let cases_dir = common::golden_cases_dir();
    let mut ran = 0usize;
    let mut skipped = 0usize;
    let mut mismatches: Vec<String> = Vec::new();

    let mut case_names: Vec<String> = fs::read_dir(&cases_dir)
        .expect("golden cases dir")
        .filter_map(|e| {
            let p = e.unwrap().path();
            p.join("case.json")
                .exists()
                .then(|| p.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    case_names.sort();

    for case in &case_names {
        let case_dir = cases_dir.join(case);
        let archive = source_archive_path(&case_dir);
        if !Path::new(&archive).exists() {
            eprintln!("[skip] {case}: source archive not present ({archive})");
            skipped += 1;
            continue;
        }

        ran += 1;
        let produced = produced_pairs(&archive);
        let golden = golden_pairs(&case_dir);

        if produced != golden {
            // Pinpoint the first divergence for a legible failure.
            let mut detail = format!(
                "{case}: listing mismatch (produced {} entries, golden {} entries)",
                produced.len(),
                golden.len()
            );
            let n = produced.len().min(golden.len());
            for i in 0..n {
                if produced[i] != golden[i] {
                    detail.push_str(&format!(
                        "\n  first diff at index {i}:\n    produced = {:?}\n    golden   = {:?}",
                        produced[i], golden[i]
                    ));
                    break;
                }
            }
            mismatches.push(detail);
        }
    }

    eprintln!("[corpus] archive byte-parity: {ran} ran, {skipped} skipped");
    assert!(
        mismatches.is_empty(),
        "archive listing byte-parity failures:\n{}",
        mismatches.join("\n")
    );
    // When any archive was accessible, every one of them must have matched
    // (mismatches is already asserted empty). If nothing ran (pure CI with no
    // corpus), the test is a documented no-op skip unless SALMA_REQUIRE_CORPUS
    // says this host should have had one.
    common::note_corpus_coverage(ran, "archive listing byte-parity");
}
