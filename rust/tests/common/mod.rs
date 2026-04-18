//! Shared fixture-loading harness for the golden-case integration tests.
//!
//! Hoisted from the Task 5 atom-expansion test (`fomod_atoms_fixtures.rs`) so
//! the Task 6 forward-simulator fixtures (`fomod_forward_simulator_fixtures.rs`)
//! drive the SAME caller input-prep the engine uses
//! (`FomodInferenceService.cpp` steps 1-2): build `sorted_norm_entries` /
//! `norm_entry_sizes` from the archive listing, derive the fomod prefix from
//! the shallowest `fomod/ModuleConfig.xml` entry, parse the XML, then run
//! `expand_all_atoms` -> `build_atom_index` -> `compute_excluded_dests` ->
//! `build_target_tree`. See PARITY-NOTES "Task 5"/"Task 6" for the derivation.
//!
//! `#![allow(dead_code)]`: cargo compiles this module separately into each
//! `tests/*.rs` crate, and no single test binary exercises every helper.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use mo2_salma_rs::fomod_atom::{AtomIndex, ExpandedAtoms, TargetFile, TargetTree};
use mo2_salma_rs::fomod_inference_atoms::{
    build_atom_index, build_target_tree, compute_excluded_dests, expand_all_atoms,
};
use mo2_salma_rs::fomod_ir::FomodInstaller;
use mo2_salma_rs::fomod_ir_parser::parse_module_config;
use mo2_salma_rs::utils::normalize_path;

/// Absolute path to the committed golden-case corpus. `CARGO_MANIFEST_DIR` is
/// `rust/`, so the corpus sits beside this harness under `tests/`.
pub fn golden_cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/cases")
}

/// Whether this host is expected to hold the full corpus of source archives.
///
/// The committed fixtures reference their archives by a machine-local ABSOLUTE
/// `source_archive_path`, and no archive is tracked in git, so on CI every
/// corpus-gated case skips. Those tests then reach a vacuous
/// `assert!(failures.is_empty())` over an empty vector and report ok, which is
/// indistinguishable from having actually verified something.
///
/// Set `SALMA_REQUIRE_CORPUS=1` on any host that IS supposed to have the corpus
/// (a developer box, or a CI job that provisions it) to turn a zero-case run
/// into a hard failure. Unset, the skip stays a documented no-op.
pub fn require_corpus() -> bool {
    std::env::var_os("SALMA_REQUIRE_CORPUS").is_some_and(|v| !v.is_empty())
}

/// Record how many cases a corpus-gated test actually exercised, failing when
/// nothing ran and [`require_corpus`] says something should have. `what` names
/// the gate so the message points at the right oracle.
pub fn note_corpus_coverage(ran: usize, what: &str) {
    if ran > 0 {
        return;
    }
    assert!(
        !require_corpus(),
        "SALMA_REQUIRE_CORPUS is set but {what} exercised 0 cases: the source \
         archives are absent, or the source_archive_path entries in \
         rust/tests/golden/cases/*/case.json are stale for this host"
    );
    eprintln!("[corpus] no source archives present - {what} not exercised on this host");
}

/// Sorted names of every committed fixture case that ships a ModuleConfig.xml.
pub fn committed_cases() -> Vec<String> {
    let mut cases: Vec<String> = fs::read_dir(golden_cases_dir())
        .expect("golden cases dir")
        .filter_map(|e| {
            let e = e.unwrap();
            e.path()
                .join("ModuleConfig.xml")
                .exists()
                .then(|| e.file_name().to_string_lossy().into_owned())
        })
        .collect();
    cases.sort();
    cases
}

/// Just-enough JSON reader for the machine-generated fixture files
/// (`archive_entries.json`, `target_tree.json`, `expected.json`): avoids a JSON
/// crate dependency; any shape surprise panics, which is the right failure mode
/// in a test.
pub mod minijson {
    #[derive(Debug)]
    pub enum Value {
        Null,
        Bool(bool),
        Number(f64),
        String(String),
        Array(Vec<Value>),
        Object(Vec<(String, Value)>),
    }

    impl Value {
        pub fn member(&self, key: &str) -> Option<&Value> {
            match self {
                Value::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                other => panic!("member({key:?}) on non-object {other:?}"),
            }
        }

        pub fn as_array(&self) -> &[Value] {
            match self {
                Value::Array(items) => items,
                other => panic!("as_array on {other:?}"),
            }
        }

        pub fn as_str(&self) -> &str {
            match self {
                Value::String(s) => s,
                other => panic!("as_str on {other:?}"),
            }
        }

        pub fn as_u64(&self) -> u64 {
            match self {
                // Fixture sizes stay far below 2^53; f64 is exact there.
                Value::Number(n) => *n as u64,
                other => panic!("as_u64 on {other:?}"),
            }
        }

        pub fn as_bool(&self) -> bool {
            match self {
                Value::Bool(b) => *b,
                other => panic!("as_bool on {other:?}"),
            }
        }
    }

    pub fn parse(text: &str) -> Value {
        let mut p = Parser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        let value = p.value();
        p.skip_ws();
        assert_eq!(p.pos, p.bytes.len(), "trailing JSON content");
        value
    }

    struct Parser<'a> {
        bytes: &'a [u8],
        pos: usize,
    }

    impl Parser<'_> {
        fn skip_ws(&mut self) {
            while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                self.pos += 1;
            }
        }

        fn expect(&mut self, b: u8) {
            assert_eq!(self.bytes.get(self.pos), Some(&b), "at byte {}", self.pos);
            self.pos += 1;
        }

        fn value(&mut self) -> Value {
            self.skip_ws();
            match *self.bytes.get(self.pos).expect("unexpected JSON end") {
                b'{' => self.object(),
                b'[' => self.array(),
                b'"' => Value::String(self.string()),
                b't' => self.literal("true", Value::Bool(true)),
                b'f' => self.literal("false", Value::Bool(false)),
                b'n' => self.literal("null", Value::Null),
                _ => self.number(),
            }
        }

        fn literal(&mut self, word: &str, value: Value) -> Value {
            assert!(
                self.bytes[self.pos..].starts_with(word.as_bytes()),
                "bad literal at byte {}",
                self.pos
            );
            self.pos += word.len();
            value
        }

        fn object(&mut self) -> Value {
            self.expect(b'{');
            let mut pairs = Vec::new();
            self.skip_ws();
            if self.bytes.get(self.pos) == Some(&b'}') {
                self.pos += 1;
                return Value::Object(pairs);
            }
            loop {
                self.skip_ws();
                let key = self.string();
                self.skip_ws();
                self.expect(b':');
                pairs.push((key, self.value()));
                self.skip_ws();
                match self.bytes.get(self.pos) {
                    Some(b',') => self.pos += 1,
                    Some(b'}') => {
                        self.pos += 1;
                        return Value::Object(pairs);
                    }
                    other => panic!("bad object separator {other:?} at byte {}", self.pos),
                }
            }
        }

        fn array(&mut self) -> Value {
            self.expect(b'[');
            let mut items = Vec::new();
            self.skip_ws();
            if self.bytes.get(self.pos) == Some(&b']') {
                self.pos += 1;
                return Value::Array(items);
            }
            loop {
                items.push(self.value());
                self.skip_ws();
                match self.bytes.get(self.pos) {
                    Some(b',') => self.pos += 1,
                    Some(b']') => {
                        self.pos += 1;
                        return Value::Array(items);
                    }
                    other => panic!("bad array separator {other:?} at byte {}", self.pos),
                }
            }
        }

        fn string(&mut self) -> String {
            self.expect(b'"');
            let mut out = String::new();
            loop {
                match *self.bytes.get(self.pos).expect("unterminated JSON string") {
                    b'"' => {
                        self.pos += 1;
                        return out;
                    }
                    b'\\' => {
                        self.pos += 1;
                        let esc = *self.bytes.get(self.pos).expect("truncated escape");
                        self.pos += 1;
                        match esc {
                            b'"' => out.push('"'),
                            b'\\' => out.push('\\'),
                            b'/' => out.push('/'),
                            b'b' => out.push('\u{8}'),
                            b'f' => out.push('\u{c}'),
                            b'n' => out.push('\n'),
                            b'r' => out.push('\r'),
                            b't' => out.push('\t'),
                            b'u' => {
                                let s = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
                                    .unwrap();
                                self.pos += 4;
                                let code = u32::from_str_radix(s, 16).expect("bad \\u escape");
                                out.push(char::from_u32(code).expect("bad \\u code point"));
                            }
                            other => panic!("unsupported escape \\{}", other as char),
                        }
                    }
                    b => {
                        let len = match b {
                            0..=0x7f => 1,
                            0xc0..=0xdf => 2,
                            0xe0..=0xef => 3,
                            _ => 4,
                        };
                        out.push_str(
                            std::str::from_utf8(&self.bytes[self.pos..self.pos + len]).unwrap(),
                        );
                        self.pos += len;
                    }
                }
            }
        }

        fn number(&mut self) -> Value {
            let start = self.pos;
            while matches!(
                self.bytes.get(self.pos),
                Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
            ) {
                self.pos += 1;
            }
            let s = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap();
            Value::Number(s.parse().unwrap_or_else(|_| panic!("bad number {s:?}")))
        }
    }
}

// ---------------------------------------------------------------------------
// Caller input prep, replicating FomodInferenceService.cpp:833-886.
// ---------------------------------------------------------------------------

/// Raw archive listing in document order: (path, size) pairs from
/// `archive_entries.json` (order matters for last-write-wins size collisions).
pub fn load_archive_entries(case_dir: &Path) -> Vec<(String, u64)> {
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

/// Installed-file map from `target_tree.json` (path -> size).
pub fn load_installed_files(case_dir: &Path) -> HashMap<String, u64> {
    let text =
        fs::read_to_string(case_dir.join("target_tree.json")).expect("target_tree.json readable");
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

/// Build a [`TargetTree`] from `target_tree.json` INCLUDING the committed FNV-1a
/// hashes (`{path, size, fnv1a}`), the size+hash target the C++ produces after
/// `scan_installed_files` + `hash_contested_files`. Task 12's fixture test uses
/// this to SUBSTITUTE for the mod-dir scan (the installed files are absent in the
/// committed corpus). The `fnv1a` field is a lowercase hex u64.
///
/// Mirror of `build_target_tree`'s top-level `meta.ini` skip so the substituted
/// tree matches the engine's (no committed case ships a `meta.ini` row, but the
/// rule is kept for faithfulness).
pub fn load_target_tree_with_hashes(case_dir: &Path) -> TargetTree {
    let text =
        fs::read_to_string(case_dir.join("target_tree.json")).expect("target_tree.json readable");
    let mut target = TargetTree::new();
    for e in minijson::parse(&text).as_array() {
        let path = e.member("path").expect("path").as_str().to_string();
        if path == "meta.ini" {
            continue;
        }
        let size = e.member("size").expect("size").as_u64();
        let hash_hex = e.member("fnv1a").expect("fnv1a").as_str();
        let hash = u64::from_str_radix(hash_hex, 16).expect("fnv1a is hex u64");
        target.insert(path, TargetFile { size, hash });
    }
    target
}

/// Load and parse a fixture's `expected.json` (the authoritative C++ output).
pub fn load_expected(case: &str) -> minijson::Value {
    let text = fs::read_to_string(golden_cases_dir().join(case).join("expected.json"))
        .expect("expected.json readable");
    minijson::parse(&text)
}

/// Mirror of the engine's entry-index prep (`FomodInferenceService.cpp:836`):
/// skip directory markers (trailing "/" or "\\"), normalize each survivor
/// (keeping duplicates), byte-wise sort; sizes keyed by normalized path with
/// the value looked up by the ORIGINAL path, last-write-wins on collisions.
pub fn prep_entries(raw: &[(String, u64)]) -> (Vec<String>, HashMap<String, u64>) {
    let mut sorted_norm = Vec::with_capacity(raw.len());
    let mut norm_sizes = HashMap::new();
    for (path, size) in raw {
        if path.ends_with('/') || path.ends_with('\\') {
            continue;
        }
        let norm = normalize_path(path);
        sorted_norm.push(norm.clone());
        norm_sizes.insert(norm, *size);
    }
    sorted_norm.sort();
    (sorted_norm, norm_sizes)
}

/// Mirror of the engine's fomod-prefix derivation
/// (`FomodInferenceService.cpp:850-882`): candidates are normalized entries
/// equal to `fomod/moduleconfig.xml` or ending with `/fomod/moduleconfig.xml`
/// (path-boundary check); pick the shallowest by '/' count, ties broken by
/// shorter total length keeping the first otherwise; then strip the suffix and
/// its joining slash.
pub fn derive_prefix(raw: &[(String, u64)]) -> Option<String> {
    const SUFFIX: &str = "fomod/moduleconfig.xml";
    let mut best = String::new();
    let mut best_depth = usize::MAX;
    for (path, _) in raw {
        let norm = normalize_path(path);
        let is_candidate =
            norm == SUFFIX || (norm.len() > SUFFIX.len() && norm.ends_with(&format!("/{SUFFIX}")));
        if !is_candidate {
            continue;
        }
        let depth = norm.matches('/').count();
        if depth < best_depth
            || (depth == best_depth && (best.is_empty() || norm.len() < best.len()))
        {
            best = norm;
            best_depth = depth;
        }
    }
    if best.is_empty() {
        return None;
    }
    let mut suffix_pos = best.len() - SUFFIX.len();
    if suffix_pos > 0 && best.as_bytes()[suffix_pos - 1] == b'/' {
        suffix_pos -= 1;
    }
    Some(best[..suffix_pos].to_string())
}

/// Fully-prepared inputs for a fixture case: the derived prefix, the parsed IR,
/// and the Task 5 pipeline outputs.
pub struct CaseRun {
    pub prefix: String,
    pub installer: FomodInstaller,
    pub atoms: ExpandedAtoms,
    pub index: AtomIndex,
    pub excluded: HashSet<String>,
    pub target: TargetTree,
}

/// Reconstruct the C++ solver's `[step][group][plugin]` selection grid from a
/// fixture's `expected.json`. Hoisted from the Task 6 forward-simulator
/// fixtures so the Task 7 propagator fixtures share the exact positional walk.
///
/// The IR is walked by position; `expected.json` emits one entry per IR step
/// and per IR group (verified by the alignment asserts). Within a group each IR
/// plugin is selected iff its name appears in that group's `plugins` (selected)
/// array; name matches are consumed in order so duplicate plugin names within a
/// group resolve positionally.
pub fn build_selection_grid(
    installer: &FomodInstaller,
    expected: &minijson::Value,
    case: &str,
) -> Vec<Vec<Vec<bool>>> {
    let steps = expected.member("steps").expect("steps").as_array();
    assert_eq!(
        steps.len(),
        installer.steps.len(),
        "{case}: expected.json step count vs IR step count"
    );

    let mut grid = Vec::with_capacity(installer.steps.len());
    for (si, ir_step) in installer.steps.iter().enumerate() {
        let egroups = steps[si].member("groups").expect("groups").as_array();
        assert_eq!(
            egroups.len(),
            ir_step.groups.len(),
            "{case}: step {si} group count vs IR"
        );

        let mut step_grid = Vec::with_capacity(ir_step.groups.len());
        for (gi, ir_group) in ir_step.groups.iter().enumerate() {
            let selected = egroups[gi].member("plugins").expect("plugins").as_array();
            let deselected: &[minijson::Value] = egroups[gi]
                .member("deselected")
                .map(|v| v.as_array())
                .unwrap_or(&[]);
            assert_eq!(
                selected.len() + deselected.len(),
                ir_group.plugins.len(),
                "{case}: step {si} group {gi}: selected+deselected != IR plugin count"
            );

            // Multiset of selected plugin names; duplicates resolve positionally.
            let mut sel_counts: HashMap<String, i32> = HashMap::new();
            for p in selected {
                *sel_counts
                    .entry(p.member("name").expect("name").as_str().to_string())
                    .or_insert(0) += 1;
            }

            let mut group_grid = Vec::with_capacity(ir_group.plugins.len());
            for ir_plugin in &ir_group.plugins {
                let take = sel_counts.get_mut(&ir_plugin.name).is_some_and(|n| {
                    if *n > 0 {
                        *n -= 1;
                        true
                    } else {
                        false
                    }
                });
                group_grid.push(take);
            }
            step_grid.push(group_grid);
        }
        grid.push(step_grid);
    }
    grid
}

/// Prepare a fixture case end to end: load archive listing + installed files,
/// derive the prefix, parse the XML, then run the Task 5 pipeline.
pub fn run_case(case: &str) -> CaseRun {
    let case_dir = golden_cases_dir().join(case);
    let raw = load_archive_entries(&case_dir);
    let (sorted_entries, entry_sizes) = prep_entries(&raw);
    let prefix = derive_prefix(&raw).unwrap_or_else(|| panic!("{case}: no fomod prefix found"));
    let bytes = fs::read(case_dir.join("ModuleConfig.xml"))
        .unwrap_or_else(|e| panic!("read fixture {case}: {e}"));
    let installer = parse_module_config(&bytes, &prefix)
        .unwrap_or_else(|e| panic!("parse fixture {case}: {e}"));
    let atoms = expand_all_atoms(&installer, &sorted_entries, &entry_sizes);
    let index = build_atom_index(&atoms);
    let excluded = compute_excluded_dests(&index);
    let target = build_target_tree(&load_installed_files(&case_dir));
    CaseRun {
        prefix,
        installer,
        atoms,
        index,
        excluded,
        target,
    }
}
