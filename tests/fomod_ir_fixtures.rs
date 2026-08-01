//! Fixture-driven parser tests over the committed golden ModuleConfig.xml
//! files in `rust/tests/golden/cases/` (Task 4).
//!
//! Every committed fixture is parsed through the full byte pipeline
//! (`parse_module_config`), which exercises the real corpus encodings:
//! 11 UTF-16 LE with BOM, 2 UTF-8 with BOM, 2 UTF-8 without BOM.
//!
//! Expected values were derived by reading each fixture XML (transcoded for
//! the UTF-16 ones) and applying the C++ parser rules, then cross-checked two
//! ways against independent sources: the step/group/plugin name sequences in
//! each case's `expected.json` (produced by the reference C++ DLL) and the
//! `step_count`/`group_types` fields in `case.json` (produced by the golden
//! generator's own XML scan). See PARITY-NOTES "Task 4" for the derivation
//! record.

use std::fs;
use std::path::{Path, PathBuf};

use mo2_salma_rs::fomod_ir::{
    FomodCondition, FomodConditionOp, FomodConditionType, FomodGroupType as GT, FomodInstaller,
    compute_flat_starts, total_flat_plugins,
};
use mo2_salma_rs::fomod_ir_parser::parse_module_config;
use mo2_salma_rs::types::PluginType;
use mo2_salma_rs::utils::normalize_path;

fn golden_cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/cases")
}

/// Just-enough JSON reader for the machine-generated `expected.json` fixtures
/// (nlohmann::json pretty-print output; reference C++ DLL inference results).
/// Avoids a JSON crate dependency, like `extract_json_string` below; any
/// shape surprise panics, which is the right failure mode in a test.
mod minijson {
    #[derive(Debug)]
    pub enum Value {
        Null,
        // The bool/number payloads are only read through the derived Debug
        // impl (panic messages), which dead-code analysis ignores on purpose.
        #[allow(dead_code)]
        Bool(bool),
        #[allow(dead_code)]
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
                                let c = self.unicode_escape();
                                out.push(c);
                            }
                            other => panic!("unsupported escape \\{}", other as char),
                        }
                    }
                    b => {
                        // Copy one UTF-8 sequence verbatim.
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

        fn unicode_escape(&mut self) -> char {
            let hex4 = |p: &mut Self| -> u32 {
                let s = std::str::from_utf8(&p.bytes[p.pos..p.pos + 4]).unwrap();
                p.pos += 4;
                u32::from_str_radix(s, 16).expect("bad \\u escape")
            };
            let hi = hex4(self);
            if (0xd800..0xdc00).contains(&hi) {
                // Surrogate pair: \uD8xx\uDCxx.
                assert_eq!(&self.bytes[self.pos..self.pos + 2], b"\\u");
                self.pos += 2;
                let lo = hex4(self);
                let code = 0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00);
                char::from_u32(code).expect("bad surrogate pair")
            } else {
                char::from_u32(hi).expect("bad \\u code point")
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

/// Minimal extractor for a top-level string field of the pretty-printed
/// case.json files (avoids a JSON crate dependency for one field).
fn extract_json_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let pos = text.find(&needle)? + needle.len();
    let rest = text[pos..].trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            },
            other => out.push(other),
        }
    }
    None
}

/// Derive the archive prefix the engine passes to `FomodIRParser::parse` for
/// this case: the C++ (`FomodInferenceService.cpp` step 2) normalizes the
/// located ModuleConfig.xml entry path and strips the trailing
/// `fomod/moduleconfig.xml` suffix plus its joining slash.
fn archive_prefix_for(case_dir: &Path) -> String {
    let text = fs::read_to_string(case_dir.join("case.json")).expect("case.json readable");
    let entry = extract_json_string(&text, "module_config_entry").expect("field present");
    let norm = normalize_path(&entry);
    let suffix = "fomod/moduleconfig.xml";
    assert!(
        norm.ends_with(suffix),
        "module_config_entry {norm:?} must end with the module config suffix"
    );
    let mut suffix_pos = norm.len() - suffix.len();
    if suffix_pos > 0 && norm.as_bytes()[suffix_pos - 1] == b'/' {
        suffix_pos -= 1;
    }
    norm[..suffix_pos].to_string()
}

fn parse_case(case: &str) -> (FomodInstaller, String) {
    let case_dir = golden_cases_dir().join(case);
    let bytes = fs::read(case_dir.join("ModuleConfig.xml"))
        .unwrap_or_else(|e| panic!("read fixture {case}: {e}"));
    let prefix = archive_prefix_for(&case_dir);
    let installer = parse_module_config(&bytes, &prefix)
        .unwrap_or_else(|e| panic!("parse fixture {case}: {e}"));
    (installer, prefix)
}

struct GroupExpect {
    name: &'static str,
    gtype: GT,
    plugins: usize,
}

struct StepExpect {
    name: &'static str,
    visible: bool,
    groups: &'static [GroupExpect],
}

struct CaseExpect {
    case: &'static str,
    archive_prefix: &'static str,
    required_files: usize,
    conditional_patterns: usize,
    steps: &'static [StepExpect],
}

const fn g(name: &'static str, gtype: GT, plugins: usize) -> GroupExpect {
    GroupExpect {
        name,
        gtype,
        plugins,
    }
}

const fn s(name: &'static str, visible: bool, groups: &'static [GroupExpect]) -> StepExpect {
    StepExpect {
        name,
        visible,
        groups,
    }
}

/// One entry per committed ModuleConfig.xml (15 of the 16 cases;
/// sevenz_empty_no_moduleconfig has none by design).
const CASES: &[CaseExpect] = &[
    CaseExpect {
        case: "rar_7step_sos",
        archive_prefix: "schlongs_of_skyrim_se - v1.1.4",
        required_files: 1,
        conditional_patterns: 0,
        steps: &[
            s(
                "Body Type",
                false,
                &[g("Body Type", GT::SelectExactlyOne, 2)],
            ),
            s(
                "Skin Texture",
                true,
                &[g("Skin Texture", GT::SelectExactlyOne, 2)],
            ),
            s(
                "Skin Texture",
                true,
                &[g("Skin Texture", GT::SelectExactlyOne, 2)],
            ),
            s(
                "Schlong Addons - Default - Hairless",
                true,
                &[
                    g("Schlong Addons", GT::SelectAtLeastOne, 3),
                    g("Skeletons", GT::SelectAny, 1),
                    g("Optional", GT::SelectAny, 1),
                ],
            ),
            s(
                "Schlong Addons - Default - Hairy",
                true,
                &[
                    g("Schlong Addons", GT::SelectAtLeastOne, 3),
                    g("Skeletons", GT::SelectAny, 1),
                    g("Optional", GT::SelectAny, 1),
                ],
            ),
            s(
                "Schlong Addons - BodyBuilder - Hairless",
                true,
                &[
                    g("Schlong Addons", GT::SelectAtLeastOne, 3),
                    g("Skeletons", GT::SelectAny, 1),
                    g("Optional", GT::SelectAny, 1),
                ],
            ),
            s(
                "Schlong Addons - BodyBuilder - Hairy",
                true,
                &[
                    g("Schlong Addons", GT::SelectAtLeastOne, 3),
                    g("Skeletons", GT::SelectAny, 1),
                    g("Optional", GT::SelectAny, 1),
                ],
            ),
        ],
    },
    CaseExpect {
        case: "rar_exactlyone_heel_volume",
        archive_prefix: "",
        required_files: 0,
        conditional_patterns: 0,
        steps: &[s(
            "Step1",
            false,
            &[g("Main Files", GT::SelectExactlyOne, 3)],
        )],
    },
    CaseExpect {
        case: "rar_selectall_cbpc_config",
        archive_prefix: "3ba jello butt physics (cbpc config)",
        required_files: 0,
        conditional_patterns: 0,
        steps: &[s(
            "3BA Jello Butt Physics (CBPC Config)",
            false,
            &[
                g("Butt Physics", GT::SelectAll, 1),
                g("Optional", GT::SelectAny, 2),
            ],
        )],
    },
    CaseExpect {
        case: "sevenz_2step_nec_feet",
        archive_prefix: "",
        required_files: 0,
        conditional_patterns: 0,
        steps: &[
            s(
                "Base Files",
                false,
                &[g("Nec DAZ Feet Sliders", GT::SelectAll, 1)],
            ),
            s(
                "Option Files",
                false,
                &[g(
                    "Nec Feet falmer, prisoner, priest shoes",
                    GT::SelectExactlyOne,
                    2,
                )],
            ),
        ],
    },
    CaseExpect {
        case: "sevenz_3step_tk_dodge",
        archive_prefix: "",
        required_files: 0,
        conditional_patterns: 0,
        steps: &[
            s("Necessary", false, &[g("Requirements", GT::SelectAll, 3)]),
            s("Install", false, &[g("How To", GT::SelectAll, 1)]),
            s(
                "Have Fun!!",
                false,
                &[g("Have Fun!!", GT::SelectExactlyOne, 1)],
            ),
        ],
    },
    CaseExpect {
        case: "sevenz_3step_yorha_patches",
        archive_prefix: "",
        required_files: 0,
        conditional_patterns: 0,
        steps: &[
            s(
                "Choose your body",
                false,
                &[g("Body", GT::SelectExactlyOne, 2)],
            ),
            s(
                "Choose your patches - CBBE 3BA - BHUNP",
                true,
                &[g("Choose your patches", GT::SelectExactlyOne, 3)],
            ),
            s(
                "Choose your patches - Touched by Dibella",
                true,
                &[g("Choose your patches", GT::SelectExactlyOne, 3)],
            ),
        ],
    },
    CaseExpect {
        case: "sevenz_4step_lewdmarks",
        archive_prefix: "",
        required_files: 0,
        conditional_patterns: 10,
        steps: &[
            s(
                "Select Skyrim version",
                false,
                &[g("Select Skyrim version", GT::SelectExactlyOne, 2)],
            ),
            s(
                "Select Mod Version",
                false,
                &[g("Select Mod Version", GT::SelectAtLeastOne, 2)],
            ),
            s(
                "Patch for SlaveTats",
                true,
                &[g("Patch for SlaveTats", GT::SelectAny, 1)],
            ),
            s(
                "Select your body mod",
                false,
                &[g("Select texture", GT::SelectExactlyOne, 2)],
            ),
        ],
    },
    CaseExpect {
        case: "sevenz_9step_the_pure",
        archive_prefix: "",
        required_files: 0,
        conditional_patterns: 0,
        steps: &[
            s(
                "The Pure",
                false,
                &[g("Texture Quality", GT::SelectExactlyOne, 2)],
            ),
            s(
                "The Pure",
                true,
                &[g("Diffuse Map Options", GT::SelectExactlyOne, 4)],
            ),
            s(
                "The Pure",
                true,
                &[g("Diffuse Map Options", GT::SelectExactlyOne, 4)],
            ),
            s(
                "The Pure",
                true,
                &[g("Nomal Map Options", GT::SelectExactlyOne, 4)],
            ),
            s(
                "The Pure",
                true,
                &[g("Nomal Map Options", GT::SelectExactlyOne, 4)],
            ),
            s(
                "The Pure",
                true,
                &[g("Specular Map Options", GT::SelectExactlyOne, 3)],
            ),
            s(
                "The Pure",
                true,
                &[g("Specular Map Options", GT::SelectExactlyOne, 3)],
            ),
            s(
                "The Pure",
                false,
                &[g("Subsurface Map Options", GT::SelectExactlyOne, 2)],
            ),
            s(
                "The Pure",
                false,
                &[g("Optional add-ons", GT::SelectAny, 2)],
            ),
        ],
    },
    CaseExpect {
        case: "sevenz_atmostone_slavetats_riek",
        archive_prefix: "alpia slavetats riek se-le",
        required_files: 0,
        conditional_patterns: 0,
        steps: &[s(
            "SE - LE",
            false,
            &[
                g("SE", GT::SelectAtMostOne, 1),
                g("LE", GT::SelectAtMostOne, 1),
            ],
        )],
    },
    CaseExpect {
        case: "sevenz_selectall_hh_walk",
        archive_prefix: "",
        required_files: 0,
        conditional_patterns: 0,
        steps: &[s(
            "Step1",
            false,
            &[
                g("Base Files", GT::SelectAll, 1),
                g("Version", GT::SelectExactlyOne, 3),
            ],
        )],
    },
    CaseExpect {
        case: "sevenz_selectany_racemenu_plugins",
        archive_prefix: "alpia racemenu plugins",
        required_files: 0,
        conditional_patterns: 0,
        steps: &[s("Plugins", false, &[g("RM Plugins", GT::SelectAny, 3)])],
    },
    CaseExpect {
        case: "zip_11step_cbbe_3ba",
        archive_prefix: "",
        required_files: 0,
        conditional_patterns: 97,
        steps: &[
            s(
                "CBBE 3BBB Advanced Main",
                false,
                &[
                    g("CBBE 3BA Base", GT::SelectAny, 5),
                    g("CBBE 3BA ygNord", GT::SelectAny, 2),
                    g("CBBE 3BA UniqueCharacter", GT::SelectAny, 2),
                ],
            ),
            s(
                "Base CBPC",
                false,
                &[
                    g("Physics Type Select", GT::SelectAll, 1),
                    g("CBPC ini file", GT::SelectExactlyOne, 4),
                ],
            ),
            s(
                "Physics Selecting",
                false,
                &[g("Physics Select", GT::SelectExactlyOne, 6)],
            ),
            s(
                "CBPC Physics Preset Type",
                true,
                &[g("Select Type", GT::SelectExactlyOne, 2)],
            ),
            s(
                "(CBPC) Boobs Physics Preset",
                false,
                &[
                    g("Boobs Physics Preset (CBBE 3BBB)", GT::SelectExactlyOne, 3),
                    g("Boobs Physics strength (High)", GT::SelectExactlyOne, 4),
                    g("Boobs Physics strength (Low)", GT::SelectExactlyOne, 4),
                    g("Boobs Collision Select", GT::SelectExactlyOne, 5),
                    g("Boobs Gravity", GT::SelectExactlyOne, 2),
                    g("Boobs More Gravity", GT::SelectExactlyOne, 2),
                    g("Boobs Push-Up", GT::SelectExactlyOne, 2),
                ],
            ),
            s(
                "(CBPC-NewType) Belly, Butt, Leg Physics Preset",
                true,
                &[
                    g("Belly Physics Preset", GT::SelectExactlyOne, 4),
                    g("Butt Physics Preset", GT::SelectExactlyOne, 4),
                    g("Leg Physics Preset", GT::SelectExactlyOne, 4),
                ],
            ),
            s(
                "(CBPC-OldType) Belly, Butt, Leg Physics Preset",
                true,
                &[
                    g("Basic CBPC Preset", GT::SelectAny, 1),
                    g("Leg Physics Preset", GT::SelectExactlyOne, 4),
                    g("Vagina Physics", GT::SelectAny, 1),
                ],
            ),
            s(
                "(SMP) Player Physics Preset",
                true,
                &[
                    g("Boobs Physics Preset", GT::SelectExactlyOne, 3),
                    g("Boobs Physics Strength", GT::SelectExactlyOne, 4),
                ],
            ),
            s(
                "(CBPC)Extra Physics",
                false,
                &[
                    g("SOS Physics", GT::SelectExactlyOne, 3),
                    g("Change vagina collision", GT::SelectExactlyOne, 2),
                    g("Add anal collision", GT::SelectExactlyOne, 2),
                ],
            ),
            s(
                "Extra Textures",
                false,
                &[
                    g("Vagina Textures", GT::SelectExactlyOne, 18),
                    g("Vagina - SSS", GT::SelectAtMostOne, 1),
                ],
            ),
            s(
                "Extra Patches",
                false,
                &[
                    g("Compatible Patch", GT::SelectAny, 2),
                    g("Racemenu", GT::SelectExactlyOne, 3),
                ],
            ),
        ],
    },
    CaseExpect {
        case: "zip_atmostone_heels_srd",
        archive_prefix: "",
        required_files: 1,
        conditional_patterns: 0,
        steps: &[s(
            "Main",
            false,
            &[
                g("Main", GT::SelectExactlyOne, 1),
                g("Optional - Armor Replacer", GT::SelectAny, 3),
                g("ESL-flagged plugin file", GT::SelectAtMostOne, 2),
            ],
        )],
    },
    CaseExpect {
        case: "zip_exactlyone_mu_joint_fix",
        archive_prefix: "",
        required_files: 1,
        conditional_patterns: 0,
        steps: &[s(
            "Installation",
            false,
            &[g("Base", GT::SelectExactlyOne, 2)],
        )],
    },
    CaseExpect {
        case: "zip_exactlyone_racecompat",
        archive_prefix: "",
        required_files: 1,
        conditional_patterns: 2,
        // optionalFileGroups has no order attribute -> Ascending: these four
        // groups are byte-wise sorted by name, NOT in document order.
        steps: &[s(
            "Dawnguard Optionals",
            false,
            &[
                g("Moonlight Tales", GT::SelectExactlyOne, 2),
                g(
                    "Optional Compatiblity patches (Dawnguard)",
                    GT::SelectExactlyOne,
                    2,
                ),
                g(
                    "Optional Vampire Lord Transformation fix script",
                    GT::SelectExactlyOne,
                    2,
                ),
                g(
                    "Optional Vampire overhaul patches (Dawnguard)",
                    GT::SelectExactlyOne,
                    4,
                ),
            ],
        )],
    },
];

#[test]
fn every_committed_fixture_parses_with_the_expected_structure() {
    // Guard: the committed corpus holds exactly the fixtures the table lists
    // (plus the deliberately config-less sevenz_empty_no_moduleconfig).
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
    let mut in_table: Vec<String> = CASES.iter().map(|c| c.case.to_string()).collect();
    in_table.sort();
    assert_eq!(
        on_disk, in_table,
        "fixture table must cover every committed ModuleConfig.xml"
    );
    assert!(
        !golden_cases_dir()
            .join("sevenz_empty_no_moduleconfig/ModuleConfig.xml")
            .exists(),
        "sevenz_empty_no_moduleconfig stays config-less"
    );

    for expect in CASES {
        let (installer, prefix) = parse_case(expect.case);
        let case = expect.case;
        assert_eq!(
            prefix, expect.archive_prefix,
            "{case}: derived archive prefix"
        );

        assert!(
            installer.module_dependencies.is_none(),
            "{case}: no fixture declares moduleDependencies"
        );
        assert_eq!(
            installer.required_files.len(),
            expect.required_files,
            "{case}: required file count"
        );
        assert_eq!(
            installer.conditional_patterns.len(),
            expect.conditional_patterns,
            "{case}: conditional pattern count"
        );
        assert_eq!(
            installer.steps.len(),
            expect.steps.len(),
            "{case}: step count"
        );

        for (si, (step, sexp)) in installer.steps.iter().zip(expect.steps).enumerate() {
            assert_eq!(step.name, sexp.name, "{case}: step[{si}] name (in order)");
            assert_eq!(step.ordinal, si as i32, "{case}: step[{si}] ordinal");
            assert_eq!(
                step.visible.is_some(),
                sexp.visible,
                "{case}: step[{si}] visibility condition presence"
            );
            assert_eq!(
                step.groups.len(),
                sexp.groups.len(),
                "{case}: step[{si}] group count"
            );
            for (gi, (group, gexp)) in step.groups.iter().zip(sexp.groups).enumerate() {
                assert_eq!(group.name, gexp.name, "{case}: step[{si}] group[{gi}] name");
                assert_eq!(
                    group.r#type, gexp.gtype,
                    "{case}: step[{si}] group[{gi}] type"
                );
                assert_eq!(
                    group.plugins.len(),
                    gexp.plugins,
                    "{case}: step[{si}] group[{gi}] plugin count"
                );
            }
        }
    }
}

/// Every fixture's step, group, and PLUGIN names are cross-checked in order
/// against the authoritative C++ DLL output (`expected.json`, schema v2) at
/// test time - not just the counts in the table above. The C++ assembler
/// walks the IR in order and splits each group's plugins into `plugins`
/// (selected) and `deselected`, so interleaving the two lists back together
/// must reproduce exactly the parsed IR's plugin order.
#[test]
fn every_fixture_matches_expected_json_step_group_plugin_names() {
    fn names<'v>(group: &'v minijson::Value, key: &str) -> Vec<&'v str> {
        group.member(key).map_or_else(Vec::new, |list| {
            list.as_array()
                .iter()
                .map(|p| p.member("name").expect("plugin name").as_str())
                .collect()
        })
    }

    for expect in CASES {
        let case = expect.case;
        let (installer, _) = parse_case(case);
        let text = fs::read_to_string(golden_cases_dir().join(case).join("expected.json"))
            .unwrap_or_else(|e| panic!("read expected.json for {case}: {e}"));
        let root = minijson::parse(&text);
        let steps = root.member("steps").expect("steps").as_array();
        assert_eq!(
            steps.len(),
            installer.steps.len(),
            "{case}: expected.json step count"
        );
        for (si, (ir_step, jstep)) in installer.steps.iter().zip(steps).enumerate() {
            assert_eq!(
                jstep.member("name").expect("step name").as_str(),
                ir_step.name,
                "{case}: step[{si}] name vs expected.json"
            );
            let jgroups = jstep.member("groups").expect("groups").as_array();
            assert_eq!(
                jgroups.len(),
                ir_step.groups.len(),
                "{case}: step[{si}] group count vs expected.json"
            );
            for (gi, (ir_group, jgroup)) in ir_step.groups.iter().zip(jgroups).enumerate() {
                assert_eq!(
                    jgroup.member("name").expect("group name").as_str(),
                    ir_group.name,
                    "{case}: step[{si}] group[{gi}] name vs expected.json"
                );
                let mut selected = names(jgroup, "plugins").into_iter().peekable();
                let mut deselected = names(jgroup, "deselected").into_iter().peekable();
                for ir_name in ir_group.plugins.iter().map(|p| p.name.as_str()) {
                    if selected.peek() == Some(&ir_name) {
                        selected.next();
                    } else if deselected.peek() == Some(&ir_name) {
                        deselected.next();
                    } else {
                        panic!(
                            "{case}: step[{si}] group[{gi}]: IR plugin {ir_name:?} is not next \
                             in expected.json (selected next: {:?}, deselected next: {:?})",
                            selected.peek(),
                            deselected.peek()
                        );
                    }
                }
                assert!(
                    selected.peek().is_none() && deselected.peek().is_none(),
                    "{case}: step[{si}] group[{gi}]: expected.json lists plugins the IR lacks"
                );
            }
        }
    }
}

#[test]
fn zip_exactlyone_mu_joint_fix_details() {
    let (installer, _) = parse_case("zip_exactlyone_mu_joint_fix");

    // <folder source="base" destination="" /> - destination PRESENT and empty.
    let req = &installer.required_files[0];
    assert_eq!(req.source, "base");
    assert_eq!(req.destination, "");
    assert_eq!(req.priority, 0);
    assert!(req.is_folder);
    assert!(!req.always_install);
    assert!(!req.install_if_usable);

    let group = &installer.steps[0].groups[0];
    let names: Vec<&str> = group.plugins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["SE_AE", "VR"]);
    assert!(
        group
            .plugins
            .iter()
            .all(|p| p.r#type == PluginType::Optional)
    );

    let se_ae = &group.plugins[0];
    assert_eq!(se_ae.files.len(), 1);
    assert_eq!(se_ae.files[0].source, "base_seae");
    assert_eq!(se_ae.files[0].destination, "");
    assert!(se_ae.files[0].is_folder);
    assert_eq!(se_ae.files[0].priority, 0);
    assert!(se_ae.type_patterns.is_empty());
    assert!(se_ae.condition_flags.is_empty());
    assert!(se_ae.dependencies.is_none());

    assert_eq!(
        installer.steps[0].groups[0].plugins[1].files[0].source,
        "base_vr"
    );
    assert_eq!(total_flat_plugins(&installer), 2);
}

#[test]
fn sevenz_3step_tk_dodge_details() {
    let (installer, _) = parse_case("sevenz_3step_tk_dodge");

    let reqs = &installer.steps[0].groups[0];
    let names: Vec<&str> = reqs.plugins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["Need it!!", "Pandora", "Animation Motion Revolution"]
    );
    assert!(
        reqs.plugins
            .iter()
            .all(|p| p.r#type == PluginType::Optional)
    );

    // <folder source="meshes" destination="meshes" priority="0" />
    let need_it = &reqs.plugins[0];
    assert_eq!(need_it.files.len(), 1);
    assert_eq!(need_it.files[0].source, "meshes");
    assert_eq!(need_it.files[0].destination, "meshes");
    assert!(need_it.files[0].is_folder);
    assert_eq!(need_it.files[0].priority, 0);

    // Plugins without a <files> node carry no entries.
    assert!(reqs.plugins[1].files.is_empty());
    assert!(reqs.plugins[2].files.is_empty());

    let fun = &installer.steps[2].groups[0].plugins[0];
    assert_eq!(fun.name, "Have FUN!");
    assert!(fun.files.is_empty());
    assert_eq!(total_flat_plugins(&installer), 5);
}

#[test]
fn rar_exactlyone_heel_volume_details() {
    let (installer, _) = parse_case("rar_exactlyone_heel_volume");

    let group = &installer.steps[0].groups[0];
    let names: Vec<&str> = group.plugins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Default Version",
            "Extended Version",
            "Extended Jump Version"
        ]
    );
    assert!(
        group
            .plugins
            .iter()
            .all(|p| p.r#type == PluginType::Optional)
    );

    // <file source="00 Data\Main\Heels Sound Volume.esp"
    //       destination="Heels Sound Volume.esp" priority="0" />
    let default_version = &group.plugins[0];
    assert_eq!(default_version.files.len(), 1);
    let entry = &default_version.files[0];
    assert_eq!(entry.source, "00 data/main/heels sound volume.esp");
    assert_eq!(entry.destination, "heels sound volume.esp");
    assert_eq!(entry.priority, 0);
    assert!(!entry.is_folder);

    assert_eq!(
        group.plugins[2].files[0].source,
        "00 data/mainexjump/heels sound volume.esp"
    );
}

#[test]
fn zip_11step_cbbe_3ba_details() {
    let (installer, _) = parse_case("zip_11step_cbbe_3ba");

    assert_eq!(total_flat_plugins(&installer), 100);
    let starts = compute_flat_starts(&installer);
    assert_eq!(starts.len(), 11);
    assert_eq!(starts[0], vec![0, 5, 7]);
    assert_eq!(starts[4], vec![22, 25, 29, 33, 38, 40, 42]);
    assert_eq!(starts[10], vec![95, 97]);

    // Plugin names in order and their types for step 0, group 0.
    let base = &installer.steps[0].groups[0];
    let names: Vec<&str> = base.plugins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Base install",
            "Nevernude",
            "Underwear",
            "Pre-built Body mesh",
            "ECE Slider compatible"
        ]
    );
    let types: Vec<PluginType> = base.plugins.iter().map(|p| p.r#type).collect();
    assert_eq!(
        types,
        [
            PluginType::Required,
            PluginType::Optional,
            PluginType::Optional,
            PluginType::Recommended,
            PluginType::Optional, // from <dependencyType><defaultType name="Optional"/>
        ]
    );

    // <folder source="00 Base\CalienteTools" destination="CalienteTools"/>
    let base_install = &base.plugins[0];
    assert_eq!(base_install.files.len(), 1);
    assert_eq!(base_install.files[0].source, "00 base/calientetools");
    assert_eq!(base_install.files[0].destination, "calientetools");
    assert!(base_install.files[0].is_folder);

    // Nevernude: a condition flag plus a root-destination folder.
    let nevernude = &base.plugins[1];
    assert_eq!(
        nevernude.condition_flags,
        vec![("NeverNude".to_string(), "Active".to_string())]
    );
    assert_eq!(nevernude.files[0].source, "00 base - nevernude");
    assert_eq!(nevernude.files[0].destination, "");

    // ECE Slider compatible: dependencyType with one type pattern.
    let ece = &base.plugins[4];
    assert_eq!(ece.type_patterns.len(), 1);
    let pattern = &ece.type_patterns[0];
    assert_eq!(pattern.result_type, PluginType::Recommended);
    let cond = &pattern.condition;
    assert_eq!(cond.r#type, FomodConditionType::Composite);
    assert_eq!(cond.op, FomodConditionOp::And);
    assert_eq!(cond.children.len(), 1);
    assert_eq!(cond.children[0].r#type, FomodConditionType::File);
    assert_eq!(cond.children[0].file_path, "EnhancedCharacterEdit.esp");
    assert_eq!(cond.children[0].file_state, "Active");

    // Step 3 visibility: <dependencies operator="And"> with one flag leaf.
    let step3 = &installer.steps[3];
    assert_eq!(step3.name, "CBPC Physics Preset Type");
    let visible: &FomodCondition = step3.visible.as_ref().expect("visible condition");
    assert_eq!(visible.r#type, FomodConditionType::Composite);
    assert_eq!(visible.op, FomodConditionOp::And);
    assert_eq!(visible.children.len(), 1);
    assert_eq!(visible.children[0].r#type, FomodConditionType::Flag);
    assert_eq!(visible.children[0].flag_name, "PageEnable");
    assert_eq!(visible.children[0].flag_value, "On");

    // Step 3 "Select Type" plugins: flag values read via node text.
    let select_type = &step3.groups[0];
    let new_type = &select_type.plugins[0];
    assert_eq!(new_type.name, "New Type (More selectable options)");
    assert_eq!(
        new_type.condition_flags,
        vec![("NewType".to_string(), "On".to_string())]
    );
    assert_eq!(new_type.files.len(), 1);
    assert_eq!(
        new_type.files[0].source,
        "10 physics patch/75 cbpc separation/skse/plugins/cbpconfig_bbp.txt"
    );
    assert_eq!(
        new_type.files[0].destination,
        "skse/plugins/cbpconfig_bbp.txt"
    );
    assert!(!new_type.files[0].is_folder);
    let old_type = &select_type.plugins[1];
    assert_eq!(
        old_type.condition_flags,
        vec![("NewType".to_string(), "Off".to_string())]
    );
    assert!(old_type.files.is_empty());

    // First conditional pattern: And over two flag leaves, one folder entry.
    assert_eq!(installer.conditional_patterns.len(), 97);
    let first = &installer.conditional_patterns[0];
    assert_eq!(first.condition.op, FomodConditionOp::And);
    assert_eq!(first.condition.children.len(), 2);
    assert_eq!(first.condition.children[0].flag_name, "Full");
    assert_eq!(first.condition.children[0].flag_value, "Active");
    assert_eq!(first.condition.children[1].flag_name, "Option1");
    assert_eq!(first.condition.children[1].flag_value, "Active");
    assert_eq!(first.files.len(), 1);
    assert_eq!(first.files[0].source, "10 physics patch/60 3bca full smp");
    assert_eq!(first.files[0].destination, "");
    assert!(first.files[0].is_folder);
}

/// The archive-prefix join is observable in the IR: sources of a prefixed
/// case start with the prefix, mirroring how the engine maps IR sources onto
/// archive entries.
#[test]
fn prefixed_cases_carry_the_prefix_in_every_source() {
    for case in [
        "rar_7step_sos",
        "rar_selectall_cbpc_config",
        "sevenz_atmostone_slavetats_riek",
        "sevenz_selectany_racemenu_plugins",
    ] {
        let (installer, prefix) = parse_case(case);
        assert!(!prefix.is_empty(), "{case} must have a non-empty prefix");
        let all_files = installer
            .required_files
            .iter()
            .chain(installer.steps.iter().flat_map(|s| {
                s.groups
                    .iter()
                    .flat_map(|g| g.plugins.iter().flat_map(|p| p.files.iter()))
            }))
            .chain(
                installer
                    .conditional_patterns
                    .iter()
                    .flat_map(|p| p.files.iter()),
            );
        let mut seen = 0usize;
        for entry in all_files {
            assert!(
                entry.source == prefix || entry.source.starts_with(&format!("{prefix}/")),
                "{case}: source {:?} must start with prefix {prefix:?}",
                entry.source
            );
            seen += 1;
        }
        assert!(seen > 0, "{case}: fixture has file entries");
    }
}
