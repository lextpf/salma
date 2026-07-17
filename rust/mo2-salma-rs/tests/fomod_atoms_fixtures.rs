//! Fixture-driven atom-expansion tests over the committed golden cases in
//! `rust/tests/golden/cases/` (Task 5).
//!
//! For every committed fixture with a ModuleConfig.xml this replicates the
//! engine's input preparation (`FomodInferenceService.cpp` steps 1-2: build
//! `sorted_norm_entries` / `norm_entry_sizes` from the archive listing and
//! derive the fomod prefix from the shallowest `fomod/ModuleConfig.xml`
//! entry), parses the XML, and runs the full Task 5 pipeline:
//! `expand_all_atoms` -> `build_atom_index` -> `compute_excluded_dests` ->
//! `build_target_tree`.
//!
//! Expected counts were derived with a throwaway script mirroring the C++
//! rules and hand-verified for three diverse fixtures
//! (zip_exactlyone_mu_joint_fix, zip_exactlyone_racecompat,
//! zip_11step_cbbe_3ba) against the raw XML and archive_entries.json; see
//! PARITY-NOTES "Task 5" for the derivation record.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use mo2_salma_rs::fomod_atom::{AtomIndex, ExpandedAtoms, FomodAtom, Origin, TargetTree};
use mo2_salma_rs::fomod_inference_atoms::{
    build_atom_index, build_target_tree, compute_excluded_dests, expand_all_atoms,
};
use mo2_salma_rs::fomod_ir_parser::parse_module_config;
use mo2_salma_rs::utils::normalize_path;

fn golden_cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/cases")
}

/// Just-enough JSON reader for the machine-generated fixture files
/// (`archive_entries.json`, `target_tree.json`, `expected.json`), same
/// pattern as `tests/fomod_ir_fixtures.rs`: avoids a JSON crate dependency;
/// any shape surprise panics, which is the right failure mode in a test.
mod minijson {
    #[derive(Debug)]
    pub enum Value {
        Null,
        #[allow(dead_code)]
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
fn load_archive_entries(case_dir: &Path) -> Vec<(String, u64)> {
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
fn load_installed_files(case_dir: &Path) -> HashMap<String, u64> {
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

/// Mirror of the engine's entry-index prep (`FomodInferenceService.cpp:836`):
/// skip directory markers (trailing "/" or "\\"), normalize each survivor
/// (keeping duplicates), byte-wise sort; sizes keyed by normalized path with
/// the value looked up by the ORIGINAL path, last-write-wins on collisions.
fn prep_entries(raw: &[(String, u64)]) -> (Vec<String>, HashMap<String, u64>) {
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
/// shorter total length keeping the first otherwise; then strip the suffix
/// and its joining slash.
fn derive_prefix(raw: &[(String, u64)]) -> Option<String> {
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

struct CaseRun {
    prefix: String,
    atoms: ExpandedAtoms,
    index: AtomIndex,
    excluded: std::collections::HashSet<String>,
    target: TargetTree,
}

fn run_case(case: &str) -> CaseRun {
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
        atoms,
        index,
        excluded,
        target,
    }
}

// ---------------------------------------------------------------------------
// Per-fixture numeric expectations. Derived with a scratchpad script that
// mirrors the C++ rules (see the module doc comment); the three detail
// fixtures below were hand-verified against the raw XML + archive entries
// before freezing these literals.
// ---------------------------------------------------------------------------

struct CountsExpect {
    case: &'static str,
    required: usize,
    plugin: usize,
    conditional: usize,
    atom_index: usize,
    excluded: usize,
}

const COUNTS: &[CountsExpect] = &[
    CountsExpect {
        case: "rar_7step_sos",
        required: 99,
        plugin: 258,
        conditional: 0,
        atom_index: 157,
        excluded: 0,
    },
    CountsExpect {
        case: "rar_exactlyone_heel_volume",
        required: 0,
        plugin: 3,
        conditional: 0,
        atom_index: 1,
        excluded: 0,
    },
    CountsExpect {
        case: "rar_selectall_cbpc_config",
        required: 0,
        plugin: 4,
        conditional: 0,
        atom_index: 4,
        excluded: 0,
    },
    CountsExpect {
        case: "sevenz_2step_nec_feet",
        required: 0,
        plugin: 31,
        conditional: 0,
        atom_index: 31,
        excluded: 0,
    },
    CountsExpect {
        case: "sevenz_3step_tk_dodge",
        required: 0,
        plugin: 12,
        conditional: 0,
        atom_index: 12,
        excluded: 0,
    },
    CountsExpect {
        case: "sevenz_3step_yorha_patches",
        required: 0,
        plugin: 6,
        conditional: 0,
        atom_index: 6,
        excluded: 0,
    },
    CountsExpect {
        case: "sevenz_4step_lewdmarks",
        required: 0,
        plugin: 0,
        conditional: 36,
        atom_index: 11,
        excluded: 0,
    },
    CountsExpect {
        case: "sevenz_9step_the_pure",
        required: 0,
        plugin: 178,
        conditional: 0,
        atom_index: 41,
        excluded: 0,
    },
    CountsExpect {
        case: "sevenz_atmostone_slavetats_riek",
        required: 0,
        plugin: 64,
        conditional: 0,
        atom_index: 32,
        excluded: 0,
    },
    CountsExpect {
        case: "sevenz_selectall_hh_walk",
        required: 0,
        plugin: 122,
        conditional: 0,
        atom_index: 72,
        excluded: 0,
    },
    CountsExpect {
        case: "sevenz_selectany_racemenu_plugins",
        required: 0,
        plugin: 9,
        conditional: 0,
        atom_index: 9,
        excluded: 0,
    },
    CountsExpect {
        case: "zip_11step_cbbe_3ba",
        required: 0,
        plugin: 493,
        conditional: 185,
        atom_index: 229,
        excluded: 0,
    },
    CountsExpect {
        case: "zip_atmostone_heels_srd",
        required: 1,
        plugin: 6,
        conditional: 0,
        atom_index: 6,
        excluded: 0,
    },
    CountsExpect {
        case: "zip_exactlyone_mu_joint_fix",
        required: 4,
        plugin: 4,
        conditional: 0,
        atom_index: 6,
        excluded: 0,
    },
    CountsExpect {
        case: "zip_exactlyone_racecompat",
        required: 1,
        plugin: 142,
        conditional: 4,
        atom_index: 96,
        excluded: 0,
    },
];

#[test]
fn every_fixture_matches_the_derived_atom_counts() {
    // Guard: the table covers exactly the committed ModuleConfig.xml cases.
    let mut on_disk: Vec<String> = fs::read_dir(golden_cases_dir())
        .expect("golden cases dir")
        .filter_map(|e| {
            let e = e.unwrap();
            e.path()
                .join("ModuleConfig.xml")
                .exists()
                .then(|| e.file_name().to_string_lossy().into_owned())
        })
        .collect();
    on_disk.sort();
    let mut in_table: Vec<String> = COUNTS.iter().map(|c| c.case.to_string()).collect();
    in_table.sort();
    assert_eq!(
        on_disk, in_table,
        "table must cover every committed fixture"
    );

    for expect in COUNTS {
        let case = expect.case;
        let run = run_case(case);
        let n_required = run.atoms.required.len();
        let n_plugin: usize = run.atoms.per_plugin.iter().map(Vec::len).sum();
        let n_conditional: usize = run.atoms.per_conditional.iter().map(Vec::len).sum();
        assert_eq!(n_required, expect.required, "{case}: required atom count");
        assert_eq!(n_plugin, expect.plugin, "{case}: plugin atom count");
        assert_eq!(
            n_conditional, expect.conditional,
            "{case}: conditional atom count"
        );
        let mut total = 0usize;
        run.atoms.for_each(|_| total += 1);
        assert_eq!(
            total,
            expect.required + expect.plugin + expect.conditional,
            "{case}: total atom count via for_each"
        );
        assert_eq!(
            run.index.len(),
            expect.atom_index,
            "{case}: atom_index size"
        );
        assert_eq!(
            run.excluded.len(),
            expect.excluded,
            "{case}: excluded_dests size"
        );
        // The index is exactly the distinct dest_path set.
        let mut index_total = 0usize;
        for v in run.index.values() {
            index_total += v.len();
        }
        assert_eq!(index_total, total, "{case}: index holds every atom");
    }
}

/// C++-grounded property: the golden targets were produced by real installs
/// replayed from these archives, so every installed file (except MO2's
/// meta.ini) must be reachable by some atom.
///
/// Documented exception: rar_exactlyone_heel_volume's installed mod folder
/// contains 35 files (base-mod .wav content and a readme) that are NOT in the
/// 8-entry archive at all - the FOMOD patch was installed into an existing
/// mod folder. The authoritative C++ output for that case
/// (`expected.json` -> diagnostics.repro) records exactly `missing: 35,
/// reproduced: 1`, so the exception pins the same numbers the C++ DLL
/// produced rather than weakening the property.
#[test]
fn every_target_dest_is_reachable_by_some_atom() {
    for expect in COUNTS {
        let case = expect.case;
        let run = run_case(case);
        let missing: Vec<&String> = {
            let mut m: Vec<&String> = run
                .target
                .keys()
                .filter(|dest| !run.index.contains_key(*dest))
                .collect();
            m.sort();
            m
        };
        if case == "rar_exactlyone_heel_volume" {
            // Cross-check the exception against the C++ DLL's own repro
            // diagnostics for this fixture.
            let text = fs::read_to_string(golden_cases_dir().join(case).join("expected.json"))
                .expect("expected.json readable");
            let root = minijson::parse(&text);
            let repro = root
                .member("diagnostics")
                .expect("diagnostics")
                .member("repro")
                .expect("repro");
            let cpp_missing = repro.member("missing").expect("missing").as_u64() as usize;
            let cpp_reproduced = repro.member("reproduced").expect("reproduced").as_u64() as usize;
            assert_eq!(missing.len(), cpp_missing, "{case}: missing-count parity");
            assert_eq!(
                run.target.len() - missing.len(),
                cpp_reproduced,
                "{case}: reproduced-count parity"
            );
            // The single covered target file is the plugin payload.
            assert!(run.index.contains_key("heels sound volume.esp"));
            continue;
        }
        assert!(
            missing.is_empty(),
            "{case}: target dests unreachable by any atom: {missing:?}"
        );
    }
}

/// The prefix derived from the archive listing (the engine's real code path)
/// must agree with the prefix derived from case.json's `module_config_entry`
/// (how the Task 4 parser fixtures derive it).
#[test]
fn derived_prefix_matches_case_json_module_config_entry() {
    for expect in COUNTS {
        let case_dir = golden_cases_dir().join(expect.case);
        let raw = load_archive_entries(&case_dir);
        let derived = derive_prefix(&raw).expect("prefix");
        let text = fs::read_to_string(case_dir.join("case.json")).expect("case.json");
        let entry = minijson::parse(&text)
            .member("module_config_entry")
            .expect("module_config_entry")
            .as_str()
            .to_string();
        let norm = normalize_path(&entry);
        const SUFFIX: &str = "fomod/moduleconfig.xml";
        assert!(norm.ends_with(SUFFIX), "{}: entry {norm:?}", expect.case);
        let mut pos = norm.len() - SUFFIX.len();
        if pos > 0 && norm.as_bytes()[pos - 1] == b'/' {
            pos -= 1;
        }
        assert_eq!(derived, norm[..pos], "{}: prefix agreement", expect.case);
    }
}

/// Synthetic checks for the prefix-derivation boundary logic
/// (FomodInferenceService.cpp:850-882).
#[test]
fn derive_prefix_boundary_and_shallowest_rules() {
    let e = |p: &str| (p.to_string(), 0u64);
    // Path-boundary: "xfomod/moduleconfig.xml" is NOT a candidate.
    assert_eq!(derive_prefix(&[e("xfomod/ModuleConfig.xml")]), None);
    // Top-level entry -> empty prefix.
    assert_eq!(
        derive_prefix(&[e("fomod/ModuleConfig.xml")]),
        Some(String::new())
    );
    // Nested entry -> prefix without the joining slash.
    assert_eq!(
        derive_prefix(&[e("Mod Root/Fomod/ModuleConfig.xml")]),
        Some("mod root".to_string())
    );
    // Shallowest wins regardless of listing order.
    assert_eq!(
        derive_prefix(&[
            e("a/b/fomod/ModuleConfig.xml"),
            e("c/fomod/ModuleConfig.xml")
        ]),
        Some("c".to_string())
    );
    // Equal depth: shorter total length wins.
    assert_eq!(
        derive_prefix(&[
            e("longername/fomod/ModuleConfig.xml"),
            e("tiny/fomod/ModuleConfig.xml")
        ]),
        Some("tiny".to_string())
    );
    // Equal depth and equal length: first seen wins.
    assert_eq!(
        derive_prefix(&[
            e("aaaa/fomod/ModuleConfig.xml"),
            e("bbbb/fomod/ModuleConfig.xml")
        ]),
        Some("aaaa".to_string())
    );
}

/// Synthetic checks for the entry-prep rules (FomodInferenceService.cpp:836).
#[test]
fn prep_entries_skips_dir_markers_and_last_write_wins() {
    let raw = vec![
        ("Dir/".to_string(), 0u64),
        ("Dir\\".to_string(), 0u64),
        ("B.txt".to_string(), 2u64),
        // Two originals normalize to the same key: last write wins, and the
        // duplicate normalized path is KEPT in the sorted list.
        ("a.TXT".to_string(), 10u64),
        ("A.txt".to_string(), 20u64),
    ];
    let (sorted, sizes) = prep_entries(&raw);
    assert_eq!(sorted, ["a.txt", "a.txt", "b.txt"]);
    assert_eq!(sizes.len(), 2);
    assert_eq!(sizes["a.txt"], 20);
    assert_eq!(sizes["b.txt"], 2);
}

// ---------------------------------------------------------------------------
// Sample dest-mapping assertions on the three hand-verified fixtures.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn assert_atom(
    case: &str,
    a: &FomodAtom,
    source: &str,
    dest: &str,
    priority: i32,
    doc: i32,
    origin: Origin,
    plugin_index: i32,
    conditional_index: i32,
    size: u64,
) {
    assert_eq!(a.source_path, source, "{case}: source_path");
    assert_eq!(a.dest_path, dest, "{case}: dest_path");
    assert_eq!(a.priority, priority, "{case}: priority of {source}");
    assert_eq!(a.document_order, doc, "{case}: document_order of {source}");
    assert_eq!(a.origin, origin, "{case}: origin of {source}");
    assert_eq!(
        a.plugin_index, plugin_index,
        "{case}: plugin_index of {source}"
    );
    assert_eq!(
        a.conditional_index, conditional_index,
        "{case}: conditional_index of {source}"
    );
    assert_eq!(a.file_size, size, "{case}: file_size of {source}");
    assert_eq!(a.content_hash, 0, "{case}: content_hash starts 0");
}

#[test]
fn mu_joint_fix_sample_atoms() {
    let case = "zip_exactlyone_mu_joint_fix";
    let run = run_case(case);
    assert_eq!(run.prefix, "");

    // Required folder "base" -> 4 atoms, all sharing document_order 0.
    let req = &run.atoms.required;
    assert_eq!(req.len(), 4);
    assert!(req.iter().all(|a| a.document_order == 0));
    assert_atom(
        case,
        &req[0],
        "base/scripts/mujointfixutil.pex",
        "scripts/mujointfixutil.pex",
        0,
        0,
        Origin::Required,
        -1,
        -1,
        1232,
    );
    assert_atom(
        case,
        &req[3],
        "base/skse/plugins/mujointfix.log",
        "skse/plugins/mujointfix.log",
        0,
        0,
        Origin::Required,
        -1,
        -1,
        0,
    );

    // Plugin folders "base_seae" (flat 0, doc 1) and "base_vr" (flat 1,
    // doc 2), each 2 atoms at empty destination (rel at root).
    assert_atom(
        case,
        &run.atoms.per_plugin[0][0],
        "base_seae/skse/plugins/mujointfix.dll",
        "skse/plugins/mujointfix.dll",
        0,
        1,
        Origin::Plugin,
        0,
        -1,
        802816,
    );
    assert_atom(
        case,
        &run.atoms.per_plugin[1][1],
        "base_vr/skse/plugins/mujointfix.pdb",
        "skse/plugins/mujointfix.pdb",
        0,
        2,
        Origin::Plugin,
        1,
        -1,
        27054080,
    );

    // Contested destination: both plugins ship the dll; index preserves the
    // for_each order (flat 0 before flat 1).
    let dll = &run.index["skse/plugins/mujointfix.dll"];
    assert_eq!(dll.len(), 2);
    assert_eq!(dll[0].plugin_index, 0);
    assert_eq!(dll[0].source_path, "base_seae/skse/plugins/mujointfix.dll");
    assert_eq!(dll[1].plugin_index, 1);
    assert_eq!(dll[1].source_path, "base_vr/skse/plugins/mujointfix.dll");
}

#[test]
fn racecompat_sample_atoms() {
    let case = "zip_exactlyone_racecompat";
    let run = run_case(case);
    assert_eq!(run.prefix, "");

    // Required <file> entry (file branch, explicit destination).
    assert_atom(
        case,
        &run.atoms.required[0],
        "00 core/racecompatibility readme.txt",
        "racecompatibility readme.txt",
        0,
        0,
        Origin::Required,
        -1,
        -1,
        12579,
    );

    // Groups are re-sorted Ascending (no order attr on optionalFileGroups):
    // flat 0 = "Moonlight Tales"/"Install", whose first folder expands from
    // "20 Dawnguard Werewolf Script\Scripts".
    assert_atom(
        case,
        &run.atoms.per_plugin[0][0],
        "20 dawnguard werewolf script/scripts/playerwerewolfchangescript.pex",
        "scripts/playerwerewolfchangescript.pex",
        0,
        1,
        Origin::Plugin,
        0,
        -1,
        13344,
    );

    // flat 4 = "Optional Vampire Lord Transformation fix script"/"Install",
    // priority 3, doc 13 (after P2's 4 entries doc 3-6 and P3's 6 entries
    // doc 7-12).
    assert_atom(
        case,
        &run.atoms.per_plugin[4][0],
        "25 dawnguard vamp lord transform fix/scripts/dlc1vampiretransformvisual.pex",
        "scripts/dlc1vampiretransformvisual.pex",
        3,
        13,
        Origin::Plugin,
        4,
        -1,
        2912,
    );

    // flat 7 = "Better Vampires": its LAST entry is a <file> with NO
    // destination attribute -> destination falls back to the (normalized)
    // source path; doc 17 (folder Scripts doc 15, folder Source doc 16).
    let p7 = &run.atoms.per_plugin[7];
    assert_eq!(p7.len(), 11);
    assert_atom(
        case,
        p7.last().unwrap(),
        "30 dawnguard bettervampires/readme bettervamps+racecompatibility for dg.txt",
        "30 dawnguard bettervampires/readme bettervamps+racecompatibility for dg.txt",
        2,
        17,
        Origin::Plugin,
        7,
        -1,
        878,
    );

    // flat 9 = "Sacrilege", second folder entry (Source), doc 21.
    assert_atom(
        case,
        &run.atoms.per_plugin[9][1],
        "32 dawnguard sacrilege/source/scripts/playervampirequestscript.psc",
        "source/scripts/playervampirequestscript.psc",
        2,
        21,
        Origin::Plugin,
        9,
        -1,
        26307,
    );

    // Conditional pattern 0, first folder entry: doc 22 (conditionals start
    // after the 21 plugin entries + 1 required entry).
    assert_atom(
        case,
        &run.atoms.per_conditional[0][0],
        "20 dawnguard script/scripts/playervampirequestscript.pex",
        "scripts/playervampirequestscript.pex",
        1,
        22,
        Origin::Conditional,
        -1,
        0,
        13466,
    );

    // Contested destination across origins: index order follows for_each
    // (plugins in flat order, then conditionals in pattern order).
    let contested = &run.index["scripts/playervampirequestscript.pex"];
    let picture: Vec<(&str, Origin, i32, i32)> = contested
        .iter()
        .map(|a| {
            (
                a.source_path.as_str(),
                a.origin,
                a.priority,
                a.document_order,
            )
        })
        .collect();
    assert_eq!(
        picture,
        [
            (
                "30 dawnguard bettervampires/scripts/playervampirequestscript.pex",
                Origin::Plugin,
                2,
                15
            ),
            (
                "31 dawnguard sacrosanct/scripts/playervampirequestscript.pex",
                Origin::Plugin,
                2,
                18
            ),
            (
                "32 dawnguard sacrilege/scripts/playervampirequestscript.pex",
                Origin::Plugin,
                2,
                20
            ),
            (
                "20 dawnguard script/scripts/playervampirequestscript.pex",
                Origin::Conditional,
                1,
                22
            ),
            (
                "21 dawnguard uskp script/scripts/playervampirequestscript.pex",
                Origin::Conditional,
                1,
                24
            ),
        ]
    );
}

#[test]
fn cbbe_3ba_sample_atoms() {
    let case = "zip_11step_cbbe_3ba";
    let run = run_case(case);
    assert_eq!(run.prefix, "");

    // Stress shape: 100 flat plugins, 97 conditional patterns.
    assert_eq!(run.atoms.per_plugin.len(), 100);
    assert_eq!(run.atoms.per_conditional.len(), 97);

    // Plugin 0 "Base install": folder "00 Base\CalienteTools" ->
    // "CalienteTools", 35 atoms all sharing doc 0 (no required files).
    let p0 = &run.atoms.per_plugin[0];
    assert_eq!(p0.len(), 35);
    assert!(p0.iter().all(|a| a.document_order == 0));
    assert_atom(
        case,
        &p0[0],
        "00 base/calientetools/bodyslide/reftemplates/cbbe 3ba unibody.xml",
        "calientetools/bodyslide/reftemplates/cbbe 3ba unibody.xml",
        0,
        0,
        Origin::Plugin,
        0,
        -1,
        3176,
    );

    // Plugin 1 "Nevernude": root destination folder, doc 1.
    let p1 = &run.atoms.per_plugin[1];
    assert_eq!(p1.len(), 5);
    assert_atom(
        case,
        &p1[0],
        "00 base - nevernude/calientetools/bodyslide/shapedata/se 3bbb amazing/cbbe 3bbb amazing nevernude uniboob.nif",
        "calientetools/bodyslide/shapedata/se 3bbb amazing/cbbe 3bbb amazing nevernude uniboob.nif",
        0,
        1,
        Origin::Plugin,
        1,
        -1,
        2129725,
    );

    // Plugin 20 "New Type" (step 3, group 0, plugin 0): single <file> entry,
    // file branch keeps the parser-normalized destination.
    let p20 = &run.atoms.per_plugin[20];
    assert_eq!(p20.len(), 1);
    assert_atom(
        case,
        &p20[0],
        "10 physics patch/75 cbpc separation/skse/plugins/cbpconfig_bbp.txt",
        "skse/plugins/cbpconfig_bbp.txt",
        0,
        28,
        Origin::Plugin,
        20,
        -1,
        817,
    );

    // Conditional pattern 0: first atom at doc 123 (the 123 plugin-side
    // entries consume doc 0-122; hand-verified by counting file/folder
    // elements in the raw XML).
    assert_atom(
        case,
        &run.atoms.per_conditional[0][0],
        "10 physics patch/60 3bca full smp/skse/plugins/hdtskinnedmeshconfigs/outfits/3bca-a-amazing.xml",
        "skse/plugins/hdtskinnedmeshconfigs/outfits/3bca-a-amazing.xml",
        0,
        123,
        Origin::Conditional,
        -1,
        0,
        96201,
    );

    // Pattern 95's folder ("02 TeraElinBase - UnderWear") matches ZERO
    // archive entries: it produces no atoms but still consumes a doc_order
    // slot, so pattern 96 lands on doc 222 (the corpus maximum).
    assert!(run.atoms.per_conditional[95].is_empty());
    let c96 = &run.atoms.per_conditional[96];
    assert_eq!(c96.len(), 2);
    assert_atom(
        case,
        &c96[0],
        "03 uniquecharacterbase - underwear/calientetools/bodyslide/slidersets/se unique 3bbb amazing - underwear.osp",
        "calientetools/bodyslide/slidersets/se unique 3bbb amazing - underwear.osp",
        0,
        222,
        Origin::Conditional,
        -1,
        96,
        205437,
    );
    let mut max_doc = 0;
    run.atoms
        .for_each(|a| max_doc = max_doc.max(a.document_order));
    assert_eq!(max_doc, 222);
}
