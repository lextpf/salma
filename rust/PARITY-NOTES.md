# PARITY-NOTES

Running log of behavioral parity decisions and divergences between the Rust
port (`rust/mo2-salma-rs`, DLL `mo2_salma_rs.dll`) and the C++ source of truth
(`src/`, DLL `mo2-salma.dll`). The C++ engine is authoritative; every entry
records where the Rust side matches it and where it intentionally does not yet.

## Task 1 - C ABI skeleton (Milestone 1)

### Exports (must stay exactly these 8, undecorated)

Mirror of `src/CApi.hpp`, same signatures and ownership:

| Symbol                | Return         | Ownership                                  |
| --------------------- | -------------- | ------------------------------------------ |
| `getApiVersion`       | `const char*`  | static, never freed                        |
| `setLogCallback`      | `void`         | -                                          |
| `install`             | `const char*`  | owned, free via `freeResult`               |
| `installWithConfig`   | `const char*`  | owned, free via `freeResult`               |
| `inferFomodSelections`| `const char*`  | owned, free via `freeResult`               |
| `installSucceeded`    | `bool`         | -                                          |
| `freeResult`          | `void`         | consumes an owned pointer                  |
| `resolveModArchive`   | `const char*`  | owned, free via `freeResult`               |

### Confirmed matches with C++

- `getApiVersion` returns the static string `"1.2.0"`. The compile-time guard
  `version[0] == major[0]` in `src/CApi.hpp` is mirrored by a Rust unit test
  and by keeping `MO2_SALMA_API_VERSION` / `MO2_SALMA_API_MAJOR` /
  `API_VERSION_C` in one module, cross-checked by a test.
- Null-input strings are byte-identical to C++:
  - `install` / `installWithConfig`: `"archivePath and modPath must not be null"`.
  - `inferFomodSelections`: `"archivePath and modPath must not be null"`.
  - `resolveModArchive`: `""` when `installationFile` or `modFolder` is null.
- Panic (Rust) maps to C++ `catch (...)`:
  - `install` / `installWithConfig`: `"Unknown fatal error during installation"`,
    success flag cleared.
  - `inferFomodSelections`: `""`.
  - `resolveModArchive`: `""`.
- `installSucceeded` semantics match the C++ mutex-guarded global: last install
  wins, cross-thread visible. Implemented as an `AtomicBool` (SeqCst); the C++
  mutex only ever guarded a bool, so a single atomic is equivalent.
- `freeResult(nullptr)` is a no-op (C++ `free(nullptr)`).
- Empty returns are a valid non-null pointer to `""` (mirrors `_strdup("")`),
  reclaimable by `freeResult`.

### Ownership / allocator note

C++ pairs `_strdup` (malloc family) with `free`. Rust pairs `CString::into_raw`
with `CString::from_raw` (Rust global allocator). Both are internally
consistent because this DLL both allocates and frees the string. Passing a
pointer from a foreign allocator to `freeResult` is undefined behavior, exactly
as in C++ - documented on `freeResult`.

### Intentional divergences (temporary, Milestone 1 stubs)

- `install` / `installWithConfig` with valid inputs return the placeholder
  `"install not yet implemented in mo2_salma_rs"` and set the success flag
  false. C++ returns the real installed mod path. Placeholder removed when the
  install replay is ported (Tasks 14/15).
- `inferFomodSelections` with valid inputs returns `""`. C++ runs the full
  inference pipeline. The `""` placeholder is indistinguishable from the C++
  "no FOMOD / pipeline failure" empty result, so callers are not misled about
  the contract, only about the (not yet implemented) success path. Ported in
  Tasks 4-12.
- `resolveModArchive` with valid inputs returns `""`. C++ runs the archive
  resolution fallback chain. Ported in Task 15.
- `installWithConfig` ignores `jsonPath` in Milestone 1 (the stub does not
  install). C++ coerces null `jsonPath` to `""` and reads selections from it.
- No logger is wired up yet. C++ logs an error line on each caught exception;
  the Rust stubs catch panics silently. `setLogCallback` retains the pointer in
  a lock-free atomic but nothing consumes it until Task 17.

### Build-artifact findings (export table)

- The release DLL exports exactly the 8 required undecorated names.
- `install` and `installWithConfig` share one RVA in the release DLL: MSVC
  identical-code folding (`/OPT:ICF`) merges them because the two stubs compile
  to byte-identical code today. This is harmless and correct - both currently
  do the same thing - and they will un-fold automatically once `installWithConfig`
  starts consuming `jsonPath` (Task 15). Left as-is; no linker tweaks.
- `setLogCallback` stores the callback into `LOG_CALLBACK`, but nothing reads
  that static until the logger lands (Task 17). The release optimizer therefore
  treated the store as a dead write and eliminated it, folding the emptied
  function into the allocator shim (verified via `dumpbin /disasm`: the export
  target was a bare `ret`). A `std::hint::black_box(LOG_CALLBACK.load(...))`
  keeps the value live so the pointer is genuinely retained per the ABI
  contract; the DLL's `setLogCallback` is now a real, distinct function. Remove
  the `black_box` line once a real reader (the logger) exists.

### UTF-8 handling

C++ passes raw `const char*` bytes straight into `std::string` without UTF-8
validation. Rust must decode to `&str` for the (future) service calls, so
invalid UTF-8 on a non-null pointer is treated like a caught exception and
returns the per-export failure value (`"Unknown fatal error during
installation"` for install, `""` for infer/resolve). This is stricter than C++
today but only affects malformed non-UTF-8 paths, which the MO2 plugin never
produces (it encodes paths as UTF-8).

## Task 3 - Utils port

Port of `src/Utils.hpp` + `src/Utils.cpp` to
`rust/mo2-salma-rs/src/utils.rs` (plus `types.rs` for `PluginType` from
`src/Types.hpp`). All 59 TEST()/TEST_F() cases in `tests/utils_test.cpp` are
ported 1:1 into `utils.rs`'s `#[cfg(test)]` module (snake_case names, same
inputs and expected outputs), plus 16 Rust-only tests covering the functions
the C++ suite does not exercise directly.

- `Utils.cpp to_lower` -> `utils::to_lower`: implemented with
  `str::to_ascii_lowercase`, which matches the C++ `unsigned char` +
  `std::tolower` ("C" locale) semantics exactly - only ASCII `A-Z` is
  lowercased, bytes >= 0x80 (all multi-byte UTF-8) pass through unchanged.
  Validated by the 5 ported ToLower tests. One representational difference:
  C++ accepts arbitrary byte strings, Rust requires valid UTF-8 (invalid
  UTF-8 is already rejected at the ABI boundary per Task 1).
- `Utils.cpp normalize_path` -> `utils::normalize_path`: ported from the .cpp
  implementation, same stage order (lowercase, backslash->slash, strip leading
  "./" then leading "/", strip trailing "/", single-pass "//" collapse, drop
  "."/".." segments syntactically). The `.hpp` mermaid pipeline comment was
  checked against the code: no drift, the comment matches the implementation
  order. Validated by the 10 ported NormalizePath tests.
- `Utils.hpp fnv1a_hash` -> `utils::fnv1a_hash`: `pub const fn` (mirrors the
  C++ `constexpr`), offset basis 14695981039346656037, prime 1099511628211,
  u64 wrapping multiply, byte-stream order. Validated three ways: published
  FNV-1a-64 vectors (`""`, `"a"`, `"foobar"`), and a fixture test hashing the
  committed bytes of two golden `ModuleConfig.xml` files
  (`zip_exactlyone_mu_joint_fix` -> `91d7af004eea83d1`,
  `sevenz_3step_tk_dodge` -> `777a83236a2c8541`) against constants computed
  once with `rust/tools/gen_golden.py`'s `fnv1a_hex` reference.
- `FomodCSPPrecompute.cpp hash_combine` -> `utils::hash_combine`: this helper
  lives in the CSP module in C++, not in Utils; it is hosted in `utils.rs` so
  Task 8 can consume it. Exact formula
  `seed ^= v + 0x9e3779b97f4a7c15 + (seed << 6) + (seed >> 2)` with wrapping
  adds. Validated against constants computed once with a python model of the
  C++ expression (single combine from 0, combine from the FNV basis, and a
  chained fnv1a combine).
- `Utils.cpp random_hex_string` -> `utils::random_hex_string`: identical
  output alphabet (`0-9a-f`), exact length, thread-local state. The
  randomness source intentionally differs: C++ uses thread-local
  `std::mt19937` seeded from `std::random_device`; Rust uses a thread-local
  SplitMix64 stream seeded from `std::collections::hash_map::RandomState`
  (OS-seeded std entropy) mixed with the system clock. Chosen over the `rand`
  crate to keep the dependency tree minimal; every C++ call site uses the
  value as a scratch-name/uniqueness token, and both sources are
  non-cryptographic. Rust has no default arguments, so the C++ `length = 12`
  default became `utils::RANDOM_HEX_DEFAULT_LEN`; the ported DefaultLength
  test passes it explicitly. Validated by the 5 ported RandomHexString tests.
- `Utils.cpp parse_plugin_type_string` / `plugin_type_to_string` ->
  `utils::parse_plugin_type_string` / `utils::plugin_type_to_string`, with
  `PluginType` ported to `types.rs`. Lookup miss (including empty string)
  defaults to `PluginType::Optional` exactly as the C++ `enum_map`
  specialization. The C++ `to_string` miss value `"Unknown"` is unreachable
  because every `PluginType` value is in the map; the Rust exhaustive `match`
  encodes that directly (no `"Unknown"` arm needed). Validated by the 7
  ported ParsePluginTypeString tests plus a Rust-only round-trip test.
- `Utils.hpp EnumStringMap` / `HashDispatch` / `operator""_h` /
  `no_hash_collisions` -> not ported 1:1 (deliberate mapping decision): these
  are C++ compile-time dispatch machinery. Rust replaces `EnumStringMap` with
  plain `match` (see previous bullet) and will replace the
  `switch (fnv1a_hash(...))` + `"..."_h` pattern in `FomodIRParser.cpp` with
  `match` on `&str` in Task 4, which needs no collision checker.
  `fnv1a_hash` stays `const fn` so hash constants remain expressible if ever
  needed. Validated by a const-eval test.
- `Utils.cpp normalize_destination_for_join` ->
  `utils::normalize_destination_for_join`: same sequential strip order (all
  leading `/`/`\` first, then repeated `./`/`.\` prefixes). Quirk replicated
  exactly and pinned by a test: because the loops do not alternate, an input
  like `.//foo` returns `/foo` (the `./` strip re-exposes a slash that the
  already-finished slash loop never sees). Not "fixed" in Rust; in the C++
  pipeline downstream callers (`FomodIRParser`) re-normalize with
  `normalize_path`, so behavior parity requires keeping the quirk.
- `Utils.cpp resolve_file_destination` -> `utils::resolve_file_destination`:
  exact semantics - `<file>` with empty destination installs to the source
  filename (`find_last_of("/\\")` -> `rfind(['/', '\\'])`), `<file>` with a
  trailing `/` or `\` destination appends the source filename, `<folder>`
  destinations pass through (empty stays empty), result goes through
  `normalize_destination_for_join`. Validated by Rust-only tests (the C++
  suite has none for this function); Task 5 atom expansion consumes it.
- `Utils.cpp is_safe_destination` -> `utils::is_safe_destination`: identical
  rules including the empty-input and normalizes-to-empty accepts, the
  leading-`/` reject (unreachable after `normalize_path`, ported verbatim),
  and the `norm[1] == ':'` drive-letter reject. Validated by Rust-only tests.
- `Utils.cpp is_safe_mod_name` -> `utils::is_safe_mod_name`: identical rule
  order (empty; leading/trailing whitespace; separators; absolute path;
  "."/".."; trailing "."; reserved device stem CON PRN AUX NUL COM1-9 LPT1-9
  on the lowercase stem before the final "."). The whitespace check uses a
  custom C-locale `isspace` set (space, \t, \n, \v, \f, \r) because Rust's
  `is_ascii_whitespace` omits `\v` (0x0B). The `Path::is_absolute` check is
  defensive dead code in both languages (anything absolute contains a
  separator, which is rejected earlier); kept for parity. Validated by the 20
  ported IsSafeModName tests.
- `Utils.cpp is_inside` -> `utils::is_inside`: C++ uses
  `std::filesystem::weakly_canonical` on both paths (errors -> false) then
  `lexically_relative`, requiring non-empty and not starting with `..`. Rust
  std has no weakly_canonical, so `utils::weakly_canonical` implements the
  MSVC strategy: try `fs::canonicalize` on the full path, then on
  progressively shorter leading prefixes; the first success is canonicalized
  and the nonexistent tail is appended and lexically normalized
  (`utils::lexically_normal`, a port of the C++ `lexically_normal` rules).
  Canonicalize errors other than NotFound/NotADirectory propagate and map to
  `false`, mirroring the C++ error-code path. Because both weakly-canonical
  results are in lexically normal form, the `lexically_relative` + `..` check
  reduces to `Path::starts_with` (component-wise); the C++ `child == parent`
  case yields `.` and passes, so equality is "inside" in both. Known
  representational notes: Rust canonicalize returns `\\?\`-prefixed paths on
  Windows (consistently on both sides of the comparison, so containment is
  unaffected), and trailing-separator preservation differs (irrelevant to the
  component-wise comparison). Validated by the ported containment test plus
  Rust-only tests (equal paths, sibling-with-common-prefix, nonexistent-tail
  and fully-lexical weakly_canonical cases).
- `Utils.cpp executable_directory` / `module_directory` ->
  `utils::executable_directory` / `utils::module_directory`, using
  `windows-sys` 0.60 (features `Win32_Foundation`,
  `Win32_System_LibraryLoader` only). `module_path_for` ports the C++ helper
  exactly: `GetModuleFileNameW` with a MAX_PATH buffer, doubling retry loop
  capped at 5, discard on `len == 0` or retries exhausted, parent directory
  of the module file. `executable_directory` queries a null module (the C++
  `GetModuleFileNameW(nullptr, ...)` path) rather than
  `std::env::current_exe`, for exact parity. `module_directory` uses
  `GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, anchor, ...)` like
  the C++. One divergence: the C++ fallback `std::filesystem::current_path()`
  throws on failure; the Rust fallback maps that practically unreachable
  failure to `"."` instead of panicking. Validated by Rust-only tests
  (directory exists; `module_directory` of a local symbol equals
  `executable_directory` in a statically linked test binary). Task 17 (logs
  next to the DLL) will exercise the DLL-module case for real.
- `Utils.cpp get_ordered_nodes` -> `utils::get_ordered_nodes` +
  `utils::parse_node_order` + `utils::NodeOrder`: ported generically (the C++
  is pugixml-typed; Task 4 wires the XML crate). Exact C++ branch structure
  preserved: missing `order` attribute defaults to `"Ascending"`;
  `"Descending"`/`"Ascending"` sort by name; anything else - including
  `"Explicit"`, unknown values, and casing mismatches - keeps document order.
  Comparators are byte-wise lexicographic in both (`std::string` traits
  compare as unsigned char; Rust `str::cmp` compares bytes). C++ uses
  `std::ranges::sort` (unstable); Rust uses `sort_unstable_by`, so relative
  order of equal names is unspecified in BOTH implementations - any future
  golden mismatch on FOMODs with duplicate step/group/plugin names within one
  parent could stem from this and is not a Rust regression. Validated by the
  5 ported GetOrderedNodes tests (all with distinct names, as in C++).
- `Utils.cpp xml_bool_attribute_true` -> `utils::xml_bool_attribute_true`:
  ported as `Option<&str>` (None = missing/null attribute -> false;
  `Some(value)` -> lowercase equals `"true"` or `"1"`). Task 4 adapts the XML
  crate's attribute type to this. Validated by the 6 ported XmlBoolAttribute
  tests.
- Test-mapping note: the C++ `GetOrderedNodesTest` / `XmlBoolAttributeTest`
  fixtures parse XML documents via pugixml; the ported Rust tests feed the
  generic equivalents the same data (document-ordered `(name, doc_index)`
  pairs, pre-read attribute values as `Option<&str>`) and assert the same
  expected outputs. The containment test uses a differently named temp dir
  (`salma_rs_modname_containment_test`) so the Rust and C++ suites cannot
  collide when run concurrently; inputs are otherwise identical.
- `Utils.hpp Result<T>` (`std::expected<T, std::string>` alias) -> not
  ported; Rust's built-in `Result<T, String>` fills this role when the first
  consumer lands.

## Task 4 - FomodIR + XML parser

Port of `src/FomodIR.hpp` to `rust/mo2-salma-rs/src/fomod_ir.rs` and
`src/FomodIRParser.hpp`/`.cpp` to `rust/mo2-salma-rs/src/fomod_ir_parser.rs`,
including the pugixml document-load semantics the C++ callers rely on
(`doc.load_buffer` with DEFAULT options at `src/FomodInferenceService.cpp:908`,
`doc.load_file` at `src/InstallationService.cpp:316`). The reference pugixml
is the vcpkg-pinned 1.15 (`buildtrees/pugixml/src/v1.15-...`); every pugixml
behavior below was read from that exact source, not from docs.

### XML crate choice

- roxmltree 0.21.1 (current release; pinned as `roxmltree = "0.21"` +
  Cargo.lock). Chosen per the port plan: read-only DOM with document-order
  children, byte ranges into the input (needed to reconstruct pugixml's
  pcdata/CDATA node boundaries), no unsafe, one transitive dep (memchr). The
  Document borrows the input string, so `parse_module_config(bytes, prefix)`
  keeps the borrow internal and returns an owned `FomodInstaller`.

### API mapping

- C++ `pugi::xml_document::load_buffer(bytes)` (fallible) ->
  `fomod_ir_parser::decode_xml_bytes` (encoding autodetect + transcode + BOM
  strip) followed by `fomod_ir_parser::load_document` (roxmltree parse), both
  fallible via `FomodXmlError`. C++ `FomodIRParser::parse(doc, prefix)`
  (infallible, empty installer when no `<config>` root) ->
  `fomod_ir_parser::parse(&doc, prefix)`. Convenience composition:
  `fomod_ir_parser::parse_module_config(bytes, prefix)`. Callers arrive in
  Tasks 12/15. Validated by the micro-test suite and 15 golden fixtures.

### IR structs (`FomodIR.hpp` -> `fomod_ir.rs`)

- All structs/enums ported with identical field names (C++ `type` becomes
  Rust `r#type`), variant sets, and defaults: condition Composite/And,
  priority 0, is_folder/always_install/install_if_usable false, plugin type
  Optional, group type SelectAny, step ordinal 0. Derives: Debug + Clone +
  Default + PartialEq. Validated by dedicated default-value tests.
- `types.rs PluginType` gained `#[derive(Default)]` (`Optional`), mirroring
  both the C++ `enum_map<PluginType>` default and the `FomodIR.hpp` field
  initializers; no other Task 3 code touched.
- `FomodIR.hpp total_flat_plugins` / `compute_flat_starts` ->
  `fomod_ir::total_flat_plugins` / `fomod_ir::compute_flat_starts`, `i32`
  like the C++ `int`. Validated by synthetic-shape tests plus
  `zip_11step_cbbe_3ba` (100 plugins; spot-checked start rows).
- `FomodIR.hpp enum_map<FomodConditionOp>` / `enum_map<FomodGroupType>` ->
  `fomod_ir::parse_condition_op` / `parse_group_type` (+ `..._to_string`).
  Read from `Utils.hpp EnumStringMap::from_string`: EXACT case-sensitive
  match over the entry table, miss (including empty string) returns the map
  default (`And` / `SelectAny`). Validated by exact-match, case-mismatch,
  and garbage-input tests.

### Document loading / encoding

- `pugixml guess_buffer_encoding` (pugixml.cpp:2025) ->
  `fomod_ir_parser::guess_buffer_encoding`, an exact port including check
  order: buffers < 4 bytes skip detection (UTF-8); UTF-32 BE/LE BOM outranks
  the UTF-16 LE BOM prefix; UTF-16 BE/LE BOM; UTF-8 BOM; `<` byte-pattern
  probes for BOM-less UTF-32/UTF-16 (`3C 00` / `00 3C` etc.); then pugixml
  1.15's declaration probe (`parse_declaration_encoding`, ported exactly with
  its ct_space/ct_symbol tables) which only recognizes "ISO-8859-1"/"latin1"
  (case-insensitive letters) -> latin1; everything else UTF-8. The declared
  encoding is otherwise IGNORED (a `UTF-16` declaration on UTF-8 bytes parses
  as UTF-8), matching encoding_auto. Validated by a probe-table unit test
  mirroring the C++ branch order and an identical-parse test across UTF-8,
  UTF-8 BOM, UTF-16LE/BE (BOM and BOM-less), and UTF-32LE/BE.
- Corpus scan (committed `rust/tests/golden/cases/*/ModuleConfig.xml`, 15
  files; the gitignored `full/` tree has no ModuleConfig.xml on this machine,
  only JSON fixtures): 11 UTF-16 LE with BOM (`FF FE`), 2 UTF-8 with BOM
  (sevenz_4step_lewdmarks, sevenz_9step_the_pure), 2 UTF-8 without BOM
  (rar_7step_sos, zip_exactlyone_racecompat). All parse through the byte
  pipeline in the fixture suite.
- BOM handling matches pugixml `parse_skip_bom`: the BOM scalar is stripped
  from the decoded text before parsing (pugixml strips it post-conversion).
  Validated by `bom_scalar_is_stripped_before_parsing`.
- Odd trailing byte in UTF-16 input is dropped (pugixml converts
  `size / sizeof(u16)` units); UTF-32 likewise via `chunks_exact`.
- Intentional strictness divergence: invalid UTF-8, lone UTF-16 surrogates,
  and out-of-range UTF-32 units are `FomodXmlError::Decode` errors, where
  pugixml silently passes through / drops mangled bytes. Rust strings cannot
  represent them; consistent with the Task 1 ABI decision. Latin1 decodes
  byte -> U+00XX exactly like pugixml's latin1 conversion.
- Malformed-input acceptance is NOT symmetric: pugixml 1.15 with DEFAULT
  options ACCEPTS a range of non-well-formed documents (its escape decoder
  cancels on unmatched entity references and leaves them literal,
  pugixml.cpp:2501-2637; there is no single-root rule - status_no_document_element
  only fires when NO element exists, pugixml.cpp:3612; comments only scan for
  the first `-->`; there is no character validation), so `load_buffer`
  SUCCEEDS and `FomodInferenceService.cpp:908` proceeds to build an IR and
  infer. An earlier revision of this note claimed both sides failed such
  loads "at the same boundary" - that was false (review finding, Task 4
  review pass) and the handling is now split three ways:
  - Recovered by `pugixml_lenient_pass` (runs between decode and parse, so
    the port accepts these like C++ and produces the same IR): bare `&` and
    unknown/undeclared entity references in text or attribute values
    (pugixml leaves them literal; the pass rewrites the `&` to `&amp;`,
    which decodes back to the same literal - `name="Body & Soul"` and
    `a&nbsp;b` now parse identically to C++), character references the two
    parsers do not agree on (`&#X41;`, `&#;`, `&#65a;`, `&lt` without the
    semicolon - literal on both sides now, exactly as pugixml cancels),
    references to entities declared in an internal DTD (pugixml skips the
    DOCTYPE and keeps `&name;` literal; neutralizing the reference BEFORE
    roxmltree sees it removes the expansion divergence the previous
    revision of this note accepted), `--` or a trailing `-` inside
    comments (pugixml only scans for the first `-->` and drops comments
    from the tree, so blanking the body is unobservable), a stray `]]>`
    in character data (pugixml's pcdata scanner only stops at `<`/`&`/`\r`,
    strconv_pcdata pugixml.cpp:2712-2742, so C++ carries the literal
    `]]>` into IR values; the pass rewrites it to `]]&gt;`, which decodes
    back to the same text - pinned by
    `stray_cdata_terminator_stays_literal_like_pugixml`), and XML
    declarations wherever pugixml would skip them (default options have
    parse_declaration and parse_pi both off, so parse_question's skip
    branch, pugixml.cpp:3283-3289, just scans for the first `?>` with no
    position or grammar check; roxmltree hard-fails a declaration preceded
    by whitespace or a comment, lacking `version`, or sitting mid-document.
    The pass rewrites every `<?xml` + whitespace span to a `<!-- -->`
    placeholder, which is equally absent from the pugixml tree and splits
    pcdata runs exactly like the skipped span; other `<?...?>` targets are
    valid roxmltree PIs and copied verbatim - pinned by
    `xml_declarations_are_skipped_like_pugixml`). The agreed
    reference set was derived from pugixml `strconv_escape` and roxmltree
    `consume_reference` side by side: five predefined named entities, plus
    `&#`/`&#x` (lowercase `x` only, both sides) char refs whose value is an
    XML-valid char.
  - Still rejected by the port where C++ would parse and infer (ACCEPTED
    divergences, each pinned by an `accepted_divergence_*` unit test,
    except non-UTF-8 bytes which `invalid_utf8_is_a_decode_error` pins):
    multiple root elements (pugixml keeps them as siblings and
    `doc.child("config")` finds the config root; roxmltree hard-fails), raw
    control characters in text (pugixml passes the bytes through into IR
    values; roxmltree rejects NonXmlChar), non-UTF-8 bytes under a guessed
    UTF-8 encoding (pugixml mangles; Rust `Decode` error, per the Task 1
    ABI decision - the most likely real-world hit in this whole inventory
    is a windows-1252-encoded ModuleConfig.xml with high bytes:
    `guess_buffer_encoding` recognizes only ISO-8859-1/latin1,
    pugixml.cpp:2053-2068, so a declared `windows-1252` falls through to
    UTF-8, C++ passes the raw 0x80-0xFF bytes into IR values as MOJIBAKE,
    and exact parity is unattainable because Rust strings cannot hold
    those byte sequences; the port declines instead), undeclared
    namespace prefixes such as
    `xsi:noNamespaceSchemaLocation` without `xmlns:xsi` (pugixml is
    namespace-unaware; roxmltree resolves prefixes and fails), duplicate
    attributes on one element (pugixml keeps both, `attribute()` returns
    the first; roxmltree fails), and raw `<` inside attribute values
    (pugixml scans to the closing quote; roxmltree rejects). Consequence
    once wired (Tasks 12/15): for such archives the C++ DLL can infer while
    the Rust port returns no-inference. Recorded as a conscious
    safety-over-leniency call on inputs that are broken XML rather than
    routine tool output; revisit if round-trip testing (Task 16) surfaces
    real archives in these classes.
  - Rejected by the port for a MEMORY-SAFETY bound where C++ parses:
    element nesting deeper than `MAX_ELEMENT_DEPTH` (48). pugixml 1.15's
    `xml_parser::parse` is an iterative cursor loop with no per-depth
    recursion, so `doc.load_buffer` at `FomodInferenceService.cpp:908`
    succeeds on arbitrarily deep documents; roxmltree 0.21.1 recurses once
    per nesting level inside `Document::parse` and aborts the process with
    an uncatchable STATUS_STACK_OVERFLOW (not a Rust panic - the
    `catch_unwind` at the C ABI boundary cannot contain it). Measured on
    the pinned roxmltree (default 1 MiB Windows main-thread stack): debug
    parses total depth 65, dies at 81; release dies around 2000 (~14 KB
    file). `load_document` therefore runs `element_depth_exceeds` (an
    iterative, quote-aware scan that skips comments/CDATA/DOCTYPE/PIs)
    BEFORE roxmltree and returns `FomodXmlError::TooDeep` instead. 48
    covers every meaningful FOMOD shape (~8 structural levels plus the
    32-level `MAX_DEPENDENCY_DEPTH` ceiling on nested `<dependencies>`,
    which C++ compiles to an always-false Or beyond 32 but still parses);
    the deepest parity test document is 37. Pinned by
    `element_depth_at_the_limit_still_parses`,
    `element_depth_beyond_the_limit_is_an_error_not_a_stack_overflow`, and
    `depth_guard_counts_only_real_element_nesting`.
  - Rejected by BOTH parsers (true shared failure boundary): unclosed tags,
    unterminated comments/CDATA/PIs.
  - Residual reference divergence inside the recovered class: character
    references to XML-invalid code points (`&#1;`, `&#x0;`, `&#xD800;`)
    stay literal in the port, while pugixml emits the raw scalar (control
    bytes or mangled UTF-8) into the value. Neither side of that trade can
    match the other exactly (Rust strings cannot hold mangled bytes);
    literal was chosen as representable and lossless, pinned by
    `mixed_valid_and_invalid_references_decode_like_pugixml`. The same
    residual class includes references whose numeric value overflows u32:
    pugixml `strconv_escape` accumulates `16 * ucsc + digit` (or `10 *`)
    in an unsigned int with wraparound, pugixml.cpp:2509-2554, so
    `&#x100000041;` (0x41 mod 2^32) decodes to `A` and `&#4294967341;`
    (45 mod 2^32) to `-` in C++, while roxmltree rejects the reference;
    the port keeps the literal text, pinned by
    `overflowing_char_refs_stay_literal_not_pugixml_wraparound`.
  DTDs: `allow_dtd = true` so a benign DOCTYPE does not hard-fail (pugixml
  skips doctype); with references neutralized by the lenient pass, roxmltree
  entity EXPANSION can no longer occur, so DTD-declared entities now match
  pugixml (literal `&name;`), pinned by
  `dtd_declared_entities_stay_literal_like_pugixml`.

### Parser semantics (`FomodIRParser.cpp parse` -> `fomod_ir_parser::parse`)

- `doc.child("config")` -> first element child of the document root named
  `config`; missing -> default (empty) installer. Leading comments are
  transparent (not in the pugixml tree; skipped by the Rust lookup).
  Validated by `missing_config_root_...` and `config_root_is_found_past_...`.
- Name matching: pugixml compares RAW qualified names ("pfx:tag"), roxmltree
  namespace-aware local names. The port matches on the local name and ignores
  the namespace, which is identical to pugixml for every un-prefixed element
  (all real FOMOD configs; the schema is noNamespaceSchemaLocation) and for
  default-namespaced documents. Divergence only for explicitly prefixed
  condition/config elements, which pugixml would reject by raw-name mismatch
  while this port matches their local name; accepted, documented here.
- moduleDependencies (`FomodIRParser.cpp:198`): prefer `<dependencies>`
  child; else compile from `moduleDependencies` itself if it has ANY pugixml
  first child; else absent. The pugixml tree drops comments/PIs and
  whitespace-only pcdata but keeps non-ws pcdata and CDATA -
  `pugi_has_first_child` reproduces this over roxmltree (including
  roxmltree's merging of adjacent pcdata/CDATA runs, detected via node byte
  ranges). Validated by 4 micro-tests incl. whitespace-only and text-only
  content.
- `compile_condition` (`FomodIRParser.cpp:52`): operator attr
  `as_string("And")` -> `parse_condition_op` (miss -> And); depth > 32
  (`MAX_DEPENDENCY_DEPTH`, mirrored from `FomodDependencyEvaluator.hpp:16`
  as a `pub const` in `fomod_ir_parser.rs` until Task 5 re-homes it) returns
  Composite/op=Or with NO children (always-false); more than 10000 element
  children truncate, and the counter increments BEFORE dispatch so unknown
  (skipped) elements consume cap slots exactly as in C++; only element
  children counted; leaf kinds flagDependency, fileDependency (state default
  "Active"), gameDependency, pluginDependency (type default "Active"),
  fomodDependency, fommDependency, foseDependency, nested dependencies;
  unknown elements skipped. The C++ `switch` on `fnv1a_hash(name)` becomes a
  `match` on `&str` (no collision checker needed, per the Task 3 mapping
  decision). Validated by micro-tests: all leaves + defaults, unknown-skip,
  depth-33 -> empty Or (and depth-32 still compiles), 10005-children
  truncation built programmatically, unknown-elements-count-toward-cap,
  text-children-not-counted.
- `parse_file_entry` (`FomodIRParser.cpp:145`): destination attribute
  PRESENT (even empty) is used verbatim; ABSENT falls back to the source
  attribute value. `is_folder` = element name == "folder". `full_source` =
  prefix empty ? source : prefix + "/" + source, then Task 3
  `utils::normalize_path`. Folder destination = `normalize_path(dest_raw)`;
  file destination =
  `normalize_path(resolve_file_destination(source, dest_raw, true))` (Task 3
  port reused, not duplicated). Validated by micro-tests (present-empty vs
  absent destination for file AND folder, trailing-slash destination, prefix
  join) and by fixture entries (mu_joint_fix has a real
  `folder source="base" destination=""` required file).
- `priority = node.attribute("priority").as_int(0)`: the task brief describes
  as_int as plain strtol-base-10, but the pinned pugixml 1.15 implements
  `string_to_integer<unsigned int>` (pugixml.cpp:4570) which additionally
  accepts a `0x`/`0X` hex prefix and SATURATES to INT_MIN/INT_MAX on
  overflow (it matches strtol only up to there). `pugi_as_int` ports the
  1.15 code exactly: ct_space skip, single optional sign, hex path (overflow
  = more than 8 significant hex digits), decimal path (overflow heuristic on
  digit count + lead digit + high bit), negative clamp at 0x80000000.
  Missing attribute -> 0. Validated by `priority_follows_pugixml_as_int_...`
  ("12abc" -> 12, "abc" -> 0, missing -> 0, hex, INT_MAX/INT_MIN edges) and
  `pugi_as_int_edge_cases` (wrap-around decimals, 9-hex-digit overflow,
  leading zeros).
- `always_install` / `install_if_usable` via Task 3
  `utils::xml_bool_attribute_true` (absent -> false; "true"/"1"
  case-insensitive). Validated by micro-test.
- `for_each_file_node`: only element children named exactly "file"/"folder"
  with a non-empty `source` attribute -> `file_nodes` iterator. Validated by
  micro-test (missing source, empty source, wrong element name, interleaved
  text all filtered).
- Step/group/plugin ordering (`Utils.cpp get_ordered_nodes`): the `order`
  attribute is read from the PARENT (`installSteps`, `optionalFileGroups`,
  `plugins`); "Descending"/"Ascending" sort byte-wise by `name` attribute,
  anything else (missing attr defaults to the string "Ascending"; "Explicit"
  and garbage/casing mismatches) keeps document order. Wired to roxmltree via
  the generic Task 3 `utils::get_ordered_nodes` (unstable sort on both sides,
  as documented in Task 3 - equal names remain an accepted nondeterminism
  site). A missing parent behaves like pugixml's null node: empty list.
  Validated by Explicit/Ascending/Descending/missing/garbage micro-tests and
  by zip_exactlyone_racecompat, whose group order is genuinely re-sorted
  (missing order attr -> Ascending differs from document order there).
- `step.ordinal` is assigned AFTER ordering (zero-based index into the
  ordered sequence), `FomodIRParser.cpp:218`. Validated by
  `ordinals_are_assigned_after_ordering` (Ascending re-sort).
- `step.visible` (`FomodIRParser.cpp:226`): prefer `visible/dependencies`;
  else compile from `visible` itself when it has any pugixml first child;
  empty or whitespace-only `<visible>` stays absent. Validated by 3
  micro-tests.
- typeDescriptor (`FomodIRParser.cpp:254`): `type` child wins
  (`name` attr default "Optional" via Task 3 `parse_plugin_type_string`,
  which also maps present-but-unknown/empty names to Optional); else
  `dependencyType`: `defaultType` name -> base type (absent leaves
  Optional); each `patterns/pattern` gets its condition from a
  `dependencies` child (ABSENT -> default-constructed condition =
  Composite/And, no children = always-true) and `result_type` from its
  `type` child (absent node leaves Optional). Validated by 5 micro-tests and
  the cbbe_3ba "ECE Slider compatible" fixture plugin (defaultType Optional +
  one Recommended pattern on a fileDependency).
- conditionFlags (`FomodIRParser.cpp:310`): flag name from `name` attr;
  EMPTY NAME -> entry skipped entirely; empty VALUE kept. The value is
  pugixml `node.text().as_string()` = the FIRST pcdata/cdata child of the
  pugixml tree, NOT concatenated mixed content, scanning past element
  children, with whitespace-only pcdata absent from that tree.
  roxmltree MERGES directly adjacent pcdata/CDATA runs into one text node,
  so `pugi_text` reconstructs pugixml's node boundaries from the document
  byte ranges: the common no-CDATA case uses roxmltree's decoded text
  directly; runs involving CDATA walk the raw chunks (pcdata chunks decoded
  with a local parse_escapes+parse_eol equivalent, CDATA chunks taken raw
  with EOL normalization, ws-only pcdata chunks dropped, first surviving
  chunk wins). CRITICAL boundary rule (review findings 1/7): pugixml decides
  whether to DROP a ws-only pcdata run on the RAW source chars BEFORE escape
  expansion (`PUGI_IMPL_SKIPWS` stops at the `&` of a character reference,
  pugixml.cpp:3476-3496), so raw `&#32;` / `&#13;&#10;` IS a pcdata node
  whose decoded value is whitespace - `is_ws_only` is therefore applied to
  the raw input byte range (`Document::input_text` + `Node::range`), never
  to the decoded text, in both `text_node_exists_in_pugi_tree` and
  `pugi_text` (common and merged-run paths). This matters in the wild: the
  FOMOD Creation Tool emits `&#13;&#10;` character references in text
  content. Validated by 10 micro-tests: first-text-with-element-between,
  element-first-then-text, ws-only -> "", empty name skipped / empty value
  kept, four CDATA adjacency layouts, entity decoding on both paths, and the
  charref-whitespace family (`&#32;` flag value " ", moduleDependencies /
  visible presence, merged pcdata+CDATA runs). A second review fix in the
  merged-run decoder: EOL normalization is applied to the LITERAL segments
  only, so a reference-produced CR (`&#13;`) survives as `\r` exactly as in
  pugixml and in roxmltree's own decoding. Known residual divergence: none
  found for inputs the port accepts (the DTD-entity exception recorded here
  earlier is gone - the lenient pass neutralizes those references, see the
  document-loading section).
- conditionalFileInstalls/patterns/pattern (`FomodIRParser.cpp:335`):
  condition from `dependencies` child (absent -> always-true default), files
  via the file-node filter; missing `<patterns>` yields none. Validated by
  micro-tests and the cbbe_3ba (97 patterns) / lewdmarks (10) /
  racecompat (2) fixtures.
- Plugin `dependencies` child -> `plugin.dependencies = Some(...)`,
  `FomodIRParser.cpp:323`. Validated by micro-test.
- Logging: C++ logs warnings (unknown condition elements, depth/breadth
  truncation) through `Logger`. Decision: emit NOTHING in Rust until the
  Task 17 logger lands - no logging dependency, no stub state. Log output is
  not part of golden parity; the code sites carry comments marking where the
  Task 17 logger hooks in.
- The C++ `static_assert(no_hash_collisions(...))` guarding the fnv1a switch
  has no Rust counterpart because the `match` on strings cannot collide;
  recorded as intentionally not ported.

### Fixture tests (`tests/fomod_ir_fixtures.rs`)

- All 15 committed `rust/tests/golden/cases/*/ModuleConfig.xml` parse through
  `parse_module_config` with the case's REAL archive prefix, derived from
  case.json `module_config_entry` exactly as
  `FomodInferenceService.cpp:878-882` does (normalize, strip the
  `fomod/moduleconfig.xml` suffix and its joining slash); 4 cases have
  non-empty prefixes and a dedicated test asserts every IR source carries
  the prefix. Asserted for every fixture: step count, step names in order,
  ordinals, per-step visible presence, per-step group count, group names in
  order, group types, per-group plugin count, required-file count,
  conditional-pattern count, moduleDependencies absence. Detail fixtures
  (zip_exactlyone_mu_joint_fix, sevenz_3step_tk_dodge,
  rar_exactlyone_heel_volume, and zip_11step_cbbe_3ba as the largest):
  plugin names in order, plugin types (incl. Required/Recommended and a
  dependencyType default), FomodFileEntry field values
  (source/destination/priority/is_folder incl. present-empty destination),
  condition tree shape for a step visibility, a type pattern, and the first
  conditional pattern, plus flat-index cross-checks (cbbe_3ba: 100 plugins).
- Expected-value derivation: read from the fixture XMLs themselves (UTF-16
  fixtures transcoded for reading), applying the C++ ordering rules; the
  full (step name, group name, plugin name-set) sequence of ALL 15 fixtures
  was cross-checked against each case's `expected.json` (authoritative C++
  DLL output) with an independent scripted comparison before the Rust
  expectations were written - all 15 matched - and step counts/group types
  agree with `case.json`. The scripted model was only used to enumerate and
  cross-check; the assertion literals for the detail fixtures were verified
  against the raw XML by hand.
- Review finding 6 hardening: the one-off scripted cross-check is now a
  permanent test.
  `every_fixture_matches_expected_json_step_group_plugin_names` re-reads
  each case's `expected.json` at test time (via a dependency-free mini JSON
  reader in the test file) and asserts, for every fixture, the ordered step
  names, ordered group names, and the ordered PLUGIN names of every group:
  the C++ assembler walks the IR in order and splits each group's plugins
  into `plugins` (selected) and `deselected`, so the test interleaves the
  two lists back together and requires them to reproduce exactly the parsed
  IR's plugin sequence. Plugin-name content of all 15 fixtures (not just
  the 4 detail fixtures) is therefore guarded against the authoritative C++
  output, closing the counts-only gap in the expectation table.

### Test-suite delta

- 95 new tests: 12 in `fomod_ir.rs`, 76 in `fomod_ir_parser.rs` (every
  parity trap a-k from the task brief has at least one dedicated micro-test,
  plus the encoding matrix, the charref-whitespace family, the lenient-pass
  parity cases, the accepted-divergence pins, and the element-depth guard
  trio), 7 in `tests/fomod_ir_fixtures.rs`.

### Review findings and fixes (Task 4 review pass)

Eight review findings against the Task 4 working tree; disposition of each,
naming the C++ ground truth, the Rust site, and the validation used.

- Finding 1 + 7 (critical/major, duplicate reports): ws-only pcdata
  existence must be decided on RAW source chars, not decoded text. C++: the
  pugixml tree drop rule (pugixml.cpp:3476-3496) behind
  `FomodIRParser.cpp` `first_child()` / `text()` consumers
  (moduleDependencies :204, visible :232, conditionFlags :316). Rust:
  `fomod_ir_parser.rs` `is_ws_only` callers `text_node_exists_in_pugi_tree`
  and `pugi_text`, which test `Document::input_text()[Node::range()]`.
  Validation: new `charref_whitespace_*` tests (`&#32;` flag value " ",
  `&#13;&#10;` value "\r\n", moduleDependencies/visible present-but-
  childless, merged pcdata+CDATA runs); all pass. While pinning the merged
  path, one adjacent real bug was found and fixed: `decode_pcdata_chunk`
  ran `normalize_eol` over the WHOLE decoded chunk, turning a
  reference-produced `\r` (`&#13;`) into `\n`; it now normalizes literal
  segments only, matching pugixml and roxmltree.
- Finding 2 (critical): documents pugixml accepts hard-failed the Rust
  load, and this file justified that with a false same-boundary claim. C++:
  `pugi::xml_document::load_buffer` default-options tolerance
  (pugixml.cpp:2501-2637 escape cancel, :3612 no single-root rule) feeding
  `FomodInferenceService.cpp:908`. Rust: new `pugixml_lenient_pass` +
  `agreed_reference_len` + `doctype_len` in `fomod_ir_parser.rs`, applied
  by `parse_module_config` between decode and parse. Validation: unit tests
  for bare `&` in attributes, unknown entities, malformed references,
  `--` comments, CDATA/comment/DOCTYPE verbatim copying; remaining
  rejection classes pinned by `accepted_divergence_*` tests; the corrected
  acceptance decision is recorded in the document-loading section above.
- Finding 3 (minor): `allow_dtd` let roxmltree EXPAND internal-DTD
  entities where pugixml leaves `&name;` literal. C++: pugixml doctype
  skip + escape-decoder cancel. Rust: fixed as a byproduct of the finding-2
  lenient pass (references neutralized before parse). Validation:
  `dtd_declared_entities_stay_literal_like_pugixml` (was a divergence, now
  parity); the old accepted-divergence note was removed.
- Finding 4 (minor): local-name matching parses prefixed documents that
  pugixml (raw qualified-name comparison) would reduce to an empty
  installer. Already documented as accepted (real FOMOD schemas are
  noNamespaceSchemaLocation); now also pinned by
  `accepted_divergence_prefixed_elements_match_local_names` so the boundary
  cannot drift silently. No behavior change.
- Finding 5 (major): undeclared namespace prefixes and duplicate
  attributes - roxmltree rejects, pugixml accepts - were neither documented
  nor tested. C++: pugixml namespace-unawareness and first-wins
  `attribute()` lookup behind `FomodIRParser.cpp` `attr_or`. Rust:
  unchanged behavior (still rejected; a lenient rewrite would need real
  tag-level parsing), now documented in the still-rejected inventory above
  and pinned by `accepted_divergence_undeclared_namespace_prefix_fails` and
  `accepted_divergence_duplicate_attributes_fail`.
- Finding 6 (minor): the fixture table asserted plugin COUNTS only for 11
  of 15 fixtures. Fixed with the permanent `expected.json` cross-check test
  described in the fixture section above; ordered plugin names of every
  group in all 15 fixtures are now compared against the C++ DLL output at
  test time.
- Finding 8 (minor): the "no residual divergence except DTD entities"
  claim in the conditionFlags section was falsified by the
  charref-whitespace divergence. The claim has been rewritten to match the
  now-validated behavior (raw-range existence checks, lenient-pass entity
  handling), and both former exceptions are covered by tests instead of
  prose promises.

### Review findings and fixes (Task 4 second review pass)

Six findings against the reviewed Task 4 working tree; disposition of each,
naming the C++ ground truth, the Rust site, and the validation used.

- Finding 1 (critical): a stray `]]>` in character data hard-failed the
  Rust load while pugixml builds a full IR. C++: `strconv_pcdata`
  (pugixml.cpp:2712-2742, scanner stops only at `<`/`&`/`\r`) behind
  `pugi::xml_document::load_buffer` at `FomodInferenceService.cpp:908`.
  Rust: `fomod_ir_parser.rs pugixml_lenient_pass`, new `b']'` arm that
  rewrites `]]>` outside CDATA/comments/DOCTYPE/PIs to `]]&gt;`, which
  decodes back to the same literal text. Validation:
  `stray_cdata_terminator_stays_literal_like_pugixml` (text value,
  pcdata-existence content, attribute value, and a real CDATA terminator
  left untouched); recovered-class inventory updated above.
- Finding 2 (major): XML declarations pugixml skips (whitespace before
  the declaration, missing `version`) failed the Rust load. C++:
  `parse_question` skip branch (pugixml.cpp:3283-3289; with default
  options parse_declaration and parse_pi are off, so any `<?...?>` is
  skipped by scanning for `?>` with no position/grammar validation).
  Rust: `pugixml_lenient_pass` now rewrites every `<?xml` + whitespace
  span (up to the first `?>`, matching pugixml's span boundary) to a
  `<!-- -->` placeholder - equally absent from the pugixml tree and
  splitting pcdata runs exactly like the skipped span; non-declaration
  `<?...?>` targets stay verbatim (valid roxmltree PIs). Valid
  declarations are rewritten too, avoiding a replica of roxmltree's
  declaration grammar. Validation:
  `xml_declarations_are_skipped_like_pugixml` (leading whitespace,
  missing version, mid-document, after a comment, mid-text pcdata split,
  verbatim `<?xml-stylesheet`/`<?XML` PIs) plus a roxmltree 0.21.1 probe
  run confirming which variants roxmltree rejects vs parses as PIs; the
  15-fixture suite exercises the rewrite on every real declaration.
- Finding 3 (major): the documented accepted strictness divergences
  (multiple roots, raw control chars, non-UTF-8 bytes, undeclared
  prefixes, duplicate attributes, raw `<` in attributes) remain. No
  behavior change - the finding itself notes each class is deliberate,
  pinned, and scheduled for the Task 16 revisit. The named most likely
  real-world hit (windows-1252 bytes: pugixml.cpp:2053-2068 recognizes
  only ISO-8859-1/latin1, C++ emits mojibake IR values that Rust strings
  cannot represent, per the Task 1 ABI decision) is now spelled out in
  the still-rejected inventory above so Task 16 knows what to look for.
- Finding 4 (minor): u32-overflowing character references (`&#x100000041;`
  wraps to `A` in pugixml, stays literal here) were missing from the
  residual-divergence inventory. C++: `strconv_escape` unsigned wraparound
  (pugixml.cpp:2509-2554). Rust: unchanged behavior
  (`agreed_reference_len` neutralizes on `u32::from_str_radix` overflow;
  roxmltree rejects the reference, so there is no agreed decoding to
  preserve); the wraparound sub-case is now named in the residual
  paragraph above, in the `agreed_reference_len` comments, and pinned by
  `overflowing_char_refs_stay_literal_not_pugixml_wraparound`.
- Finding 5 (minor): duplicate of finding 3 at class level (documented,
  test-pinned strictness divergences persist). Same disposition: accepted
  boundary, no code change, inventory strengthened, Task 16 revisit plan
  unchanged.
- Finding 6 (minor): the accepted-divergence inventory was incomplete -
  literal `]]>` in text, whitespace before the declaration, and a
  mid-document declaration were neither documented nor pinned. Resolved
  by RECOVERING all three classes instead of documenting them as
  rejections (findings 1 and 2 above); they are now listed in the
  recovered-class inventory and pinned by the two new parity tests, so
  the still-rejected list is complete again.

### Review findings and fixes (Task 4 third review pass)

Two findings against the twice-reviewed Task 4 working tree; disposition of
each, naming the C++ ground truth, the Rust site, and the validation used.

- Finding 1 (major): re-report of the accepted acceptance-divergence class
  (windows-1252/non-UTF-8 bytes, undeclared namespace prefixes, duplicate
  attributes, raw `<` in attribute values, raw control chars, multiple
  roots), kept open so the boundary stays visible. C++ ground truth:
  `pugi::xml_document::load_buffer` default-options tolerance feeding
  `FomodInferenceService.cpp:908` (pugixml validates neither UTF-8 nor
  namespaces, so `FomodIRParser::parse` builds a full IR, with raw mojibake
  bytes in string values for the cp1252 case). Rust site:
  `fomod_ir_parser.rs decode_xml_bytes` / `load_document` - behavior
  intentionally unchanged; exact byte parity for the mojibake case is
  unattainable because Rust strings cannot hold invalid UTF-8 (Task 1 ABI
  decision), and the remaining classes stay safety-over-leniency rejections
  pending the Task 16 corpus revisit. Fix applied: the finding's named
  most-likely real-world input (declared `windows-1252`, byte 0x92 inside a
  plugin name) is now pinned by its own test,
  `accepted_divergence_windows_1252_bytes_fail_decode`, alongside the other
  `accepted_divergence_*` pins, so the one class where a wild archive can
  regress from infer to no-inference is guarded by an exact-input test, not
  only by the generic `invalid_utf8_is_a_decode_error`.
- Finding 2 (critical): deeply nested XML aborted the process with an
  uncatchable stack overflow inside `roxmltree::Document::parse`, where the
  C++ pipeline parses the same bytes; the divergence was untested and
  missing from this inventory. C++ ground truth: pugixml 1.15
  `xml_parser::parse` (iterative cursor loop, no per-depth recursion)
  behind `doc.load_buffer` at `FomodInferenceService.cpp:908`; the only
  C++ recursion, `compile_condition_impl`, is guarded at
  `MAX_DEPENDENCY_DEPTH` = 32 (`FomodIRParser.cpp:57`). Rust site: new
  `pub const MAX_ELEMENT_DEPTH: usize = 48` and `element_depth_exceeds`
  (iterative single pass, quote-aware inside tags, skipping
  comments/CDATA/DOCTYPE/PIs; unterminated constructs fall through to
  roxmltree's own error) enforced by `load_document` BEFORE roxmltree runs,
  failing as the new `FomodXmlError::TooDeep`; the misleading "depth
  prevents stack overflow" comment on the condition guards was rewritten
  to name what each bound actually protects. Bound calibration: scratch
  probe against the pinned roxmltree 0.21.1 on this machine - debug build,
  default 1 MiB main-thread stack, prints before/after parse and
  `mem::forget` to exclude Drop - parses total depth 65 and dies inside
  `Document::parse` at depth 81 (release around 2000), reproducing the
  reviewer's numbers; 48 sits below the shallowest measured overflow with
  margin while covering every meaningful FOMOD shape (deepest existing
  parity test: 37). Recorded above as a new bounded-depth entry in the
  acceptance inventory (the "still-rejected list is complete" claim from
  the second pass was wrong until then). Validation:
  `element_depth_at_the_limit_still_parses` (real roxmltree recursion at
  the bound, in debug, passes),
  `element_depth_beyond_the_limit_is_an_error_not_a_stack_overflow`
  (depth 49 and the previously process-killing ~2000-deep document now
  return `TooDeep`), and `depth_guard_counts_only_real_element_nesting`
  (comment/CDATA/PI bodies, DOCTYPE subset, quoted `>` in attribute
  values, siblings, and self-closing elements add no depth).
