# PARITY-NOTES

Running log of behavioral parity decisions and divergences between the Rust
port (the crate, DLL `mo2_salma_rs.dll`) and the C++ source of truth
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

RESOLVED as of Task 15. Every stub in this list is gone; the entries are kept
so the history reads correctly, each annotated with where it was closed.

- ~~`install` / `installWithConfig` with valid inputs return the placeholder
  `"install not yet implemented in mo2_salma_rs"` and set the success flag
  false.~~ CLOSED in Task 15: both exports call
  `installation_service::InstallationService::install_mod` and return the
  installed mod path, setting the flag true on any `Ok`.
- ~~`inferFomodSelections` with valid inputs returns `""`.~~ CLOSED in Task 12:
  `capi.rs` constructs `FomodInferenceService` and runs the real pipeline.
  (This bullet was already stale before Task 15; see the Task 12 section.)
- ~~`resolveModArchive` with valid inputs returns `""`.~~ CLOSED in Task 15:
  wired to `archive_resolver::resolve_mod_archive`.
- ~~`installWithConfig` ignores `jsonPath`.~~ CLOSED in Task 15: `jsonPath` is
  borrowed, null-coerced to `""` per `CApi.cpp:86`, and forwarded to
  `install_mod`.
- STILL OPEN: no logger is wired up. C++ logs an error line on each caught
  exception; the Rust exports fail silently. `setLogCallback` retains the
  pointer in a lock-free atomic but nothing consumes it until Task 17.

### Build-artifact findings (export table)

- The release DLL exports exactly the 8 required undecorated names.
- `install` and `installWithConfig` share one RVA in the release DLL: MSVC
  identical-code folding (`/OPT:ICF`) merges them because the two stubs compile
  to byte-identical code today. This is harmless and correct - both currently
  do the same thing - and they will un-fold automatically once `installWithConfig`
  starts consuming `jsonPath` (Task 15). Left as-is; no linker tweaks.
  RESOLVED in Task 15: `installWithConfig` now borrows and forwards `jsonPath`,
  so the two bodies differ and the fold is gone (re-verify with `dumpbin
  /exports` after any release rebuild).
- `installSucceeded` was ALSO folded away in the stub era, for the same reason
  and worse: because `set_last_install_success` was only ever called with
  `false`, LLVM proved the `AtomicBool` could never be true, constant-folded the
  load, and ICF merged the emptied body into an unrelated `sevenz_rust2` symbol
  (`dumpbin /disasm` showed literally `xor eax,eax; ret`). Found by the Task 15
  pre-audit, NOT by any test. Task 15 fixes it by construction - the success
  path now stores `true` - but the lesson generalizes: an export whose only
  observable value is a compile-time constant can be optimized into a shared
  stub, and the export table alone will not reveal it. Re-check with `dumpbin`
  whenever an export's logic is stubbed out.
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
`src/utils.rs` (plus `types.rs` for `PluginType` from
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
  once with `tools/gen_golden.py`'s `fnv1a_hex` reference.
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

Port of `src/FomodIR.hpp` to `src/fomod_ir.rs` and
`src/FomodIRParser.hpp`/`.cpp` to `src/fomod_ir_parser.rs`,
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
- Corpus scan (committed `tests/golden/cases/*/ModuleConfig.xml`, 15
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
  (`MAX_DEPENDENCY_DEPTH`, mirrored from `FomodDependencyEvaluator.hpp:16`;
  it lived in `fomod_ir_parser.rs` during Task 4 and Task 5 re-homed it to
  `fomod_dependency_evaluator.rs`, matching the C++ header that owns it - the
  parser now imports it from there) returns
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

- All 15 committed `tests/golden/cases/*/ModuleConfig.xml` parse through
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

## Task 5 - Dependency evaluator + atom expansion

Port of `src/FomodDependencyEvaluator.hpp`/`.cpp` to
`src/fomod_dependency_evaluator.rs`, `src/FomodAtom.hpp` to
`fomod_atom.rs`, and `src/FomodInferenceAtoms.hpp`/`.cpp` to
`fomod_inference_atoms.rs`, plus `FomodDependencyContext` (`src/Types.hpp:85`)
added to `types.rs`. 67 new tests: 38 evaluator micro-tests, 4 atom-datatype
tests, 17 expansion micro-tests, 8 fixture integration tests
(`tests/fomod_atoms_fixtures.rs`).

### Scoping decision: assemble_json deferred to Task 10

`FomodInferenceAtoms.cpp` also contains `assemble_json` and its
anonymous-namespace helpers (`build_plugin_object`, `lookup_plugin_diag`,
`lookup_group_diag`, `lookup_step_diag`). They depend on `SolverResult`
(Task 8/9) and `InferenceDiagnostics` (Task 10) and are therefore ported in
Task 10, not here. Everything else in the file - `is_safe_dest`,
`expand_entry`, `expand_all_atoms`, `build_atom_index`,
`compute_excluded_dests`, and `build_target_tree` - is ported now.

### API mapping (C++ -> Rust)

- `evaluate_condition(cond, flags, const FomodDependencyContext*)` ->
  `fomod_dependency_evaluator::evaluate_condition(&FomodCondition,
  &HashMap<String, String>, Option<&FomodDependencyContext>)`. The C++
  default argument `context = nullptr` becomes an explicit `None`.
- `evaluate_condition_inferred(cond, flags, override, ctx)` ->
  `evaluate_condition_inferred(..., ExternalConditionOverride,
  Option<&FomodDependencyContext>)`. The ctx parameter is kept for signature
  parity and named `_context`: the C++ `LeafEvaluator<Inferred>` stores ctx
  and NEVER reads it, so inferred mode ignores any context entirely (pinned
  by `inferred_mode_ignores_context_entirely`).
- `evaluate_plugin_type(plugin, flags, ctx)` -> same name. The deliberate
  ctx asymmetry is replicated exactly: `Some(ctx)` evaluates each type
  pattern in NORMAL mode, `None` evaluates it in INFERRED mode with the
  `Unknown` override (so external leaves are false). The CSP callers all
  pass nullptr/None and always get inferred-Unknown semantics. The Some
  branch is pinned by
  `plugin_type_ctx_presence_selects_normal_vs_inferred_unknown`; the None
  branch (inferred dispatch, NOT Normal-with-null-ctx) is pinned by
  `plugin_type_none_ctx_dispatches_inferred_not_normal_with_null_ctx` via a
  state="Missing"/"Inactive" File leaf, the only leaf shape whose value
  differs between the two dispatches without a context.
- `ExternalConditionOverride` -> same name, `#[repr(u8)]`,
  Unknown=0/ForceFalse=1/ForceTrue=2 (pinned by a repr test), plus
  `#[derive(Default)]` = Unknown (the C++ default at every call site).
- `MAX_DEPENDENCY_DEPTH` re-homed from `fomod_ir_parser.rs` (its temporary
  Task 4 location) to `fomod_dependency_evaluator.rs`, matching
  `FomodDependencyEvaluator.hpp:16`; the parser imports it from there. The
  Task 4 bullet above was updated.
- The C++ `LeafEvaluator<EvalMode>` template becomes two private leaf
  functions (`eval_leaf_normal`, `eval_leaf_inferred`) selected by the
  public entry points; `evaluate_condition_core` takes a
  `&dyn Fn(&FomodCondition) -> bool`. Compile-time vs runtime dispatch is
  unobservable; behavior is identical.
- `eval_plugin_dep` returns plain `bool` in Rust: the C++ `PluginDepResult`
  struct carries `is_active`/`file_exists` members that NO caller reads
  (only `.met` is consumed, `FomodDependencyEvaluator.cpp:267`); porting the
  dead fields would only trip dead-code lints.
- `FomodAtom`/`TargetFile`/`AtomIndex`/`TargetTree`/`ExpandedAtoms` ->
  `fomod_atom.rs`, identical fields and defaults (`plugin_index` /
  `conditional_index` default to -1, so `Default` is hand-implemented; enum
  `FomodAtom::Origin` becomes module-level `Origin` since Rust has no nested
  enum). The two C++ `for_each` overloads (const/mutable template) become
  `for_each(&self, impl FnMut(&FomodAtom))` and `for_each_mut`, preserving
  the iteration order required -> per_plugin (flat order) -> per_conditional.
- `expand_entry`/`expand_all_atoms`/`build_atom_index`/
  `compute_excluded_dests`/`build_target_tree`/`is_safe_dest` ->
  `fomod_inference_atoms.rs`, same names. `std::lower_bound` over the sorted
  entry list becomes `slice::partition_point` (identical first->=prefix
  semantics, including the empty-prefix "match everything" identity).
- `FomodDependencyContext` -> `types.rs`, `HashSet<String>` fields with the
  C++ field-semantics comments (installed_files normalized lowercase
  forward-slash; installed_plugins lowercase; installed_fomods matched
  case-sensitively - the asymmetry with plugins is intentional and pinned by
  `fomod_dep_matches_exactly_without_lowercasing`).

### C++ fs::path semantics helpers

Rust `Path::extension()`/`file_name()` have different edge rules than MSVC
`fs::path` (`extension()` here EXCLUDES the dot; `.gitignore` has extension
"gitignore" in Rust but none in C++; `"file."` has none in Rust but "." in
C++), so the evaluator uses two private helpers instead of `std::path`:

- `cpp_path_filename`: everything after the last '/' or '\\' (both are
  separators on Windows); empty for a trailing separator. When NO separator
  is present, a leading drive root-name is stripped, matching MSVC's
  decomposition: "C:foo.esp" -> "foo.esp", "C:" -> "" (the drive prefix is
  exactly one ASCII letter + ':', so "CC:foo" and "1:foo" are returned
  whole). Root-names only exist at the start of a path, so the strip never
  applies after a separator ("dir/C:bar" -> "C:bar", as in MSVC). MSVC's UNC
  root-name rule is also replicated: EXACTLY two leading separators followed
  by a non-separator extend the root-name to the next separator, so when no
  further separator exists the whole path is the root-name and filename() is
  "" ("//server.esp", "\\\\server", "\\\\?", "//C:" -> ""). With a further
  separator the generic last-separator rule already agrees with MSVC
  ("//server/share.esp" -> "share.esp"), three or more leading separators
  form no root-name ("///foo" -> "foo"), and the device prefixes
  ("\\\\?\\", "\\\\.\\", "\\??\\") need no dedicated parsing for filename
  purposes because they always carry a separator at index 3
  ("\\\\?\\foo" -> "foo" either way).
- `cpp_path_extension`: INCLUDES the leading dot; "." and ".." have no
  extension; a filename whose only dot is leading (".gitignore") has no
  extension; "file." has extension ".". Via the root-name strip, "C:.esp"
  has NO extension (filename ".esp", leading-dot-only), matching MSVC.

Pinned by `cpp_path_helpers_mirror_msvc_fs_path`; every drive/UNC/device
expectation in that test was verified against an MSVC 2022 fs::path probe
program. Only ".esp"/".esm"/".esl" comparisons consume these in practice,
but the edge rules are locked so the helper can be reused verbatim later.

History: round 1 shipped without the UNC rule, justified as "unreachable
because every filename() call is gated on a plugin extension". Round 2
review showed that argument is circular (the extension gate is computed by
the same helper) and the divergence WAS reachable: the C++ parser stores
file_path raw (`FomodIRParser.cpp:94`), so a crafted
`<fileDependency file="//server.esp"/>` reaches `eval_file_dep`, where MSVC
sees filename ""/extension "" (no installed_plugins fallback, non-plugin
"Inactive" semantics) while the old helper saw "server.esp"/".esp".
Implementing the two-leading-separator rule removed the divergence;
`file_dep_unc_root_name_is_not_a_plugin` pins the evaluator-level behavior
("Active" -> false and "Inactive" -> true for an active plugin named by a
UNC-root-name-only path, fallback restored below a real UNC share).

### Version parsing: std::getline semantics

`parse_version_parts` replicates the C++ strip-then-split exactly, including
the getline-on-'.' quirk: a TRAILING '.' produces NO trailing empty token
("1.2." -> [1,2,0] after padding), while consecutive dots and a leading dot
DO produce empty tokens ("1..2" -> [1,0,2], ".1" -> [0,1,0]). Empty tokens
and i32-overflow tokens map to 0 (`std::stoi` throw -> catch -> 0 in C++;
`parse::<i32>().unwrap_or(0)` here; C++ logs a warning per malformed token,
dropped until Task 17). The cleaned string keeps only ASCII digits and '.'
(C++ `isdigit` is ASCII under the "C" locale). Tail-pad to length 3.
`compare_version_parts` iterates to max(len) with missing components read as
0, so "1.2" == "1.2.0" < "1.2.1". Pinned by `parse_version_parts_table`
(includes "v1.2.3-beta", "1.2.3.99 (custom)", "99999999999999999999" -> 0)
and `compare_version_parts_pads_missing_with_zero`. The FOMM check compares
required <= actual against the hardcoded "0.13.21"
(`fomm_dep_boundary_at_hardcoded_version` pins the exact boundary).

### Evaluator behaviors replicated (each test-pinned)

- Composite: empty And -> true, empty Or -> false (this is how the parser's
  depth-truncation bail, an empty Or, is always-false, and a
  pattern-without-dependencies, an empty And, is always-true);
  short-circuiting; depth guard `depth > 32` with depth incremented only
  when recursing into a Composite child (innermost node at depth 32 still
  evaluates, at 33 is unmet).
- Flag leaves are handled in the shared core BEFORE leaf dispatch, so they
  evaluate identically in both modes: missing flag -> `flag_value.empty()`,
  present -> exact case-sensitive equality, no trimming.
- File dep (Normal mode): empty path -> false; existence checked in order
  installed_files (normalized path) -> installed_plugins (lowercased
  filename, only for .esp/.esm/.esl of the ORIGINAL path) -> archive_root
  join -> game_path join (both probes non-throwing; `Path::exists()` matches
  the C++ `fs::exists(p, ec)` error->false semantics); no ctx -> not
  existing. States are case-sensitive literals: "Missing" -> !exists;
  "Inactive" -> for existing plugin files !active, else !exists; anything
  else (unknown strings log a warning in C++, dropped until Task 17) ->
  treated as "Active" -> exists.
- Plugin dep: empty name -> false; active = lowercased name in
  installed_plugins; file_exists fallback probes `game_path/Data/<RAW name>`
  (raw, not lowered, as in the C++ join; exercised by
  `plugin_dep_inactive_uses_game_data_dir_with_raw_name` but NOT pinned:
  NTFS is case-insensitive, so a lowered-name probe would still hit the same
  file and no test can observe the difference on the only supported
  platform); type "Inactive" -> exists && !active; ANY other type string
  (including default "Active" and garbage) -> active.
- Fomod dep: exact case-sensitive name match, no ctx -> false.
- Game dep: no ctx or empty game_path -> true (standalone); version compare
  only when both required and ctx.game_version are non-empty.
- Fose: unconditionally true in BOTH modes (the C++ returns true before the
  mode branch); the unreachable default switch arm is also true.
- Inferred mode: File/Plugin/Fomod -> `override == ForceTrue` (Unknown and
  ForceFalse both false); Game/Fomm/Fose -> true. Full 3x6 matrix pinned by
  `inferred_mode_override_matrix`.

### Atom expansion behaviors replicated (each test-pinned)

- Folder branch: prefix = source, "/"-anchored unless empty (path-boundary:
  "foo" does not match "foobar.esp"; empty source matches every entry via
  the lower_bound("")/starts_with("") identities); dest = raw
  `destination + "/" + rel` concatenation (or bare rel when destination is
  empty) BEFORE `normalize_path`; top-level `meta.ini` (exact
  post-normalization match) skipped; unsafe destinations skipped;
  source_path is the sorted entry string AS-IS; file_size from the sizes map
  (missing -> 0); content_hash 0.
- File branch: meta.ini check on `normalize_path(destination)` but BOTH the
  is_safe_dest check and the stored dest_path use the RAW
  `entry.destination` unchanged (the parser already normalized it; pinned by
  `file_branch_keeps_destination_unchanged`).
- Unsafe-destination reality check: the task brief's example "../evil" is
  actually SAFE - `is_safe_destination` normalizes first and
  `normalize_path` strips ".." segments, so "../evil" -> "evil". After
  normalization the only reachable unsafe class is a drive-letter/absolute
  path; the skip tests use "C:/evil" and the "../evil" acceptance is pinned
  explicitly. This matches C++ exactly (same normalize-first order).
- `expand_all_atoms`: doc_order increments once per file-ENTRY expansion
  call (all atoms of one folder entry share one document_order; an entry
  matching ZERO archive files still consumes a slot - pinned by cbbe_3ba's
  pattern 95/96). Pass order required -> normal plugin entries -> auto
  (always_install/install_if_usable) plugin entries with flat_idx recomputed
  from 0 -> conditional patterns; per_plugin sized from
  `total_flat_plugins` BEFORE the walk; per_conditional sized from the
  pattern count.
- `build_atom_index` preserves the for_each order within each destination's
  Vec (required first, then plugins by flat index, then conditionals -
  pinned by the racecompat contested-destination test).
- `compute_excluded_dests` truth table: Required-origin atoms set
  all_auto=false (intentional - lets the solver detect incomplete installs);
  normal plugin atoms set all_auto=false; conditional atoms set
  has_conditional and do NOT touch all_auto; excluded iff !has_conditional
  && all_auto && distinct sources <= 1.
- `build_target_tree` skips the exact key "meta.ini" only; TargetFile hash
  starts 0.

### No-logging-until-Task-17 sites (C++ log_warning calls silently dropped)

- `FomodDependencyEvaluator.cpp:69` malformed version component (per token).
- `FomodDependencyEvaluator.cpp:157` unknown file dependency state.
- `FomodDependencyEvaluator.cpp:329` condition tree exceeds maximum depth.
- `FomodInferenceAtoms.cpp:70` and `:105` unsafe atom destination (folder
  and file branches).

Each site carries a source comment naming the dropped C++ log line.

### Fixture-test derivation record (tests/fomod_atoms_fixtures.rs)

- Input prep replicates `FomodInferenceService.cpp:833-886`:
  sorted_norm_entries skips trailing-"/"/"\\" directory markers, normalizes
  (keeping post-normalization duplicates), byte-wise sorts;
  norm_entry_sizes is keyed by normalized path with values looked up by the
  ORIGINAL path, last-write-wins on collisions (synthetic pin:
  `prep_entries_skips_dir_markers_and_last_write_wins`). The fomod prefix is
  derived from the archive listing with the exact is_candidate boundary
  (`== "fomod/moduleconfig.xml"` or `.ends_with("/fomod/moduleconfig.xml")`,
  so "xfomod/..." is not a candidate), shallowest-by-'/'-count first, ties
  by shorter length keeping the first otherwise, then the suffix and its
  joining slash are stripped (synthetic pin:
  `derive_prefix_boundary_and_shallowest_rules`; cross-check against the
  case.json `module_config_entry` derivation for all 15 fixtures).
- Expected counts (per-origin atom counts, atom_index size, excluded size
  for all 15 fixtures) were derived with a throwaway scratchpad Python
  script (NOT committed) that mirrors the C++ rules - normalize_path,
  resolve_file_destination, parse_file_entry, get_ordered_nodes ordering,
  the 3-pass expansion, index/exclusion - directly from each fixture's
  ModuleConfig.xml + archive_entries.json. Three diverse fixtures were then
  HAND-VERIFIED against the raw XML and archive entries before freezing the
  literals: zip_exactlyone_mu_joint_fix (all 8 atoms checked by hand),
  zip_exactlyone_racecompat (group Ascending re-sort, flat indices 0-9, the
  full doc_order sequence 0-25, priorities 0/1/2/3, the
  destination-fallback-to-source readme file entry), and zip_11step_cbbe_3ba
  (stress: 100 plugins, 97 conditional patterns; independent grep-level
  counts confirmed 123 plugin-side entries -> first conditional at doc 123,
  35 archive files under the plugin-0 folder, and pattern 95's
  zero-match folder consuming doc 221 so pattern 96 lands on 222).
  Sample dest-mapping assertions (source, dest, priority, document_order,
  origin, plugin_index, conditional_index, file_size) cover ~5 atoms in each
  of the three fixtures, mixing folder-expanded and single-file entries.
- excluded_dests is 0 for every fixture because NO committed fixture uses
  alwaysInstall/installIfUsable (grep-verified over the corpus); the
  exclusion truth table is therefore exercised synthetically in the
  `compute_excluded_dests_truth_table` unit test, and the fixture suite only
  guards the zero case. Flagged for Task 16's wider corpus.
- Reachability property test: every destination in target_tree.json (except
  MO2's meta.ini) must be present in the atom index. Holds for 14 of 15
  fixtures. DOCUMENTED EXCEPTION, investigated before touching anything:
  rar_exactlyone_heel_volume's archive contains only 8 entries (3 esp
  variants + fomod metadata + screenshots), while its installed mod folder
  holds 36 files including 34 base-mod .wav files and a readme that are NOT
  in the archive at all - the FOMOD patch was installed into an existing mod
  folder. This is a fixture-data property, not an implementation divergence:
  the authoritative C++ output (expected.json ->
  diagnostics.repro.missing/reproduced) records exactly 35 missing / 1
  reproduced for this case, and the test asserts THOSE numbers (read from
  expected.json at test time) instead of weakening the property.

### Accepted divergences (Task 5)

- None behavioral. Representational only: `Option<&ctx>` for nullable
  pointers, `bool` instead of the dead-field `PluginDepResult`, two leaf
  functions instead of the `LeafEvaluator` template, `for_each`/`for_each_mut`
  instead of const/non-const overloads, and dropped log lines (Task 17), all
  documented above.

## Task 6 - Forward simulator + repro metrics

Ported `src/FomodForwardSimulator.hpp`/`.cpp` and, from `src/FomodCSPSolver.cpp`,
`compare_trees_impl`/`compare_trees`/`collect_mismatched_dests`, plus the two
`FomodCSPTypes.hpp`/`FomodCSPSolver.hpp` datatypes the simulator needs
(`ReproMetrics`, `InferenceOverrides`).

### API mapping

| C++ (mo2core)                        | Rust                                                  |
|--------------------------------------|-------------------------------------------------------|
| `struct SimulatedTree`               | `fomod_forward_simulator::SimulatedTree`              |
| `simulate(...)`                      | `fomod_forward_simulator::simulate`                   |
| `simulate_into(...)`                 | `fomod_forward_simulator::simulate_into`              |
| `compare_trees_impl<..>(...)` (tmpl) | `compare_trees_impl(.., impl FnMut(&str)->bool x3)`   |
| `compare_trees(...)`                 | `compare_trees` (always-true predicate wrapper)       |
| `collect_mismatched_dests(...)`      | `collect_mismatched_dests -> Vec<String>` (sorted)    |
| `struct ReproMetrics`                | `fomod_csp_types::ReproMetrics` (five `i32`)          |
| `struct InferenceOverrides`          | `fomod_csp_types::InferenceOverrides`                 |

- The nullable `const FomodDependencyContext*` / `const InferenceOverrides*`
  become `Option<&_>`. `const std::vector<...>&` selections become
  `&[Vec<Vec<bool>>]`.
- `compare_trees_impl`'s three C++ template predicates become three
  `impl FnMut(&str) -> bool` parameters. Task 9's `lower_bound` MUST reuse this
  generic function for its predicate-gated variant rather than reimplement the
  else-chain (the task's stated constraint).
- `InferenceOverrides` is declared in `FomodCSPSolver.hpp` in C++, not
  `FomodCSPTypes.hpp`; the Rust port places it in `fomod_csp_types` with a doc
  note so the simulator can consume it without the full solver header. Task 8
  EXTENDS `fomod_csp_types` with the rest of `FomodCSPTypes.hpp`.

### Phase-2 / phase-3 split and why visibility is re-evaluated

The simulator runs four phases (required -> selected/Required-typed plugins ->
auto atoms of unselected plugins -> conditional installs), mirroring
`FomodService::process_optional_files`'s Pass 1 / Pass 3 split. Phase 2 evaluates
`evaluate_plugin_type` and step visibility against the flag state accumulated
SO FAR (the C++ phase-2 comment: "eff_type is evaluated against the flag state at
this plugin's position ... matches the real installer, which detects
Required-type plugins per step using flags accumulated up to that step"). Phase 3
recomputes `flat_idx` from 0 and RE-EVALUATES step visibility against the now
FINAL flag map (the C++ phase-3 comment: "evaluate eff_type against the FINAL
flag state and apply auto atoms accordingly"). Because the flag map grows during
phase 2, the same step's visibility can differ between phase 2 and phase 3, so
the port does NOT cache visibility across phases; `compute_step_visibility` reads
`flags` at call time in both passes. Both directions of the flip are covered by
tests (`phase3_step_becomes_visible_...`, `phase3_step_becomes_invisible_...`).

### The `>=` overwrite rule vs `execute_file_operations` stable-sort

`should_overwrite(existing, new) = new.priority >= existing.priority` (note `>=`,
so among equal priorities the LATER-applied atom wins). This is equivalent to the
real installer `FomodService::execute_file_operations`, which collects ALL file
operations then `std::stable_sort`s them by `(priority ascending, document_order
ascending)` and applies in that order so the LAST write to a destination wins
(verified against `src/FomodService.cpp:686-698`). The equivalence holds for two
reasons. (1) For DISTINCT priorities the `>=` test makes the maximum-priority atom
win irrespective of application order: a lower-priority atom fails `>=` and cannot
displace a higher-priority incumbent, and a higher-priority atom always passes,
which matches the installer applying the highest-priority op last. (2) For EQUAL
priorities `>=` lets the later-applied atom win, and the simulator applies atoms
in increasing `document_order` - required (lowest doc range) then plugin normal
then plugin auto then conditional (highest), exactly the enqueue order
`expand_all_atoms` assigns (Task 5) and the phase sequence 1->2->3->4 - so the
last-applied equal-priority atom is the one with the greatest `document_order`,
which is precisely the tail of the installer's stable-sorted run for that
destination. Hence identical winners. (Caveat, inherited verbatim from the C++
simulator and therefore replicated, not a divergence: the phase-2/phase-3 split
applies a selected plugin's atoms in phase 2 and an unselected plugin's auto
atoms in phase 3, so for a same-destination EQUAL-priority conflict between a
selected atom and an unselected auto atom the phase-3 atom is applied later and
wins even if its document_order is lower than the installer's global sort would
pick. This does not manifest in any of the 15 fixtures - all reproduce the C++
`outputTree` winning sources exactly.)

### compare_trees else-chain

Per destination in `target` (skipping `excluded`): absent from sim ->
`missing`; else IF `target.size != 0 && atom.file_size != 0 && sizes differ` ->
`size_mismatch` (and the hash check is SUPPRESSED); else IF `target.hash != 0 &&
atom.content_hash != 0 && hashes differ` -> `hash_mismatch`; else `reproduced`.
A zero size or zero hash on either side falls through toward `reproduced`. The
second loop counts sim destinations absent from `target` (skipping `excluded`) as
`extra`. Each mismatch increment is gated by the corresponding predicate. The
port keeps the nested `if predicate { ++ }` inside the size/hash else-chain
(clippy does not flag it because the outer `if/else if/else` carries an else);
this is byte-faithful to the C++. `ReproMetrics::exact()` ignores `reproduced`;
`better_than` is the lexicographic `(missing, extra, size_mismatch,
hash_mismatch)` ascending then `reproduced` descending, all-equal -> false.

### collect_mismatched_dests ordering decision

The C++ collects missing/size-mismatch/hash-mismatch/extra destinations into an
`unordered_set` (unspecified order) then `std::sort`s the resulting vector before
returning. The Rust port mirrors this: collect into a `HashSet<String>`, then
`.into_iter().collect::<Vec<_>>()` and `.sort()`, yielding the same deterministic
byte-wise ascending order. Return type `Vec<String>` (not a set) matches the C++
`std::vector<std::string>`. The only Task 9 consumer, `groups_for_mismatches`,
iterates the result and looks up `dest_to_groups`; it depends on determinism, not
on any particular order, so the sorted vector is safe. Task 9 may re-home these
three functions next to the solver; if so it MUST reuse the generic
`compare_trees_impl` for its `lower_bound` predicate variant.

### Fixture oracle: overrides = None, context = None (trap (n))

The C++ golden runs scored candidates via `evaluate_candidate` ->
`simulate_into(..., context=nullptr, overrides=real)` where the overrides came
from `compute_overrides` (`FomodInferenceService.cpp:1126/1311`, Task 12, NOT
ported). Those overrides affect ONLY step-visibility conditions and
conditional-install patterns, and ONLY when those conditions contain
external-dependency leaves. `evaluate_plugin_type` takes a context, not overrides,
and the C++ golden simulate passed `context=nullptr`, so plugin type_patterns are
evaluated in inferred-Unknown mode in BOTH the golden run and the Rust fixture
run regardless of overrides (no divergence there).

Per-fixture scan of all 15 committed `ModuleConfig.xml` for external-dependency
leaves (`gameDependency`/`fileDependency`/`pluginDependency`/`fomodDependency`/
`fommDependency`/`foseDependency`) inside `<installStep>/<visible>` blocks and
`<conditionalFileInstalls>/<pattern>/<dependencies>`:

| fixture                          | steps | cond patterns | visible ext | conditional ext |
|----------------------------------|-------|---------------|-------------|-----------------|
| rar_7step_sos                    | 7     | 0             | none        | none            |
| rar_exactlyone_heel_volume       | 1     | 0             | none        | none            |
| rar_selectall_cbpc_config        | 1     | 0             | none        | none            |
| sevenz_2step_nec_feet            | 2     | 0             | none        | none            |
| sevenz_3step_tk_dodge            | 3     | 0             | none        | none            |
| sevenz_3step_yorha_patches       | 3     | 0             | none        | none            |
| sevenz_4step_lewdmarks           | 4     | 10            | none        | none            |
| sevenz_9step_the_pure            | 9     | 0             | none        | none            |
| sevenz_atmostone_slavetats_riek  | 1     | 0             | none        | none            |
| sevenz_selectall_hh_walk         | 1     | 0             | none        | none            |
| sevenz_selectany_racemenu_plugins| 1     | 0             | none        | none            |
| zip_11step_cbbe_3ba              | 11    | 97            | none        | none            |
| zip_atmostone_heels_srd          | 1     | 0             | none        | none            |
| zip_exactlyone_mu_joint_fix      | 1     | 0             | none        | none            |
| zip_exactlyone_racecompat        | 1     | 2             | none        | none            |

ALL 15 fixtures are flag-only in visibility and conditional blocks. Since a flag
leaf evaluates identically in normal and inferred-with-any-override mode, running
the fixture tests with `overrides = None` (normal evaluation) reproduces the C++
golden simulate. The premise of trap (n) is therefore VERIFIED for every fixture;
no fixture needed a constructed override vector.

### Selection-grid reconstruction and step alignment

The fixture tests rebuild the C++ solver's winning `[step][group][plugin]` grid
from `expected.json`: each step emits `groups`, each group emits `plugins`
(selected) and `deselected`. IR is walked by position; `expected.json` emits one
entry per IR step (the `<installSteps>` container inflates a raw `<installStep`
grep by exactly 1, so the emitted step count equals the IR step count; all 15
verified) and per IR group; a plugin is selected iff its name is in that group's
`plugins` array, consuming name matches in order so duplicate names resolve
positionally. Alignment asserts (`step count`, `group count`, `selected +
deselected == IR plugin count`) guard the reconstruction and passed for all 15.

### Metrics oracle and the archive-listing size discrepancy (IMPORTANT)

TWO fixtures are non-exact in the C++ output, not one: `rar_exactlyone_heel_volume`
(missing=35, reproduced=1 - the documented pre-existing-mod-folder case) AND
`rar_7step_sos` (size_mismatch=5, reproduced=139). `rar_7step_sos` has no
external-dependency conditions (table above), so overrides are irrelevant and the
C++ pipeline is non-exact regardless - task option (1). Investigating it surfaced
a fixture-DATA inconsistency that also affects `sevenz_2step_nec_feet` and
`zip_11step_cbbe_3ba`:

- `outputTree.size` IS `atom->file_size` (`FomodInferenceService.cpp:495`). The
  golden inference run recorded `file_size = 0` for many output atoms (e.g.
  120 of 144 in `rar_7step_sos`, all with nonzero size in `archive_entries.json`);
  its live archive listing did not populate uncompressed sizes for those entries,
  while the committed `archive_entries.json` snapshots carry populated sizes. This
  is the libarchive/bit7z size-reporting split that CLAUDE.md warns about, between
  the Task 2 fixture snapshot and the golden inference run.
- When `atom.file_size == 0`, `compare_trees`'s size check is skipped and the file
  falls through to `reproduced`. So the golden run reproduced files whose sizes it
  never populated. A faithful end-to-end run over the FIXTURE atoms (nonzero sizes)
  instead reports `size_mismatch` for exactly those destinations whose fixture
  size differs from the installed target size. Three fixtures have such
  differences: `rar_7step_sos` (10 vs 5), `sevenz_2step_nec_feet` (12 vs 0),
  `zip_11step_cbbe_3ba` (3 vs 0). In every case the size-independent coverage total
  `size_mismatch + reproduced` is unchanged (144, 31, 177 respectively) and
  `missing`/`extra`/`hash_mismatch` match the golden `diagnostics.repro`.

This is NOT a forward-simulator defect. The simulator's conflict resolution is
size-INDEPENDENT (it compares only priority/document-order), and the fixture tests
prove faithfulness three ways over ALL 15 fixtures without being derailed by the
size discrepancy:

1. `simulate_reproduces_cpp_output_tree`: the Rust simulate's `dest -> winning
   source` map equals the C++ `outputTree` `dest -> source` map (byte-exact,
   size-independent) for every fixture.
2. `compare_trees_reproduces_cpp_repro_metrics`: `compare_trees` fed the
   reconstructed C++ `outputTree` (with the golden atom sizes) reproduces each
   fixture's full `diagnostics.repro` counters AND `exact_match` flag.
3. `end_to_end_matches_repro_metrics`: the end-to-end run over
   the fixture atoms matches `missing`, `extra`, `hash_mismatch`, and the
   coverage total `size_mismatch + reproduced` for every fixture.

The `heel_volume` metrics pin (missing=35, reproduced=1) is read from
`expected.json` `diagnostics.repro` at test time (not hardcoded) and reproduced
exactly by the end-to-end run, because `heel_volume` is size-consistent (its lone
output atom's fixture size equals its golden size and the target size). The
follow-up owner for the archive-listing discrepancy is Task 11 (Rust archive
layer) / Task 16 (round-trip validation): whichever size-reporting behavior the
Rust archive listing adopts will decide whether these three mods reproduce exactly
or as size-mismatch in the full Rust pipeline, and that must be reconciled with
the C++ behavior there. Logged here so it is not lost.

### Dropped C++ log sites

The forward simulator (`FomodForwardSimulator.cpp`) contains NO log calls, so
nothing was dropped from it. `compare_trees_impl`/`compare_trees`/
`collect_mismatched_dests` in `FomodCSPSolver.cpp` also contain no logging. The
`evaluate_candidate` progress logging (`[solver] ...` tqdm bar) lives in
`evaluate_candidate`, which is Task 8/9 and NOT in scope here.

### Accepted divergences (Task 6)

- None behavioral. Representational only: `Option<&_>` for nullable pointers,
  `impl FnMut(&str) -> bool` predicates instead of C++ template parameters,
  `&[Vec<Vec<bool>>]` for the const-ref selections vector, `usize` `flat_idx`
  (only ever incremented from 0, so signed `int` is unnecessary), and the phase-3
  `always_install || (install_if_usable && ...)` single condition in place of the
  C++ two-branch else-if (semantically identical; clippy `if_same_then_else`).
- Test-harness reuse: shared fixture-loading helpers were hoisted into
  `tests/common/mod.rs` (minijson reader, `load_archive_entries`, `prep_entries`,
  `derive_prefix`, `run_case` extended with the parsed `installer`, plus
  `load_expected`/`committed_cases`). The Task 5 `fomod_atoms_fixtures.rs` still
  carries its own inline copies to avoid churning a passing test; de-duplicating
  it onto `common` is a safe follow-up.

## Task 7 - Constraint propagator

Ported `src/FomodPropagator.hpp`/`.cpp` (the deterministic fixpoint pre-pass
that narrows plugin domains and can short-circuit the CSP solver) and, as its
first consumer, the WHOLE `ReasonCode` enum + `reason_code_to_string` from
`src/InferenceDiagnostics.hpp`/`.cpp` lines 43-93/236-290. The rest of
`InferenceDiagnostics.hpp` (the `Reason` struct, confidence types, the
`InferenceDiagnosticsBuilder` accumulator, the `serialize_*` helpers) is Task 10.

### API mapping

| C++ (mo2core)                                  | Rust                                                        |
|------------------------------------------------|-------------------------------------------------------------|
| `struct PropagationResult`                     | `fomod_propagator::PropagationResult`                       |
| `propagate(installer, atoms, atom_index, target, excluded, overrides, context*)` | `fomod_propagator::propagate(&installer, &atoms, &atom_index, &target, &excluded, &overrides, Option<&ctx>)` |
| `enum class ReasonCode : int`                  | `inference_diagnostics::ReasonCode` (`#[repr(i32)]`)        |
| `reason_code_to_string(ReasonCode)`            | `inference_diagnostics::reason_code_to_string -> &'static str` |
| (no C++ analogue - typed detail)               | `inference_diagnostics::ReasonDetail` (one variant so far)  |

`PropagationResult` field mapping (all sized to the installer hierarchy):

| C++ field                                             | Rust field                                     |
|-------------------------------------------------------|-------------------------------------------------|
| `vector<vector<vector<bool>>> narrowed_domains`       | `Vec<Vec<Vec<bool>>> narrowed_domains`          |
| `vector<tuple<int,int>> resolved_groups`              | `Vec<(i32, i32)> resolved_groups`               |
| `bool fully_resolved`                                 | `bool fully_resolved`                           |
| `vector<vector<vector<int>>> plugin_reasons`          | `Vec<Vec<Vec<ReasonCode>>> plugin_reasons`      |
| `vector<vector<vector<json>>> plugin_reason_details`  | `Vec<Vec<Vec<Option<ReasonDetail>>>>`           |
| `vector<vector<string>> resolved_by`                  | `Vec<Vec<String>> resolved_by`                  |

- `plugin_reasons`: the C++ stores `int` ONLY to avoid pulling
  `InferenceDiagnostics.hpp` into `FomodPropagator.hpp` (documented in the C++
  header). The Rust port has no such include cycle and stores the `ReasonCode`
  enum directly; read the numeric value with `code as i32`.
- `plugin_reason_details`: the C++ uses a nullable `nlohmann::json` (null =
  "no detail"). The port uses `Option<ReasonDetail>` where `ReasonDetail` is a
  typed enum whose ONLY current variant is `UniqueFileEvidence { files, count }`
  (the only detail the propagator emits). Tasks 8-10 add variants; Task 10 maps
  each to schema-v2 JSON. No JSON model is introduced now.

### The two unused parameters (`atom_index`, `overrides`)

`propagate` keeps both to match the call signature the CSP solver and inference
orchestrator (Tasks 8/12) use, but the C++ BODY never reads either. Verified by
grep over `FomodPropagator.cpp`: `atom_index` appears only on the parameter line
(53) and `overrides` only on the parameter line (56); neither `atom_index.` nor
`overrides.` occurs anywhere in the body. Why they are unread:

- `overrides` (step-visibility / conditional tri-state) is irrelevant because
  the propagator treats ALL steps as visible (the load-bearing C++ comment "All
  steps treated as visible"); it never evaluates `FomodStep::visible`, so there
  is nothing for a visibility override to affect.
- `atom_index` (reverse dest -> atoms map) is unused because the file-evidence
  rule reads `atoms.per_plugin[flat_start + pi]` directly (forward, per-plugin),
  not the reverse index.

Rust does NOT warn on unused function parameters, so both are kept un-prefixed
(NOT `_atom_index` / `_overrides`) to preserve the exact call signature. A
`let _ = (atom_index, overrides);` line documents the intent at the top of the
body.

### Doc-vs-code gap: `Required` is NOT pinned

`FomodPropagator.hpp`'s Doxygen (rule P) says "`Required` and `NotUsable`
plugins are pinned". The CODE does NOT pin Required: for `eff == Required` the
plugin-type rule only calls `record_plugin_reason(..., FORCED_REQUIRED)` - it
never sets `domain[pi]`, never eliminates siblings, and never resolves the
group. Only `NotUsable` mutates the domain. The port reproduces the CODE
(record-only), and `rule1_required_records_reason_but_does_not_pin_or_eliminate_sibling`
proves it via a `SelectExactlyOne` group with a Required + Optional pair that
stays unresolved (usable_count 2). This is a C++ doc-vs-code gap; per the
read-only-C++ rule it is logged here, not fixed.

### The three rules and the 16-iteration fixpoint

Per group (skipping already-resolved groups), rules 1-3 run in sequence on the
SAME domain so rule 2 sees rule 1's eliminations and rule 3 sees both:

1. **Plugin type** - `eff = evaluate_plugin_type(plugin, &flags, context)`
   (reused from Task 5). `NotUsable` eliminates ONLY when NOT
   `dynamic_without_context` (`context.is_none() && !type_patterns.is_empty()`):
   a dynamic `dependencyType` outcome during context-free inference is not
   definitive enough to prune. `Required` records `FORCED_REQUIRED` (no domain
   change; see the gap above).
2. **File evidence** - for each usable plugin, its group-unique NON-auto,
   non-excluded dests: if `has_any_unique && all_unique_miss` eliminate
   (`NO_FILE_EVIDENCE`); else if any unique dest hits the target record
   `UNIQUE_FILE_EVIDENCE` with up to 4 example files and the full hit `count`.
   Auto atoms (`always_install` / `install_if_usable`) and `excluded_dests`
   never enter the evidence sets.
3. **Cardinality** - `group_resolved` per group type: `SelectAll` always;
   `SelectExactlyOne`/`SelectAtLeastOne` at `usable_count == 1`;
   `SelectAtMostOne`/`SelectAny` at `usable_count == 0` ONLY (a `SelectAtMostOne`
   with one survivor keeps "select zero" valid, left for the CSP). On resolve:
   mark `resolved[si][gi]`, push `resolved_groups`, bump `total_resolved`, set
   `changed`, attribute `resolved_by`, stamp kept-plugin reasons, then accumulate
   the selected plugins' `condition_flags` into `flags` (last-write-wins).

The outer loop runs `0..MAX_ITERATIONS` (16), setting `changed = false` each
pass and breaking early when a full step/group walk makes no change. Fixpoint
iteration (not topological or single-pass) is required because FOMOD flag
dependencies can form cycles; the cap guards malformed installers, not
non-termination (each rule is monotone).
`fixpoint_flag_set_by_later_group_resolves_earlier_group_next_iteration`
proves a group ordered BEFORE the group that sets its trigger flag can only
resolve on a later iteration (a single pass would miss it).

`resolved_by` attribution: `SelectAll -> "propagation.select_all"`; else if any
plugin reason in the group is `UNIQUE_FILE_EVIDENCE`/`NO_FILE_EVIDENCE` ->
`"propagation.unique_evidence"`; else `"propagation.cardinality"`. (Note: the
C++ header doc lists a `"propagation.required"` string in the value set, but the
CODE never emits it - Required does not resolve a group. The port matches the
CODE.)

### Determinism pin on the `unordered_set` detail ordering

Rule 2's C++ builds `plugin_dests[pi]` as a `std::unordered_set<std::string>`
and, for a positive hit, pushes `unique_target_hits` in that set's iteration
order, then records the FIRST 4 as the detail `files`. That order is
nondeterministic across runs/platforms. The port pins it: `plugin_dests` is a
`BTreeSet<String>` and `unique_target_hits` is sorted byte-ascending before
taking the first 4. This is DIAGNOSTIC-ONLY and never changes a selection,
because the two decision booleans are order-independent reductions:
`has_any_unique = OR over (unique?)` and `all_unique_miss = AND over (unique ->
miss)` are commutative/associative, so the eliminate-vs-record branch is fixed
regardless of iteration order; only the example `files` list (and its ordering)
depends on it. The full `count` is taken BEFORE truncation to 4, so it can
exceed `files.len()`.
`rule2_unique_hit_detail_is_sorted_first_four_with_full_count` and the fixture
`propagation_is_deterministic` test lock this down.

### `fully_resolved` predicate

`fully_resolved = (total_resolved == total_groups)` where `total_groups` is the
sum of group counts over all steps. When true the CSP solver is skipped by the
caller (`FomodInferenceService.cpp` passes `&propagation` only when
`resolved_groups` is non-empty; `FomodCSPOptions.cpp` then prunes each group's
option space to the narrowed domain). For a fully-resolved installer the
narrowed domain IS the selection, so the fixture test
`fully_resolved_fixtures_match_the_selection_grid` asserts
`narrowed_domains == expected.json` selection grid for each such fixture. Over
the 15 committed cases, propagation fully resolves exactly 2
(`sevenz_selectall_hh_walk`, `sevenz_3step_tk_dodge`); the count is pinned (the
identities are derived at runtime, not hardcoded).

### `record_plugin_reason` first-wins + bounds

Mirror of the C++ file-scoped helper: overwrites the reason only when the
current code is `IMPLICIT_DEFAULT`, and sets the (moved) detail at the same
index. The C++ guards `s`/`g`/`p` against negative values AND upper bounds; in
Rust the indices are unsigned loop counters so the negative guard is vacuous -
only the upper-bound guard is kept (documented in the fn doc).
`record_plugin_reason_keeps_first_code` proves first-wins: a plugin that earns
`UNIQUE_FILE_EVIDENCE` in rule 2 and would earn `FORCED_EXACTLY_ONE` in rule 3
keeps the former.

### ReasonCode ported in full now

The entire enum (24 codes: `IMPLICIT_DEFAULT`, `FORCED_*`, `*_FILE_EVIDENCE`,
`CARDINALITY_FORCED`, `CSP_PHASE_*`, `CONDITION_*`/`STEP_*`,
`EXTRA_FILE_PRODUCED`, `FOMOD_PLUS_CACHE`) is ported with the EXACT C++ integer
values (`#[repr(i32)]`), even though the propagator only emits 8 of them, so
Tasks 8-10 (which reference the CSP/condition/step/penalty/cache codes) inherit
stable values. `reason_code_to_string` is an exhaustive match returning the C++
SCREAMING_SNAKE enumerator text; the C++ `"UNKNOWN"` miss-branch is UNREACHABLE
under a closed Rust enum and is therefore encoded as the absence of a wildcard
arm (same pattern as `fomod_ir::group_type_to_string`). Variant identifiers are
Rust UpperCamelCase (`ForcedRequired`); the wire names come from
`reason_code_to_string`. `ReasonCode` derives `Default = ImplicitDefault` to
match the C++ zero-value default.

### Dropped logger site (Task 17)

The C++ `propagate` ends with `logger.log("[propagate] resolved N/M groups,
fully_resolved=...")`. There is no Rust `Logger` until Task 17, so the log site
is dropped (a comment marks it in the body). No behavior depends on it. (The
`FomodInferenceService.cpp` `[infer] Step 7c` log is a separate caller-side site
in Task 12, not part of `propagate`.)

### Test-harness reuse

`build_selection_grid` (the positional `expected.json` -> `[step][group][plugin]`
reconstructor) was hoisted from `tests/fomod_forward_simulator_fixtures.rs` into
`tests/common/mod.rs` so both the Task 6 and Task 7 fixture suites share one
copy; the Task 6 file now imports it. No behavior change.

### Accepted divergences (Task 7)

- Representational only. The C++ holds `auto& domain = result.narrowed_domains
  [si][gi]` (an alias into `result`) while `record_plugin_reason(result, ...)`
  also mutates `result`; Rust forbids that aliasing, so the port `std::mem::take`s
  the group domain into a local `Vec<bool>`, works on it, and writes it back at
  the end of the group. The reason arrays are disjoint fields, so recording
  reasons while the domain is checked out is sound and observably identical.
- The three `for pi in 0..n` loops that ONLY index `domain` were rewritten as
  `domain.iter().enumerate()` / `iter_mut().enumerate()` to satisfy clippy
  `needless_range_loop`; `pi` still indexes `group.plugins`. Semantically
  identical.
- The C++ `assert(group_resolved, ...)` before flag accumulation is trivially
  true inside the `if group_resolved` block; the port drops it (a comment notes
  why) rather than emit a vacuous `debug_assert!`.

## Task 8 - CSP precompute + types + option enumeration

Ported `src/FomodCSPTypes.hpp` (the CSP datatype set), `src/FomodCSPPrecompute.hpp`/
`.cpp` (`compute_evidence`, `build_precompute`, `build_components`, and the
flag/condition helpers), and `src/FomodCSPOptions.hpp`/`.cpp` (per-group option
enumeration, reduction, caching, and the SelectAny caps). Also pulled in the two
solver types the datatype set needs so this module is self-contained: `SolverResult`
from `src/FomodCSPSolver.hpp` and `SolverConfig`/`kConfig` from
`src/FomodCSPSolverInternal.hpp`. The `solve_fomod_csp` entry point, the 5 phases,
and the search helpers (`rebuild_flags`, `evaluate_candidate`, `run_backtrack_pass`,
`greedy_solve`, `local_search`, `estimate_search_space`, `targeted_repair_search`,
...) are Task 9 and are NOT ported here.

### Module layout

| C++ TU                            | Rust module                    |
|-----------------------------------|--------------------------------|
| `FomodCSPTypes.hpp` (+ 2 imports) | `fomod_csp_types` (extended)   |
| `FomodCSPPrecompute.hpp`/`.cpp`   | `fomod_csp_precompute`         |
| `FomodCSPOptions.hpp`/`.cpp`      | `fomod_csp_options`            |

### Precompute borrow shape (the key structural decision)

The C++ `Precompute` holds seven non-owning `const*` inputs (installer, atoms,
atom_index, target, excluded, overrides, propagation) plus owned reverse indices.
The Rust `Precompute<'a>` models the inputs as `&'a T` (for the always-present
ones) and `Option<&'a T>` (for the nullable `overrides`/`propagation`); the owned
reverse indices are plain `Vec`/`HashMap`/`HashSet`. `build_precompute<'a>(...,
groups: Vec<GroupRef>, evidence: Vec<i32>) -> Precompute<'a>` borrows the seven
inputs for `'a` and moves in the two caller-built vectors, exactly like the C++
signature. This is the shape the Task 9 phases consume: they read `Precompute`
immutably (`&Precompute`) and mutate a SEPARATE, owned `SolverState`, so the
borrow of the inputs never conflicts with the mutable search state. `Precompute`
derives `PartialEq` so the determinism tests can compare two builds directly (the
reference fields compare pointees, which are identical for two builds from the
same inputs).

### Hashing reuse (byte-exact)

- `hash_flag_subset` (`FomodCSPPrecompute.cpp:53-67`) reuses `utils::fnv1a_hash`
  + `utils::hash_combine`: seed = FNV offset basis `14695981039346656037`; per key
  in the sorted key list, fold `fnv1a(key_bytes)`, then either `fnv1a(value_bytes)`
  (present) or the `0xA5A5A5A5A5A5A5A5` sentinel (absent). This `u64` is
  `OptionCacheKey.flags_sig` and is part of the cache key equality, so it is
  byte-exact. A present empty-string value is DISTINCT from an absent key (folds
  `fnv1a("")` vs the sentinel), and the fold makes it key-order-sensitive. Pinned
  micro-test constants were computed offline from the exact C++ fold.
- `option_signature` (`FomodCSPOptions.cpp:441-460`) likewise reuses
  `fnv1a_hash`+`hash_combine`: it COPIES `produced_atoms` (the `"dest|source"`
  keys, built at `FomodCSPOptions.cpp:427`) into a vector and sorts it, folds
  each; then copies `flags_written` into a `(name, value)` vector, sorts by
  name-then-value, and folds name then value per pair. The sorts make the hash
  deterministic despite the unordered source containers; pinned constant included.
- The `OptionCacheKey` / `MemoKey` std::hash functors (`FomodCSPTypes.hpp:350-362,
  389-398`) are NOT observable (the maps are only used via find/emplace, never
  iterated to output), so the Rust ports `#[derive(Hash)]`. Only `PartialEq`/`Eq`
  on the fields (`OptionCacheKey`: group_idx, flags_sig, select_any_cap,
  exact_mode; `MemoKey`: next_idx, flag_state_sig, contested_sig) is load-bearing,
  and that is covered by field-wise equality micro-tests. `flag_state_sig` /
  `contested_sig` are produced by Task 9.

### C++ nondeterminism made deterministic (total-order tiebreaks)

Task 8 outputs are NOT recorded in the fixtures' `expected.json`, so byte-parity
against a specific C++ run is not available (it resumes at Task 9). Where the C++
uses an unstable sort or iterates an unordered container, the port adds a TOTAL
tiebreak for run-to-run determinism; exact C++ tie order is explicitly NOT
reproduced.

| C++ site | C++ order | Rust total tiebreak |
|----------|-----------|---------------------|
| `build_components` ordering (`FomodCSPPrecompute.cpp:364-366`) | unstable sort by size DESC only | size DESC, then min-member ASC (each component is pre-sorted ascending, so `comp[0]` is its min) |
| `generate_raw_options` evidence order (`FomodCSPOptions.cpp:123-125`) | unstable sort by evidence DESC | evidence DESC, then plugin-index ASC |
| `generate_raw_options` small-group powerset (`FomodCSPOptions.cpp:248-250`) | unstable sort by score DESC | score DESC, then bitmask ASC |
| `reduce_options` candidate list (`FomodCSPOptions.cpp:536-554`) | surviving indices from an UNORDERED `best_by_sig` map, then unstable sort by (evidence DESC, unique DESC, useful DESC, extra ASC) | same four keys, then raw-option index ASC as the final total tiebreak (input order from the unordered map no longer matters) |

The BFS neighbor-visit order inside `build_components` is also unordered, but each
finished component is sorted ascending before storage, so membership is
deterministic regardless. The `compute_evidence` target-tree iteration and the
`build_precompute` contested loop iterate unordered maps but only do commutative
`+=`/set-insert, so their outputs are order-independent. Every reverse-index
vector is sort+dedup ascending.

The GroupRef list the CSP entry point builds (`FomodCSPSolver.cpp:1499-1558`,
document order then a per-step UNSTABLE sort by group-priority DESC then
plugin_count ASC) is a Task 9 CALLER concern; the Task 8 fixture tests reproduce
it with a STABLE sort (ties keep document order) purely to exercise
`build_precompute` on realistic inputs. Its equal-(priority, plugin_count) tie
order is another non-bit-parity site, noted here.

### SelectAny cap pick_rank (diversity-then-fill)

`reduce_options` caps SelectAny/SelectAtLeastOne candidate lists
(`FomodCSPOptions.cpp:559-597`) via `pick_rank` (guards: rank in range, not
already picked, `narrowed.len() < cap` - the guard is strict `<`, so the boundary
is off-by-one sensitive and kept exactly). Order: (a) `pick_rank(0)` keeps the top
option; (b) diversity - for each rank in order, if the option selected-plugin
popcount is newly seen, `pick_rank(rank)`; (c) fill - `pick_rank(rank)` in order
until `cap` reached. `stats.capped_select_any_options += candidates.len() -
narrowed.len()`. The port keeps `pick_rank` as a free function (rather than a
closure) so the `&candidates` / `&mut narrowed` / `&mut chosen_rank` borrows stay
simple. A micro-test constructs > cap distinct candidates and asserts exactly
`cap` survive plus the counter delta.

### Dead forced_unique_options counter

`SolverStats::forced_unique_options` (`FomodCSPTypes.hpp:167`) is declared but
NEVER incremented anywhere in the C++ (verified across the whole solver). The Rust
field is kept for struct parity and is documented as dead; Task 8 does NOT
increment it.

### Log-only surface (Task 17)

- `group_name` (`FomodCSPOptions.cpp:29-36`) is used only in log lines. It is
  ported (pub, so no dead-code warning) but has no behavioral effect.
- `get_options_for_group` branching/group-stats logging block
  (`FomodCSPOptions.cpp:666-692`) is dropped; its ONLY side effect,
  `stats.logged_group_options[gidx] = true`, is preserved (guarded with `.get_mut`
  so an unsized counter vector cannot panic, since the flag is behaviorally inert).
- The out-of-range `gidx` error log (`FomodCSPOptions.cpp:622-623`) is dropped; the
  function still returns a shared empty `CachedOptions` (a process-wide
  `OnceLock<CachedOptions>` static, mirroring the C++ `static const CachedOptions
  kEmpty`), and never populates the cache on that path.

### Accepted divergences (Task 8)

- `get_options_for_group` uses the `Entry::Vacant` form instead of the C++
  `contains_key` + `emplace` (satisfies clippy `map_entry`) and returns via a
  final `cache.get(&key).unwrap()`. The value is computed ONLY on a miss because
  `reduce_options` has `SolverStats` side effects that must not be double-counted
  on a cache hit.
- `generate_raw_options` builds the required-mask options by cloning the
  `required` vector instead of the C++ per-index copy loop, and the SelectAll
  option / powerset masks likewise (clippy `manual_memcpy` / `needless_range_loop`).
  Value-identical.
- `SolverProgress` uses `std::time::Instant` for the C++
  `steady_clock::time_point` fields and models the not-yet-set `deadline` as
  `Option<Instant>` (`None`) rather than the C++ clock-epoch placeholder; the
  `PROGRESS_NODE_INTERVAL` / `PROGRESS_TIME_INTERVAL_MS` `static constexpr` members
  become associated `const`s. Consumed by Task 9.
- `SelectAny` cap constants are named `SELECT_ANY_CAP_{NARROW,MEDIUM,FULL}`
  (Rust SCREAMING_SNAKE) for `kSelectAnyCap{Narrow,Medium,Full}`; `kConfig` becomes
  `CONFIG` (a `const SolverConfig`, backed by `SolverConfig::DEFAULT`).

### Oracle limitation (why the fixture tests are structural)

The `Precompute` reverse indices and per-group option lists are internal
intermediates NOT recorded in `expected.json` (which holds the final selection
grid, produced at Task 9). So the golden-fixture tests here assert STRUCTURE
(shapes match the installer hierarchy; every reverse index is sorted+deduped;
components partition `[0, group_count)` size-descending) and DETERMINISM (two
builds are identical; `get_options_for_group` returns a stable option count and
valid masks). The BEHAVIORAL parity lives in the hand-derived micro-tests
(evidence +3/+2/+1 and flag propagation; the per-group-type option masks and their
total order; the post-filter, extra-only drop, signature collapse, and SelectAny
cap; the two byte-exact hash folds). Byte-parity against recorded C++ intermediates
resumes at Task 9.

## Task 9 - CSP solver phases

Port of `src/FomodCSPSolver.cpp` + `src/FomodCSPSolverPhases.cpp` to
`src/fomod_csp_solver.rs`. The C++ splits the solver core and
the five phase functions across two TUs; the port folds them into one module so
the large private helper set (rebuild_flags, evaluate_candidate, lower_bound,
contested_signature, the backtracker, etc.) stays module-private. Only
`solve_fomod_csp` is `pub`. The `CheckpointGuard` struct (CSP.cpp:941-978) is DEAD
in C++ and not ported.

### Entry control flow and phase short-circuits (`solve_fomod_csp`)

Mirrors CSP.cpp:1488-1745 exactly. (S1) flat GroupRef list in document order,
`flat_start` captured BEFORE the per-step sort. (S2) per-step priority sort. (S3)
`compute_evidence` + `build_precompute`; seed `SolverState` with every plugin
false (no propagation-forced seeding in the solver). (S4) `deadline = now + 600s`,
`select_any_cap = SELECT_ANY_CAP_NARROW` for phases 1-4. (S5) `run_initial_phases`
always; then `run_component_decomposition`, `run_residual_repair`,
`run_focused_search`, `run_global_fallback` each guarded by
`!found_exact && !deadline_exceeded`, setting `ran_phaseN=true` when entered (no
early return). (S6) assemble result. The C++ per-GroupRef bounds `assert` block
(CSP.cpp:1588-1602) is dropped (guaranteed by construction). The solver does NOT
itself check `propagation.fully_resolved` to bypass - that is the Task 12 caller.

### Per-step priority sort - total-order tiebreak (S2, tie landmine)

`group_priority`: SelectAll=4, SelectExactlyOne=3, SelectAtMostOne=2,
SelectAtLeastOne=1, SelectAny=0. C++ sorts each step's contiguous group range by
`(priority DESC, plugin_count ASC)` with an UNSTABLE `std::sort` and NO tertiary
key, so equal `(priority, plugin_count)` groups get unspecified order. The port
adds a document-order tertiary key (`group_idx ASC`) making the comparator a
TOTAL order, and uses the stable `slice::sort_by`. This guarantees run-to-run
determinism but does NOT guarantee bit-parity with a specific MSVC run when a
step has two groups of equal priority AND equal plugin_count; the golden fixtures
never exercise that tie (all pass exact-grid). Same total-order philosophy the
Task 8 component sort already applies.

### `better_than` first-found-wins and its testing implication

`ReproMetrics::better_than` rejects an EQUAL tuple (Task 6/8), so
`evaluate_candidate` (the SOLE `nodes_explored++` site, CSP.cpp:221) keeps the
FIRST-discovered candidate at any metric tuple. Every visitation/iteration/sort
order is therefore observable in the final grid whenever ties exist. Testing
consequence, realized in `tests/fomod_csp_solver_fixtures.rs`:
- METRICS parity (missing/extra/size/hash/reproduced + `exact_match` +
  `phase_reached`) is robust to grid ties and is asserted for every
  archive-consistent fixture.
- EXACT-GRID parity (byte-for-byte grid) is only asserted for the deterministic
  subset (fixtures with no accepted-path metric ties). All 12 archive-consistent
  fixtures return the exact C++ grid, so the pinned `EXACT_GRID_COUNT = 12`; there
  is currently NO fixture that matched metrics but diverged on grid bytes.

### The archive-listing size discrepancy - solver consequence (3 fixtures)

The Task 6 note records that the golden run scored atoms with `file_size = 0`
(its live archive listing did not populate uncompressed sizes) while the committed
`archive_entries.json` snapshots carry populated sizes; for 3 fixtures
(`rar_7step_sos`, `sevenz_2step_nec_feet`, `zip_11step_cbbe_3ba`) some populated
sizes differ from the installed target size. Task 9 inherits this: the SOLVER
optimizes against the fixture atoms, so for these 3 it scores the extra size
mismatches the golden run never saw. Verified behavior:
- 12 "consistent" fixtures (expected-grid metrics == `diagnostics.repro`): full
  solver parity - grid, all five repro counters, `exact_match`, `phase_reached`.
- `sevenz_2step_nec_feet`, `rar_7step_sos`: terminate quickly but score the extra
  size mismatches, so `exact_match`/`phase_reached`/`size_mismatch` diverge; only
  the size-INDEPENDENT counters (missing/extra/hash) and `size+reproduced`
  coverage match. `sevenz_2step_nec_feet` still returns the correct grid (the
  mismatches are unfixable so the greedy grid is kept); `rar_7step_sos` returns a
  different grid.
- `zip_11step_cbbe_3ba` (100 plugins): with the golden size-0 atoms the C++ run
  reached exact in the greedy phase (89 nodes); with the fixture atoms it can
  never short-circuit on exact and searches the full space to the 600s wall-clock
  deadline (measured: 544s / ~603k nodes in RELEASE, returning a different grid).
  It is EXCLUDED from the solver-driving tests (pinned by name `NON_TERMINATING`),
  and its inconsistency is still asserted via the size-split invariants on the
  KNOWN expected grid (no solver run). This is a Task 2 fixture-data discrepancy,
  NOT a solver defect: the solver is correct for consistent data (12/12 grids +
  metrics). `metrics_parity_all_fixtures` is therefore FALSE (3 fixtures diverge
  on the size split), reported as a concern, not a bug.

### Iterative backtracker + memo + lower-bound pruning

`backtrack` (CSP.cpp:980-1354) is an explicit heap-stack loop, NOT Rust recursion
(a deep installer would overflow the native 1 MB stack). Frame fields, the
two-phase structure (phase-1 init: depth/node-limit/deadline guards, then the
skip loop over single-option/invisible groups, then bounds+memo at the branching
group; phase-2: extra-only prune scan, apply option, push child), the shared
`checkpoints` / `flag_undo` / `stack` vectors, `kMaxBacktrackDepth = 500`,
`save_checkpoint` refusing past `CONFIG.max_checkpoints = 4096`, and `unwind_frame`
are ported field-for-field. Rust-specific shape: the top frame is accessed by
index (`stack[top]`) so no `&mut` borrow is held across a `stack.push`/`pop`; frames
are popped-then-unwound. Node-limit and deadline guards use `>=` and the deadline
is sampled only when `(nodes_explored & 63) == 0`, exactly as C++.
- Lower bound (CSP.cpp:572-655): `simulate` then `compare_trees_impl` with three
  predicates that count a mismatch only when NO unassigned group (`order_pos >=
  next_idx`, via `has_remaining_group` on `dest_to_{groups,size_match_groups,
  hash_capable_groups}`) and no `conditional_repair_remaining` flag-setter can fix
  it. `cannot_beat` is strict lexicographic `>` on
  `(missing, extra, size_mismatch, hash_mismatch)`.
- `run_bounds_here = enable_bounds && ci >= 4 && ci % bound_stride == 0` with the
  exact pressure-tiered stride table; `enable_memo` gated on best counters `<= 24`.
- Memo (`plan.memo: HashMap<MemoKey, ReproMetrics>`): a re-hit with a not-better
  `lb` prunes (`!lb.better_than(stored)`); store on miss or strictly-better `lb`,
  clearing the whole table at `kMaxMemoEntries = 100000`. The `MemoKey` is
  `{ next_idx = ci, flag_state_sig = hash_flag_subset(flags, memo_flags),
  contested_sig }` - both signatures are byte-exact (below). The map is used only
  via get/insert (never iterated to output), so its `#[derive(Hash)]` order is not
  observable - matching the Task 8 note.

### `contested_signature` byte-fold reuses the utils helpers

`contested_signature` (CSP.cpp:672-708) seeds `sig = 0xcbf29ce484222325` (FNV
offset basis) and, iterating `pre.contested_plugins` (the SORTED-ascending vec),
folds `(flat_plugin + 1) as u64` via `utils::hash_combine` for each contested
plugin whose group is already assigned (`0 <= order_pos < next_idx`) AND currently
selected. Unassigned groups and deselected plugins are skipped. This is a `MemoKey`
equality field, so the fold is byte-exact; the micro-test
`contested_signature_folds_selected_assigned_contested_in_sorted_order`
hand-derives the `u64` for a 2-plugin contested set (only selected, already-
assigned plugins fold, in sorted order).

### The two flag-replay orders and their call sites

Two distinct flag replays, reproduced exactly:
- `rebuild_flags` (CSP.cpp:37-100) walks steps/groups in DOCUMENT order,
  short-circuiting before `(stop_step, stop_group)` when set. Call sites:
  `evaluate_candidate`'s `best.inferred_flags` (full), `local_search` (per group,
  `stop = gref`), the non-incremental backtrack skip/bounds path, and every phase's
  full rebuild.
- `advance_flags_past_group` (CSP.cpp:897-924) advances ONE group's plugins in the
  priority-sorted `pre.groups`/plan order, pushing a `FlagDelta` per mutation for
  `undo_flags_to`. Call sites: `greedy_solve` and the incremental backtrack path
  (enabled only when `plan.order.len() == pre.groups.len()`, CSP.cpp:1402).
The two orders differ intra-step; the micro-test
`rebuild_flags_document_order_vs_advance_group_order` pins a case where document
order yields `F=b` while reverse group advance yields `F=a`.

### `SolverProgress.deadline` as `Option<Instant>`

Task 8 already modeled `deadline` as `Option<Instant>` (`None` = unset), matching
the port plan; Task 9 consumes it. The C++ sentinel test
`deadline.time_since_epoch().count() != 0` becomes `if let Some(deadline) = ...`.
Only `deadline` / `deadline_exceeded` have behavioral effect; the other
`SolverProgress` fields (`estimated_total`, `pass_start_*`, `last_progress_*`)
feed only progress logging and are NOT tracked (see dropped logs below). The
`deadline_in_the_past_trips_and_none_never_does` micro-test covers both the past
deadline (`deadline_exceeded` flips) and the `None` case (never trips, pass runs
to completion).

### `phase_reached` / `phase_per_group` derivation; `alternatives_per_group`

`phase_reached` (CSP.cpp:1701-1718): default `"csp.greedy"`; `"csp.fallback"` if
phase 5 ran, else `"csp.focused"` (4), `"csp.repair"` (3), `"csp.local_search"`
(2), in that if/elif order - it names the DEEPEST phase that RAN, not the phase
that found the result (so a fixture solved by local search inside phase 1 still
reports `"csp.greedy"`; `sevenz_atmostone_slavetats_riek` does exactly this at 5
nodes). `phase_per_group[s][g]` = `""` when `(s,g)` is in
`propagation.resolved_groups` (linear search), else `final_phase`.
`alternatives_per_group` is ALWAYS all zeros (the C++ `assign(groups, 0)`; never
computed anywhere).

### Repair local sort - added total-order tiebreak

`build_repair_plugin_map` (CSP.cpp:395-475) sorts candidate local plugin indices
by `evidence ASC` with an UNSTABLE `std::sort` and no tie key; the port adds a
`local-index ASC` tiebreak for run-to-run determinism (same philosophy as S2).
Caps at `kMaxRepairBits = 11`, keeps groups with `>= 2` bits, SelectAny/
SelectAtLeastOne only.

### `thread_local` scratch replaced with fresh allocation

`evaluate_candidate` and `lower_bound` use a C++ `thread_local SimulatedTree
scratch` reused via `simulate_into` (an allocation optimization on the hot path).
The port calls `simulate` (a fresh tree) each time; behaviorally identical (the
scratch is cleared before every use). The periodic scratch-shrink
(`kScratchShrinkThreshold`) is likewise not needed. This is a performance-only
divergence; a Task 16 profiling pass may reintroduce a reused scratch if the hot
loop needs it.

### Dropped log sites (Task 17)

Every `Logger::instance().log(...)` / `log_warning(...)` and the tqdm progress-bar
machinery in both C++ TUs are dropped (the Rust logger arrives in Task 17). Sites:
the `[solver] Starting CSP` / `Done` / `Pruning summary` / `Domain reduction
summary` / wall-clock-exceeded lines in `solve_fomod_csp`; the per-phase
`[solver] Phase: ...` / `After ...` lines in every phase function; the
`build_tqdm_bar` progress lines and initial/final bars in `evaluate_candidate` and
`run_backtrack_pass`; the checkpoint-limit and kMaxBacktrackDepth warn-once lines
in `backtrack`; the `group_name`-based affected-groups log lines. The `format_count`
/ `format_option_cap` / `format_duration` / `build_tqdm_bar` formatters
(CSP.cpp:719-778) are not ported. None sits in a decision path; the associated
progress-field bookkeeping (documented above) is dropped with them. `stats.*`
counters ARE maintained (they drive pruning-related branch decisions and are
asserted by the micro-tests).

### Consumed Task-8 dead code

This task is the first consumer of `SolverState`/`SolverSearchState`/
`SolverBestResult`/`SolverProgress`, `SearchPlan`, `MemoKey`, `FlagDelta`,
`SolverConfig`/`CONFIG`, and the pruning `SolverStats` counters. No `#[allow(
dead_code)]` needed to be removed (Task 8 relied on `pub` visibility, not
allowances).

### Test-suite delta

14 new tests: 9 solver micro-tests in `src/fomod_csp_solver.rs`
(`evaluate_candidate` first-found-wins; `apply_option`; byte-exact
`contested_signature`; the two flag-replay orders; `lower_bound` unfixable-only;
extra-only prune; `>=` node-limit boundary; memo re-hit prune; deadline
past/`None`) and 5 fixture tests in `tests/fomod_csp_solver_fixtures.rs`
(metrics parity over the 12 consistent fixtures; exact-grid over the deterministic
subset, count pinned at 12 with the two propagation-resolved fixtures asserted by
name; the inconsistent 3 diverge only on the size split; solver coverage on the 2
fast inconsistent fixtures via size-independent invariants; determinism over
`zip_exactlyone_racecompat` + `rar_7step_sos`).

## Task 10 - Diagnostics + assemble_json (schema-v2 byte parity)

Ports the confidence scoring + reason accumulation (`src/InferenceDiagnostics.hpp`
/ `.cpp`), the schema-v2 `assemble_json` (`src/FomodInferenceAtoms.cpp:306-467`),
`add_output_tree` (`src/FomodInferenceService.cpp:468-503`), and a hand-written
`nlohmann::json::dump(2)`-faithful JSON serializer. The acceptance bar is
BYTE-IDENTICAL output to the golden `expected.json` files (each of which IS the
C++ DLL's `assemble_json -> add_output_tree -> dump(2)` output).

### The JSON value model + serializer (`src/json.rs`)

No `serde` / `serde_json` (forbidden, and unnecessary): a small owned
`Value { Null, Bool, Int(i64), Double(f64), Str, Array(Vec), Object(BTreeMap) }`
plus a `dump(indent)` that reproduces `nlohmann::json::dump(2)` byte for byte.
The rules replicated, each verified against the golden fixtures:

- **Sorted object keys.** `nlohmann::json` is `std::map`-backed (NOT
  `ordered_json`), so members serialize in `std::string operator<` order ==
  unsigned-byte lexicographic == Rust `str` `Ord`. `Value::Object` stores a
  `BTreeMap<String, Value>`, which iterates in exactly that order (all keys are
  ASCII). The C++ builders insert in a different order everywhere (e.g.
  `serialize_reason` inserts code, message, detail but emits code, detail,
  message); the Rust code can insert in any order because `dump` sorts.
- **Pretty layout (indent 2).** Two spaces per nesting level; `\n` newlines; an
  object member line is `<indent>"key": value` (colon + single space); an array
  element line is `<indent>value`; members/elements are joined with `,\n`;
  `{`/`[` are immediately followed by `\n`; the closing `}`/`]` sits on its own
  line at the PARENT indent. No BOM.
- **No trailing newline.** The document ends in `}` (verified: golden files end
  `... 5d 0a 7d`).
- **Empty containers inline.** `[]` and `{}` render on a single line with no
  interior whitespace even in pretty mode (`"reasons": []`, `"deselected": []`,
  `"plugins": []`).
- **Int vs Double is load-bearing.** `Value::Int` prints a plain decimal (`0`,
  `2`, `802816`); `Value::Double` always carries a decimal point (`0.0`, `1.0`,
  `0.54`). The same zero renders `"0"` (a count/size/schema_version/nodes/
  timing/detail.count/outputTree.size) or `"0.0"` (every confidence field)
  depending on the C++ static type. The assembler constructs the correct variant
  per field and never unifies them.
- **The float `.0` rule.** nlohmann's `dtoa` and Rust's `f64` `Display` both emit
  the SHORTEST decimal that round-trips to the same IEEE-754 double, so for
  identical bits the digit sequence is identical. The one systematic gap is that
  Rust prints an integer-valued double as `1`/`0` while nlohmann prints
  `1.0`/`0.0`. `format_double` appends `.0` exactly when the shortest string
  contains none of `.`, `e`, `E` (and the value is finite; non-finite -> `null`,
  matching nlohmann). The float oracle test pins the hard values (0.6, 0.54,
  0.58, 0.8950000000000001, 0.9176215277777777 and its adjacent double
  0.9176215277777778, 0.9999999999999999, 0.9250000000000002) and every one
  reproduces the golden bytes. Latent risk (documented, NOT exercised): a
  magnitude outside ~`[1e-5, 1e16]` would switch nlohmann to an exponent format
  this simple rule does not replicate - no confidence value (all in `[0, 1]`) or
  fixture size falls there.
- **String escaping** matches nlohmann's default (`ensure_ascii=false`): `"` ->
  `\"`, `\` -> `\\`, the C0 shortcuts `\b \f \n \r \t`, any other control byte
  `< 0x20` as `\u00XX` with LOWERCASE hex; `/` is NOT escaped; non-ASCII UTF-8
  passes through as raw bytes.

### Confidence math - NO rounding on the JSON path

The four-component weighted composite (evidence 0.40, propagation 0.30, repro
0.20, ambiguity 0.10), `clamp01`, `weighted_mean` (weight <= 0 -> 1.0),
`band_for` (>=0.85 high, >=0.50 medium, else low), the per-plugin / per-group
(all-forced short-circuit to composite 1.0, else file-count weighted mean) /
per-step / run aggregation, the run penalties (extra capped at 5 * 0.05;
csp.fallback -0.10), and the `reproduced` backfill (target-derived or
selected-file-count proxy) are ported verbatim. Confidence doubles are stored
and serialized WITHOUT rounding - the wire value is the raw `f64`. The
`composite_from` multiply-add expression order is preserved EXACTLY so the bits
match: an all-ones plugin yields `0.9999999999999999` (not `1.0`), which every
fixture emits for propagation-forced plugins; the all-forced GROUP short-circuit
writes a literal `1.0`. No FMA contraction happens in either language for
`a * b + c` (MSVC without `/fp:fast`, Rust without `mul_add`), so the two produce
identical doubles. `clamp01` is expressed as `f64::clamp(0.0, 1.0)`, which is
behaviorally identical to the C++ branch form for every value the formula
produces (below-range -> 0.0, above-range -> 1.0, in-range unchanged, and the
never-occurring NaN -> NaN); the finite constant bounds mean it cannot panic.

### Reason accumulation order (must be same codes, same order)

`set_cache_hit` (Tier-1) first when applicable -> `absorb_propagation` (walks
`[s][g][p]` ascending, one reason per non-`IMPLICIT_DEFAULT`
`plugin_reasons[s][g][p]`, message by code, detail from
`plugin_reason_details`; copies `resolved_by` per group) -> `absorb_solver`
(per group with non-empty `phase_per_group`, a `CSP_PHASE_*` reason on each
SELECTED plugin with detail `{nodes, phase}`; sets `resolved_by` only if still
empty; mirrors `selected`; computes group counts: propagation if `resolved_by`
starts `propagation` or == `cache.fomod_plus`, csp if starts `csp.`) ->
`set_step_visibility` (one step reason) -> `finalize`. `serialize_reason` always
emits `code` + `message` and emits `detail` only when present (`Some(_)`),
sorted-emitting `code, detail, message`.

### Added `ReasonDetail::CspPhase { nodes: i32, phase: String }`

Task 7 shipped `ReasonDetail` with only `UniqueFileEvidence { files, count }`.
`absorb_solver` needs a second variant for the CSP-phase detail
(`InferenceDiagnostics.cpp:612-614` builds `{phase, nodes}`), so a
`CspPhase { nodes, phase }` variant was added; it serializes to sorted keys
`nodes` then `phase`. Without it every CSP-selected plugin would lose its
`reasons[].detail` and the bytes would not match.

### `add_output_tree` ported here though its C++ home is FomodInferenceService.cpp

`add_output_tree` lives in the (Task 12) inference-service TU in C++, but the
byte oracle needs it, so it is ported into `fomod_inference_atoms.rs` alongside
`assemble_json`. It sorts the simulated tree by `dest_path` (byte order), caps at
`kMaxOutputTreeEntries = 5000` (setting `outputTreeTruncated`/`outputTreeTotal`
when capped; no fixture triggers it), and emits `{path, size, source}`
(sorted keys) with `size` an `Int` (`atom.file_size`). Task 12's inference
orchestrator will call it.

### `timings_ms` non-determinism + inject-from-fixture strategy

`diagnostics.timings_ms.{list,scan,solve,total}` are wall-clock (`FomodInference
Service.cpp:1304`) and non-reproducible. For the byte tests the four integers are
parsed from `expected.json` and fed to `set_run_timings`, exactly as the task
prescribes; timing VALUES are never asserted from a live clock.

### Step visibility injected from the fixture (compute_overrides is Task 12)

15/16 committed fixtures carry a per-step visibility reason
(`STEP_VISIBILITY_FORCED` when `visible`, or `STEP_VISIBILITY_UNKNOWN`; never
`STEP_NOT_VISIBLE` in the corpus - both keep `visible: true`). These come from
`FomodInferenceService::compute_overrides` (step-unique-dest evidence), which is
Task 12 orchestration and NOT ported here. Following the same principle as the
timings injection, the byte tests read each step's visibility CODE and `visible`
flag from `expected.json` and feed `set_step_visibility`; the Rust builder then
produces the reason MESSAGE + ordering, so the message mapping and serialization
are still validated end to end. Only the `compute_overrides` OUTPUT (a 3-valued
code per step) is borrowed. Step reasons do not feed the confidence math, so this
injection does not affect any confidence value.

### Two C++ non-determinisms that block FULL byte parity (NOT fixed - logged)

1. **`outputTree[].size` size-0 discrepancy.** `atom.file_size` came, in the
   golden run, from a LIVE archive listing that returned 0 for many entries (see
   "Task 6" above), whereas the committed `archive_entries.json` snapshots carry
   populated sizes. A Rust run over the fixture atoms therefore emits the
   populated size where the golden run emitted 0 (e.g. `base files/condhhw.esp`:
   fixture 193, golden 0). Fixture-data issue, not a serializer bug.
2. **`UNIQUE_FILE_EVIDENCE` `files` example order.** The C++ propagator builds
   the up-to-4 example list by iterating a `std::unordered_set<std::string>`
   (`FomodPropagator.cpp:170,206-215`) - MSVC hash order. Both the ORDER and,
   when `count > 4` (20 of 42 details in the corpus), the chosen 4-of-N SUBSET
   are unreproducible. The Rust propagator (Task 7) sorts byte-ascending for
   determinism. This is a C++ nondeterminism (unordered-container iteration
   leaking into output); per the task rules the C++ is left untouched and the
   Rust deterministic order stands.

Neither is fixable without corrupting the port (option 1 would need the golden
listing; option 2 would need to replicate MSVC's STL hash). Both are isolated by
the tests rather than papered over.

### Byte-exact fixture subset (count PINNED = 1)

- `full_byte_parity_over_consistent_fixtures` asserts, over all 12
  archive-consistent fixtures, that the ENTIRE document is byte-identical AFTER
  collapsing the two nondeterministic regions above (every bare `"size": N`
  outputTree line, and every `"files": [ ... ]` example block). Passing this
  proves confidence, reasons, codes, messages, `count`, `nodes`, resolved_by,
  timings, cache, step visibility, key ordering, indentation, escaping, and the
  float format ALL match byte-for-byte. It then counts fixtures whose UNnormalized
  document is byte-identical (sizes and files included) and PINS that count at
  **1** (`rar_exactlyone_heel_volume` - the only consistent fixture with no
  size-0 outputTree atom AND no multi-hit `UNIQUE_FILE_EVIDENCE` example list).
- `unique_file_evidence_content_matches_over_consistent_fixtures` recovers the
  coverage the `files` normalization drops: `count` matches at every position,
  and where `count <= 4` (the full hit set fits) the file SET matches C++ - so
  the normalization hides only ORDER, never content.
- `skeleton_and_output_tree_over_all_fixtures` asserts, for EVERY committed
  fixture (consistent, the 3 size-split inconsistent, and the non-terminating
  `zip_11step_cbbe_3ba`, all driven from the KNOWN expected grid so the solver is
  never run on the non-terminating case), that `schema_version`, the
  step/group/plugin skeleton (names + selected/deselected split), and the
  `outputTree` (path, source) pairs byte-match. The size-split fixtures are ONLY
  skeleton/outputTree comparable because their `diagnostics.repro` counters (and
  thus the repro confidence component) diverge from `expected.json` on the
  documented size split, so a full-document comparison is not meaningful for them.

Tests added: `src/json.rs` (11 serializer + float-oracle + introspection
micro-tests), `src/inference_diagnostics.rs` (confidence-formula boundary tables,
the mu_joint_fix worked example, all-forced short-circuit, run penalties, the
reproduced backfill both branches, and serialize_* key-order/detail-shape tests),
`tests/inference_diagnostics_test.rs` (the 9 ported GoogleTest cases 1:1), and
`tests/inference_diagnostics_fixtures.rs` (the 4 byte-parity fixture tests).

## Task 11 - Archive layer

Ports `src/ArchiveService.hpp` / `.cpp` (the libarchive + bit7z facade) onto a
pure-crate stack chosen by a prior empirical corpus evaluation. `src/archive_service.rs`
provides `EntryListing`, `use_bit7z` / format routing, `list_entries_with_sizes`
+ `list_entries`, `read_entry` + `read_entries_batch` (256 MiB cap, solid-batch
strategy), `extract` + `extract_filtered` + `extract_prefix` (traversal
rejection), and `create_zip`. The acceptance bar is BYTE parity of
`list_entries_with_sizes` against the golden `archive_entries.json` corpus.

### Evaluated crate stack (the only sanctioned new dependencies)

- `zip = "8.6.0"`           - ZIP; native central-directory order, forward-slash paths.
- `sevenz-rust2 = "0.21.3"` - 7z (and `.001` routing); solid-block aware, header-only listing.
- `unrar = "0.5.8"`         - RAR; links the proprietary unRAR C sources via `unrar_sys`.

Transitively this pulls `flate2` (zip inflate/deflate), `lzma-rust2` (7z LZMA),
and `unrar_sys` (the `cc`-compiled unRAR vendor sources). No other deps added,
the stack is not swapped.

### Corpus byte-parity result: 16/16

`tests/archive_service_fixtures.rs` opens each committed case's REAL source
archive (from `case.json:source_archive_path`) and asserts the produced
`[{path, size}]` list equals `archive_entries.json` EXACTLY (ordered, byte-exact
path bytes + sizes) across all three formats. On this dev machine all 16
archives are present and all 16 match. The test counts run-vs-skipped cases and,
when any ran, asserts every one matched (never a subset / set-compare).

### Per-format list normalization that reproduces the golden

The golden `archive_entries.json` = the C++ `list_entries_with_sizes` serialized
as `[{path, size}]`, where `path` keeps original casing and `size` is the
uncompressed byte count. bit7z (and the `7z.exe`/`zipfile` tools that generated
the golden) echo 7-Zip / WinRAR case-insensitive stored order. Reproduced
per format:

- ZIP (`zip` crate): iterate `by_index(0..len())` in native central-directory
  order, NO sort; skip `is_dir()` entries; keep the stored forward-slash
  `name()`. `size()` is the uncompressed size.
  - Review finding (Task 11) DISPROVEN: a reviewer claimed the C++ ZIP lister
    (libarchive, `ArchiveService.cpp:459-474`, which pushes every pathname with
    no `AE_IFDIR` skip) includes directory entries and that skipping `is_dir()`
    only "coincidentally" reproduces the golden because "no corpus zip has
    directory entries". Both premises are false. FOUR corpus zips DO store
    explicit directory entries (`zip_11step_cbbe_3ba`, `zip_atmostone_heels_srd`,
    `zip_exactlyone_mu_joint_fix`, `zip_exactlyone_racecompat`), yet the
    C++-generated `archive_entries.json` for every one of them contains ZERO
    directory markers (cbbe: 466 golden entries vs 1156 stored; the 690
    directories are absent). libarchive does not surface these zip directory
    markers, so skipping `is_dir()` is the FAITHFUL reproduction of the C++
    output; including them (verified) breaks the byte-parity on all four. The
    skip is kept and now carries a code comment explaining the empirical basis.
- 7z (`sevenz-rust2`): header-only `Archive::open`; skip `is_directory()`;
  convert `/` -> `\` on each name (bit7z reports backslashes for 7z); then a
  STABLE case-insensitive sort by the backslash path. `sevenz-rust2` returns raw
  header order, which differs from bit7z on ~5/9 fixtures; the sort fixes all 9.
- RAR (`unrar`): `open_for_listing`; skip `is_directory()`; keep unrar native
  backslash `filename` (matching bit7z); STABLE case-insensitive sort by the
  backslash path (fixes the large `sos` archive).
- `EntryListing.sizes` keys are the Full-normalized path (`utils::normalize_path`
  = lowercase + `\`->`/` + strip leading `./` and `/`); `EntryListing.paths`
  keeps the per-format original-casing string. The serialized golden is the
  paths+sizes pairing (`sizes[normalize_path(path)]` per listed path); the test
  compares against that.

### Order-parity ASSUMPTION (7z / rar)

The case-insensitive sort matches bit7z BECAUSE 7-Zip / WinRAR store entries
case-insensitively sorted, so their listers echo that order. An archive authored
with an unsorted central directory would list in a different order and diverge
from this port (and from bit7z). No such archive exists in the corpus; the
assumption holds for all 9 7z/rar fixtures. The ZIP path makes no such
assumption (central-directory order is preserved verbatim).

### build.rs Win32 link requirement for unrar_sys

`unrar_sys` 0.5.8 compiles the unRAR C++ with `cc` but does NOT emit link
directives for the Win32 import libraries the sources need. Without them the
final link fails with ~13 LNK2019 unresolved externals (`RegOpenKeyExW`,
`CryptAcquireContextW`, `OpenProcessToken`, `AdjustTokenPrivileges`,
`SetFileSecurityW`, ...). `build.rs` emits
`cargo:rustc-link-lib=advapi32` and `cargo:rustc-link-lib=user32`, gated to
Windows via `CARGO_CFG_TARGET_OS` (the correct TARGET signal in a build script).

### unRAR license restriction (freeware with a use limitation)

The `unrar_sys` vendor sources are the official unRAR sources: freeware that may
be used to read / decompress RAR archives but MUST NOT be used to recreate the
RAR compression algorithm. salma only READS RAR archives (listing, single-entry
read, extraction), so it is compliant. There is no NOTICE / THIRD-PARTY file in
the repo to annotate; this note is the record. If a NOTICE file is added later
(e.g. Task 17 packaging), copy this restriction there.

### 256 MiB cap + traversal-rejection semantics

- Cap (`MAX_ENTRY_SIZE = 256*1024*1024`, value equal to `kMaxEntrySize`): the
  in-memory read paths (`read_entry`, `read_entries_batch`) reject any entry
  whose header uncompressed size exceeds the cap BEFORE allocating. The C++
  guard is `size < 0 || size > kMaxEntrySize` over a signed int64; header sizes
  here are `u64`, so the negative branch is unreachable and the guard is
  `size > MAX_ENTRY_SIZE as u64`, which still rejects forged multi-gigabyte
  sizes.
  - Scope divergence (review finding, Task 11): C++ applies `kMaxEntrySize`
    ONLY on the libarchive/zip read fallback (`read_entry` `:680`,
    `read_entries_batch` `:858`); the bit7z branch that handles `.7z`/`.rar`/
    `.001` (`:624-651`, `:717-822`) allocates the full decompressed entry with
    NO cap. This port applies the cap UNIFORMLY to all three backends
    (`read_entry_zip`/`_7z`/`_rar`, `read_batch_zip`/`_7z`/`_rar`), so it is
    STRICTER than C++ for 7z/rar: a contested loose entry larger than 256 MiB
    inside a 7z/rar is read + FNV-hashed by C++ but returned empty / omitted
    here, a latent inference divergence for such a mod. Kept deliberately: it is
    safety-positive (a decompression-bomb guard on the untrusted 7z/rar path
    C++ leaves uncapped) and practically unreachable (single >256 MiB loose
    files in FOMOD content are essentially unheard of). Documented rather than
    "fixed" by removing the guard, which would trade a real safety property for
    parity on an unreachable case.
  - Wiring proof (review finding, Task 11): a real over-cap archive is not
    cheaply constructible in a unit test (the zip/7z writers record the true
    uncompressed size, and `MAX_ENTRY_SIZE` is a non-injectable `pub const`), so
    the cap is proven by a focused boundary test on the extracted
    `exceeds_entry_cap` predicate plus a constant assertion; the cap-BEFORE-
    allocation WIRING inside each read helper rests on code inspection, not an
    executable oracle. The related `extract` pre-allocation is now clamped
    through `prealloc_hint` (below), which IS unit-tested.
- `extract` pre-allocation clamp (review finding, Task 11): `extract`/
  `extract_filtered`/`extract_prefix` intentionally have NO 256 MiB rejection
  (parity with the uncapped C++ streaming extract, `ArchiveService.cpp:330-339`),
  but `extract_zip` used to size its buffer `Vec::with_capacity(entry.size())`
  directly from the archive-controlled uncompressed size (zip64 `u64`). A forged
  huge value would force an unbounded up-front allocation - an uncatchable
  `handle_alloc_error` abort, or a capacity-overflow panic beyond `isize::MAX` -
  BEFORE any data is read, violating the "malformed archive -> Err, never a
  panic/abort" contract. Fixed with `prealloc_hint(size) = min(size,
  MAX_ENTRY_SIZE)`: the capacity hint saturates at the cap while `read_to_end`
  still grows the buffer to the real size, so a legitimate large entry extracts
  and a forged size cannot abort. Pinned by `prealloc_hint_clamps_to_cap`. The
  7z extract path already used `Vec::new()`; rar uses `unrar`'s own buffer.
  Secondary memory-model note: all three Rust extract backends read whole
  entries into memory (`read_to_end`) rather than streaming block-by-block like
  the C++ `copy_data`, so a legitimate multi-GB entry is fully buffered in RAM;
  acceptable for FOMOD content, revisited only if Task 16 surfaces a giant loose
  entry.
- Traversal (mirror of `ArchiveService.cpp:94/305/529`): each entry path is
  joined onto the canonical destination and rejected (skipped, no write) when
  the result is not inside the destination. Implemented via `utils::is_inside`
  (weakly-canonical + component-wise `starts_with`, already ported in Task 3),
  which is the exact `weakly_canonical(dest/entry).lexically_relative(dest)`
  empty-or-`..` check. Rejects `../x`, `..\x`, absolute `/x`, drive-letter
  roots, and sibling-prefix escapes (`dest-evil/x` when extracting to `dest`).
  Two tests pin it (review finding, Task 11 - the earlier single test asserted
  the absolute-escape at `out/evil_abs.txt`, a path neither a correct nor a
  buggy build ever writes, so it was vacuously true, and it had no sibling-prefix
  case): `extract_rejects_path_traversal_entries` builds a malicious zip
  (relative `..`, both separators, a rooted name, AND a sibling-prefix
  `../out-evil/*`) and asserts only the benign file lands inside plus nothing at
  the sibling location; `safe_output_path_rejects_all_escape_classes` asserts the
  guard directly and deterministically for every class INCLUDING the absolute
  drive-root escape (`/x` -> `C:\x`), which a walk of the destination tree cannot
  observe.

### Normalization profile discrepancy (hpp doc table vs implementation)

The `ArchiveService.hpp` doc table claims a "Light" profile (lowercase +
`\`->`/`, no strip) for the libarchive `read_entry` / `read_entries_batch`
paths. The IMPLEMENTATION calls `normalize_entry_path` (== `normalize_path`,
the Full profile) uniformly on both backends and both sides of every match.
This port matches the implementation (Full everywhere), not the stale doc table.
This is a C++ documentation bug, not a code bug; per the task rules the C++ is
left untouched and only recorded here.

### Simplifications vs the C++ (noted, deferred to Task 16 round-trip)

- Extraction metadata. The C++ preserves timestamps/permissions/ACLs on the
  full `extract` and timestamps on `extract_filtered`. This port writes file
  CONTENTS only (bytes + parent dirs), dropping the timestamp/permission
  distinction. Extraction CORRECTNESS (byte-identical trees) is validated by the
  Task 16 round-trip; the strong Task 11 tests are on listing parity, read_entry,
  and traversal rejection.
- `extract_filtered` backend. The C++ runs `extract_filtered` over a single
  libarchive pass for ALL formats; this port routes it per backend (zip / 7z /
  rar) but keeps the observable contract (the filter receives each raw entry
  path and decides what is written).
- `extract_prefix` entry normalization is FORMAT-AWARE (review finding, Task 11).
  The C++ uses two different profiles: the bit7z 7z/rar path
  (`ArchiveService.cpp:584`) normalizes the entry with Full `normalize_entry_path`
  (strips a leading `./` and `/`), while the libarchive ZIP fallback (`:615-617`)
  uses Light (lowercase + `\`->`/`, NO strip). An earlier revision normalized
  uniformly with Full, which for ZIP matched strictly more entries than the C++
  (any stored `./x` or `/x`). The port now mirrors both: `prefix_entry_norm`
  applies Light for ZIP and Full for 7z/rar. Impact is limited to ZIP entries
  whose stored name begins with `./` or `/` (rare), but it is a genuine C++
  behavioral match, pinned by
  `extract_prefix_entry_norm_light_for_zip_full_for_7z_rar`.
- `create_zip` entry separators. The C++ stores `fs::relative(...).string()`,
  which is backslash-separated on Windows; this port stores forward-slash entry
  names (portable, standard for zip). The created zip is re-read by the same
  engine (which normalizes paths), so this does not affect round-trips.
- `read_entries_batch` solid strategy. The C++ extracts a solid 7z matched
  subset to a temp dir and reads it back (to avoid re-decoding the block per
  random access). This port instead decodes the block ONCE via
  `ArchiveReader::for_each_entries` and streams every requested entry from that
  single pass - the strategy the evaluation prescribes; no temp dir needed.
  CRITICAL correctness dependency (review finding, Task 11 - see the "Solid-block
  stream alignment" section below): every entry `for_each_7z_entry` yields MUST
  be fully drained before the block decoder advances, because `sevenz_rust2`
  layers each file's reader over ONE shared per-block decode stream with no
  Drop/auto-skip. `read_batch_7z` also early-stops once every requested entry is
  collected, mirroring the C++ `remaining` early-out (`:848`).

### Solid-block stream alignment (review finding, Task 11 - CRITICAL fix)

The 7z read/extract helpers (`read_entry_7z`, `read_batch_7z`, `extract_7z`) all
funnel through `for_each_7z_entry`, a thin wrapper over
`sevenz_rust2::ArchiveReader::for_each_entries`. Inside a SOLID 7z block (the 7z
default, and the dominant real-mod format) `sevenz_rust2` hands EVERY file in the
block a `BoundedReader` (optionally wrapped in a `Crc32VerifyingReader`) layered
over ONE shared decode stream, and that `BoundedReader` has NO `Drop` /
auto-skip (crate `reader.rs`): the shared stream only advances by bytes the
closure actually reads. The crate's own `read_file` `read_to_end`s EVERY entry
for exactly this reason.

The initial port's skip branches returned `Ok(true)` WITHOUT draining the reader
(non-matching entries in `read_entry_7z`/`read_batch_7z`, and filtered-out /
traversal-skipped entries in `extract_7z`). Skipping a preceding entry without
draining left the shared stream mid-file, so the NEXT kept entry decoded from a
misaligned offset -> wrong bytes, or (when the file carries a CRC) a
`Crc32VerifyingReader` failure surfacing as empty/absent/`Err`. EMPIRICALLY: on
the solid corpus 7z `sevenz_3step_tk_dodge` (23 files, `fomod\info.xml` precedes
`fomod\ModuleConfig.xml` in the block), `read_entry("fomod/moduleconfig.xml")`
returned 0 bytes vs the golden 5910; `read_entries_batch` of only the config
returned it absent; `extract_prefix("meshes")` failed with
`ChecksumVerificationFailed`. This is exactly the call Task 12 makes to load
`ModuleConfig.xml`, so inference would have silently failed for solid 7z mods.
First-in-block entries and full `extract()` (which reads every entry in order)
masked the bug; ZIP (random-access `by_index`) and RAR (`unrar` skip advances
internally) are unaffected.

Fix: `for_each_7z_entry` now fully drains each entry's reader
(`std::io::copy(rd, &mut std::io::sink())`) after the closure returns, whenever
iteration continues (skipped only when stopping early, where no further entry is
decoded) - mirroring `read_file`. `read_entry_7z` stops as soon as the target is
read; `read_batch_7z` stops once all requested entries are collected. Pinned by
the new corpus-gated `read_and_extract_match_committed_module_config_over_real_corpus`
in `tests/archive_service_fixtures.rs`, which for every case shipping a committed
`ModuleConfig.xml` and a present source archive reads that config back via
`read_entry` AND `read_entries_batch` AND `extract_prefix` and byte-compares each
to the golden bytes (extract_prefix size-gated for solid 7z above 50 MiB). This
closes the coverage gap that let the bug ship: the previous read/extract tests
used only in-memory ZIP, which never exercises the solid-block stream.

### .001 routing is UNTESTED

`.001` multi-volume archives route to the 7z backend (bit7z handled them). NO
`.001` archive exists in the corpus, so this routing path is implemented but
never exercised. `sevenz-rust2` may or may not handle split volumes identically
to bit7z; revisit if a `.001` fixture is ever added.

### Release-build toolchain flakiness (CI note for Task 16/17)

Adding `unrar_sys` means every fresh build/check profile recompiles the unRAR
C++ with `cl.exe` via `cc`. On this host `cl.exe` INTERMITTENTLY crashes with an
access violation (exit `0xc0000005`) mid-compile - observed once during a
`cargo clippy` run, which then passed on immediate retry. This is the same
host-toolchain instability already noted for release builds (rustc ICE /
`cl.exe` access-violation). The gates run in DEBUG (`cargo test` / `clippy`
default); RELEASE builds must NOT be run in the gates. CI (Task 16/17) should
expect the occasional `unrar_sys` compile crash and retry, and should avoid
release builds until the toolchain instability is resolved.

Tests added (Task 11): `src/archive_service.rs` always-run unit tests
(`use_bit7z`/format routing, the cap constant + boundary guard, `prealloc_hint`
clamp, `prefix_entry_norm` Light-vs-Full, in-memory zip listing that strips a
dir entry, case-insensitive `read_entry` + missing, `read_entries_batch` subset,
the strengthened traversal-attack extraction, the direct
`safe_output_path_rejects_all_escape_classes`, and a `create_zip` round-trip) and
`tests/archive_service_fixtures.rs` (two corpus-gated oracles:
`list_entries_with_sizes_matches_golden_over_real_corpus`, the 16/16 byte-parity
listing oracle; and `read_and_extract_match_committed_module_config_over_real_corpus`,
the read/extract oracle across the 7z/rar/zip backends - both auto-skipped on CI
where no source archive is present).

## Task 12 - Target-tree scan + inference orchestration (tier-1 meta.ini)

Milestone 6. `src/FomodInferenceService.hpp`/`.cpp` (1333 LOC) ->
`src/fomod_inference_service.rs`. This is the integration capstone: it owns the
stages no earlier task covered (installed-file scan, contested-file hashing with
the bounded cache, the Tier-1 `meta.ini` shortcut, `compute_overrides`) and
sequences every previously ported stage behind `infer_selections`. The
`inferFomodSelections` export in `capi.rs` now calls it instead of returning the
Milestone-1 stub.

### API mapping (C++ -> Rust)

| C++ | Rust |
| --- | --- |
| `FomodInferenceService::infer_selections` | `FomodInferenceService::infer_selections` |
| `FomodInferenceService::scan_installed_files` (static) | `scan_installed_files` (free fn) |
| `FomodInferenceService::hash_contested_files` | `FomodInferenceService::hash_contested_files` |
| `FomodInferenceService::try_fomod_plus_json` (static) | `try_fomod_plus_json` (free fn) |
| `FomodInferenceService::compute_overrides` (static) | `compute_overrides` (free fn) |
| anon-ns `find_contested_dests` | `find_contested_dests` |
| anon-ns `build_archive_signature` | `build_archive_signature` |
| anon-ns `fetch_entry_hashes` | `FomodInferenceService::fetch_entry_hashes` (method: needs the cache) |
| anon-ns `apply_entry_hashes` | `apply_entry_hashes` |
| the inline Tier-1 block (`:984-1258`) | `try_tier1_cache` + `build_tier1_json` |
| `InferenceContext` | locals in `infer_selections` (no context struct needed) |

The statics become free functions so the fixture tests can drive each stage
directly; `fetch_entry_hashes` stays a method because the C++ passes the mutex
and map in by reference, which a `Mutex<HashMap>` field expresses directly.

### Exceptions-return-empty contract

The C++ wraps the whole pipeline in `try { ... } catch (const std::exception&)
{ return ""; }` (`:1325-1330`), and every ABI caller adds `catch (...)` on top.
The Rust has no exceptions: each failure point (archive missing, mod missing, not
a FOMOD, XML read miss, XML parse error) returns `String::new()` directly, and
`capi::inferFomodSelections`' `guard()` supplies the panic firewall. No `Result`
crosses FFI. The two `fs::exists` checks are OUTSIDE the C++ `try` (`:787-794`)
and are correspondingly the first two statements of the Rust fn.

### `fully_resolved` is solver-internal (NOT a service branch)

The plan text says "If fully resolved, the CSP solve is skipped", but the C++
service does not branch on `propagation.fully_resolved` at all - it always calls
`solve_fomod_csp`, and the fully-resolved skip/seed logic lives INSIDE the solver
(Task 9). The only service-level use of the propagation result is which argument
to pass: `None` vs `Some(&propagation)`, decided by
`propagation.resolved_groups.is_empty()`. The port matches the code, not the
plan prose.

### The entry-size lookup under-population (a C++ bug, faithfully reproduced)

`infer_selections` builds `norm_entry_sizes` with (`:836-848`):

```cpp
auto sz_it = entry_sizes.find(entry);   // `entry` is the ORIGINAL archive path
if (sz_it != entry_sizes.end())
    ctx.norm_entry_sizes[norm] = sz_it->second;
```

The inline comment claims "Look up size using the original archive path (how
listing.sizes is keyed)", but that comment is WRONG: `ArchiveService` keys
`listing.sizes` by `normalize_entry_path(path)` (`ArchiveService.cpp:424,469`),
i.e. by the NORMALIZED path, while `listing.paths` keeps the original strings.
So the lookup only succeeds for entries whose raw path already equals its
normalized form - any entry carrying an uppercase letter or a backslash silently
gets NO size, and its atoms keep `file_size == 0`.

That is not cosmetic: `find_contested_dests` treats a zero `file_size` as a
size-compatibility WILDCARD (`a.file_size == 0 || target_file.size == 0 || ...`),
so the under-population widens the contested set. The port reproduces the same
lookup against the same normalized-keyed map (`fomod_inference_service.rs:133-144`)
rather than "fixing" it, because the corpus gate demands the C++ result. Do NOT
correct this without re-baselining the whole corpus.

### Per-stage timing windows (review finding - FIXED)

`diagnostics.timings_ms.{list,scan,solve}` each measure ONE stage in the C++,
which resets a shared `t_step` immediately before each (`:824` list, `:945` scan,
`:1283` solve) and only `total_ms` runs from `t_total`. The `t_scan` window
covers `scan_installed_files` AND `build_target_tree`, not just the walk.

The initial port derived `t_list` and `t_scan` from `t_total.elapsed()`, making
both CUMULATIVE: `list_ms` also counted the two existence checks and the whole
Tier-1 `meta.ini` read+parse, and `scan_ms` additionally counted listing, the XML
read, the XML parse and atom expansion. On a large archive that reported e.g.
`{list: 3200, scan: 3600}` where the C++ reports `{list: 3200, scan: 180}`.
`compare_infer.py` ZEROES all four timings before comparing, so the corpus gate
could never have caught this. Fixed: each stage now times from its own
`Instant::now()`.

### Tier-1 `meta.ini` shortcut

`try_fomod_plus_json` reproduces the INI quirks exactly, each test-pinned:
10000-LINE cap (`++line_count > 10000`, so a key on line 10000 is read and one on
10001 aborts); `[Settings]` gating that is case-insensitive AND turns tracking
back off on any other `[...]` header; whole-line trim of `" \t\r\n"` but
key/value trim of only `" \t"`; a single outer quote-pair peel; the
`""` / `{}` / `"{}"` rejects; first-matching-key-wins whether or not the value
parses; and the `steps` must be a non-empty array.

Two byte-level details drove the reader to raw bytes rather than a decoded
string:

- **Line splitting is `std::getline`, not `str::lines()`.** `getline` yields one
  line per `\n`-terminated segment plus a final unterminated segment only when
  non-empty; a plain `bytes.split(b'\n')` adds a spurious trailing empty line for
  the usual newline-terminated file and would shift the 10000-line cap by one.
  `getline_split` implements the `getline` rule; the existing cap-boundary test
  pins it.
- **Strict UTF-8 (review finding - FIXED).** The C++ hands raw bytes to
  `nlohmann::json::parse`, which REJECTS ill-formed UTF-8 (parse_error 316), and
  the surrounding catch turns that into a Tier-1 MISS. The initial port decoded
  the whole file with `String::from_utf8_lossy`, so a Windows-1252 byte became
  U+FFFD and the blob PARSED - flipping a miss into a hit and emitting a
  completely different document. The value is now strict-validated with
  `std::str::from_utf8` before parsing.

`try_tier1_cache` name-resolves the cached blob against the IR into a
`[step][group][plugin]` grid, forward-simulates it with the same atoms and
overrides the solver path uses, and accepts it only on an EXACT reproduction
(`compare_trees(...).exact()`). `build_tier1_json` then emits a BESPOKE schema-v2
document (`phase_reached: "tier1_cache"`, `cache.hit: true`,
`cache.source: "fomod-plus"`, every confidence 1.0, `resolved_by:
"cache.fomod_plus"`), NOT `assemble_json`'s.

#### `Tier1Outcome::Abort` - malformed names fail the WHOLE call (review finding - FIXED)

The C++ reads step and group names with `src_step.value("name", "")`. That call is
NOT total: `nlohmann::basic_json::value(key, default)` throws `type_error 306`
when the receiver is not an object, and `type_error 302` when the key IS present
but is not a string (only an ABSENT key uses the default). Those throws escape to
`infer_selections`' outer catch, so `inferFomodSelections` returns `""` for the
whole call. Note the asymmetry: PLUGIN names go through the `plugin_name_of`
lambda, which guards with explicit `is_string()`/`is_object()` checks and is
therefore fully tolerant.

The initial port flattened both into `.get("name").and_then(as_str).unwrap_or("")`
and returned a full inference document where the C++ returns nothing. `name_field`
now reproduces `value()`'s exact tri-state and `try_tier1_cache` returns a
three-way `Tier1Outcome` (`Hit` / `Miss` / `Abort`) instead of an `Option`, with
`Abort` mapping to `""` at the call site. Ordering matters and is preserved: the
C++ evaluates the step name BEFORE the `find_step` lookup that may break the
loop, so every step up to and including the first unresolvable one is
name-checked; group names are only reached for a step that already resolved.

### `json::parse` hardened to nlohmann's grammar (review findings - FIXED)

`json::parse` has exactly ONE production call site - the `meta.ini` fomod-plus
blob - so every leniency in it is a Tier-1 hit/miss flip, which changes the
entire emitted document. It was a permissive hand-rolled parser; it is now strict:

- **Depth cap `MAX_PARSE_DEPTH = 512` (CRITICAL).** nlohmann's parser is
  ITERATIVE (a heap `std::vector<bool> states`; `destroy()` is iterative too), so
  it parses arbitrarily deep input and simply returns a value or a clean
  parse_error. The Rust parser is recursive descent with no bound. Measured on
  this host, a release build survived depth 2000 and died at depth 3000 with
  `STATUS_STACK_OVERFLOW` (exit `0xC0000FD`). A Windows stack overflow is an SEH
  exception, NOT a Rust panic, so `capi`'s `catch_unwind` cannot contain it: the
  HOST process (MO2, or `mo2-server.exe`) would be killed where the C++ DLL
  returns a normal document - and the input is a file in the mods tree. The cap
  turns that into an `Err` -> Tier-1 miss, which is the same OBSERVABLE outcome
  the C++ reaches for any such blob (a real fomod-plus document nests ~5 levels;
  anything deeper can never name-resolve). Same guard class as the ported
  `MAX_ELEMENT_DEPTH` (XML, 48) and `MAX_DEPENDENCY_DEPTH` (condition trees, 32).
  This also removes the secondary hazard that `Value`'s derived `Drop` and
  `write_pretty` are recursive: a value that deep can no longer be built.
- **Strict number grammar.** The old scanner accepted any run of `-+.eE0-9` and
  deferred to Rust's `FromStr`, which accepts a leading `+`, leading zeros, `.5`
  and `1.` - all parse_error 101 in nlohmann. The parser now implements RFC 8259
  directly (`-? (0 | [1-9][0-9]*) ('.' [0-9]+)? ([eE] [+-]? [0-9]+)?`).
- **`u64` integers.** nlohmann stores an integer above `i64::MAX` as
  `number_unsigned_t` and dumps the exact digits; the port degraded to `f64` and
  printed a mangled float. Added `Value::UInt(u64)`, produced ONLY by `parse` (the
  assembly path still builds `Value::Int` everywhere, so no existing output can
  shift). This is reachable: the Tier-1 emitter echoes a cached `deselected`
  entry's RAW `name` value into the output, and `deselected` is never
  name-resolved.
- **Raw control bytes rejected.** A byte `< 0x20` inside a string is parse_error
  101 in nlohmann; it must be escaped.
- **`\uXXXX` requires four hex digits.** `u32::from_str_radix(s, 16)` accepts a
  leading `+`, so `\u+123` used to decode.

### Accepted divergences (Task 12)

Each of these was confirmed against the C++ and deliberately NOT "fixed":

- **Hash-cache signature encoding.** `build_archive_signature` is
  `<canonical_path>|<size>|<mtime>`. The C++ uses `weakly_canonical` +
  `last_write_time().time_since_epoch()` (filesystem-clock ticks); the port uses
  `fs::canonicalize` (falling back to the raw path) and nanoseconds since the Unix
  epoch. Only the self-invalidate-on-size-or-mtime property is required, and the
  signature NEVER reaches the output - it is a per-instance cache key. The cache
  itself is byte-faithful: bound `kMaxCacheEntries = 100000`, clear-all (not LRU)
  when exceeded, with the check and the clear under ONE lock.
- **Directory-walk error recovery.** The C++ uses
  `recursive_directory_iterator` with `skip_permission_denied` AND the
  `error_code` overload, so an I/O error mid-walk does NOT throw: the iterator
  ends, a warning is logged, and `scan_installed_files` returns whatever it
  collected so far (the inference then continues with a PARTIAL target tree). The
  Rust walk uses an explicit stack and skips only the unreadable directory,
  continuing with its siblings. Both yield a partial scan on error; they differ in
  WHICH files survive. Exact parity here is unattainable anyway - the two walks
  visit directories in different orders, so "what was collected before the error"
  could not match even with identical stop semantics. Requires a real I/O error
  (e.g. a path over MAX_PATH in a non-long-path-aware host) to observe.
- **Symlink relativization.** The C++ relativizes with `fs::relative`, which
  routes through `weakly_canonical` and therefore RESOLVES symlinks; the Rust
  strips the prefix lexically (`strip_prefix`). For a mod dir containing a file
  symlink that points outside the mod root, the C++ derives a key from the
  resolved target (often escaping the root) while the Rust derives it from the
  lexical path. No corpus mod uses symlinks; MO2 deploys real files.
- **MSVC text-mode CTRL+Z.** The C++ opens `meta.ini` with a default
  `std::ifstream` (TEXT mode on MSVC), so a `0x1A` byte TRUNCATES the read and any
  fomod-plus key after it is invisible -> Tier-1 miss. The Rust reads raw bytes and
  reads past it. Reproducing this would mean emulating MSVC text-mode translation
  for a byte no real `meta.ini` contains.
- **Timing VALUES are wall-clock** and can never be byte-equal; `compare_infer.py`
  zeroes `diagnostics.timings_ms.*` on both sides. The per-stage WINDOWS now match
  (see above), which is the part that was actually portable.

### `compute_overrides` determinism

Both C++ containers here are unordered, but neither ordering leaks: the
conditional rule tests `producers.size() == 1 && producers.count(ci)`, a
set-cardinality predicate, and the step rule tests `dest_step_count[d] == 1`, a
per-dest counter - both order-independent. The one order-sensitive detail is the
FLAT per-plugin walk (`step -> group -> plugin`, incrementing `flat_idx`), which
must match `expand_all_atoms`' per-plugin index order; the port walks the IR in
the same nesting order and BOUNDS-CHECKS `flat_idx` against `atoms.per_plugin.len()`
(skipping, never panicking) where the C++ would index out of range on an
IR/atom desync.

### Gate results

- `compare_infer.py --curated` (16 committed cases): 1 EXACT, 15 METRICS_EQUAL,
  0 DIVERGE, 0 SKIP.
- `compare_infer.py` full corpus (197 fixtures): 153 EXACT, 44 METRICS_EQUAL,
  0 DIVERGE, 0 SKIP.

The split is structural, not a quality signal: the 153 EXACT cases are the
inferences that return `""` on both sides, and the 44 METRICS_EQUAL cases are
every NON-EMPTY inference - each carrying wall-clock `timings_ms` plus the
`UNIQUE_FILE_EVIDENCE` `reasons[].detail.files` set (the documented Task-7
`unordered_set` ordering divergence). Byte-equality is unreachable for a
non-empty output BY CONSTRUCTION; METRICS_EQUAL is the strongest attainable
result for those, and both sanctioned divergences are the only ones
`compare_infer.py` will tolerate before reporting DIVERGE.

### Review findings and fixes (Task 12 review pass)

An adversarial review (6 dimensions x parallel reviewers, each finding then
attacked by 3 independent refuters on correctness / reachability / C++-fidelity
lenses; 25 raw findings, 16 surviving, 9 refuted) produced the fixes recorded
above. Grouped by root cause:

1. CRITICAL - unbounded `json::parse` recursion -> host-process stack overflow.
   Empirically reproduced at depth 3000 in a release build. Fixed with
   `MAX_PARSE_DEPTH`.
2. IMPORTANT - `timings_ms.list` / `.scan` cumulative instead of per-stage.
   Fixed; invisible to the corpus gate, which zeroes timings.
3. IMPORTANT - Tier-1 non-string / non-object step and group `name` coerced to
   `""` instead of failing the call. Fixed with `Tier1Outcome::Abort`.
4. IMPORTANT - `json::parse` number and string leniency (leading zeros, `+`,
   `.5`, `1.`, raw control bytes, signed `\u`). Fixed.
5. MINOR - `u64`-range integers degraded to floats. Fixed with `Value::UInt`.
6. MINOR - `meta.ini` decoded lossily instead of strict UTF-8. Fixed.
7. The scan walk-error, symlink and CTRL+Z findings were confirmed as real but
   deliberately accepted; see "Accepted divergences" above.

Refuted (recorded so they are not re-raised): the Tier-1 `total_ms` capture point
(the C++ also stamps it before the emitter runs); the "10000-line cap no longer
bounds anything" claim (the cap is a LINE cap in both, and neither language caps
line length); allocation-failure recovery (the C++ `std::bad_alloc` path is
equally fatal in practice); and five test-coverage findings that named branches
already covered by the corpus-gated fixtures.

### Tests added (Task 12)

`src/fomod_inference_service.rs` unit tests: `compute_overrides` (4 cases:
conditional unique/shared, step unique/shared, excluded+absent dests); the
`meta.ini` INI quirks (9 cases incl. the 10000-line cap boundary, `[Settings]`
gating, quote peel, empty-form rejects, first-match-wins); the new
strict-UTF-8, strict-grammar and depth-cap rejections; the four
`Tier1Outcome::Abort` / miss cases; and `scan_installed_files` recursion +
missing-dir. `src/json.rs`: strict number grammar, `u64` round-trip, raw control
bytes / signed `\u`, and the depth cap (at the cap, past it, a 100k-deep hostile
blob, and a wide-but-shallow document proving depth is per-path).
`tests/fomod_inference_service_fixtures.rs` drives the whole orchestration over
the committed corpus cases. The real gate remains `tools/compare_infer.py`.

## Task 13 - Port the fomod_inference GoogleTest suite

`tests/fomod_inference_test.cpp` (1107 LOC, 15 cases) ->
`tests/fomod_inference_test.rs`. All 15 ported 1:1, each keeping the C++ behavior
name in snake_case (`SelectAll_Deterministic` -> `select_all_deterministic`) and
appearing in the C++ file's order.

### Scope: which remaining C++ suites are in this task

`tests/` holds four GoogleTest files. `utils_test.cpp` and
`inference_diagnostics_test.cpp` were already ported (Tasks 3 and 10).
`security_context_test.cpp` is OUT of scope: `SecurityContext` lives in mo2-core
only so the tests can link it without Crow, but it is consumed exclusively by the
HTTP server layer, and no `security_context` module appears in the port's
architecture target. The DLL being replaced does not export it. That leaves
`fomod_inference_test.cpp` as the whole of Task 13.

### Fixtures stay inline

The C++ suite builds every fixture inline from an XML string and never reads
`tests/`; the port does the same, so the file runs on CI with no corpus present.
The three C++ file-static helpers are ported at the top of the Rust file:

| C++ helper | Rust |
| --- | --- |
| `parse_xml(xml, prefix = "")` | `parse_xml(xml: &str, prefix: &str)` |
| `build_atoms(installer, file_size = 100)` | `build_atoms(&FomodInstaller, u64)` |
| `build_target(paths, file_size = 100)` | `build_target(&[&str], u64)` |

`build_atoms` is deliberately NOT the production `expand_all_atoms`: it fabricates
one atom per file entry at a uniform size, walking required -> per-plugin (flat)
-> per-conditional with `document_order` drawn from a SINGLE counter across all
three passes. Several cases assert on conflict resolution, which depends on that
exact ordering. C++ default arguments have no Rust equivalent, so call sites spell
out the defaults via the `DEFAULT_SIZE = 100` const and an explicit `""` prefix.

### Mechanical mappings applied throughout

- `EXPECT_*` and `ASSERT_*` both become `assert!` / `assert_eq!`. Rust has no
  non-fatal assertion, so every `EXPECT_` is STRENGTHENED to fatal. This can only
  turn a multi-failure report into a first-failure report; it can never let a
  failing assertion pass.
- `ASSERT_EQ(x.size(), 3u)` -> `assert_eq!(x.len(), 3)`; both sides are `usize`,
  so the `u` suffix is dropped.
- `EXPECT_TRUE(sim.files.count(d))` -> `assert!(sim.files.contains_key(d))`
  (`count` on a `std::unordered_map` is 0/1).
- Raw pointers for optional arguments (`nullptr` context, `&overrides`) become
  `Option<&T>` (`None` / `Some(&overrides)`).
- `InferenceOverrides` is default-constructed then `.assign(n, Unknown)`-ed in
  C++; the port uses a struct literal with `vec![ExternalConditionOverride::Unknown; n]`.
  Struct-literal field order is not semantically meaningful, so the literals keep
  the C++ STATEMENT order (`step_visible` first) even though the Rust struct
  declares `conditional_active` first.
- The gtest `<<` streamed failure message becomes the `assert!` message argument,
  text preserved.

### Verification that nothing was weakened

Two mechanical checks, both run against the committed files:

- **Assertion count per case**: 63 assertions in the C++ suite, 63 in the Rust
  suite, and every one of the 15 cases matches its counterpart EXACTLY (no case
  gained or lost an assertion).
- **XML fixtures**: the suite has 10 XML literals; all 10 are identical between
  `R"(...)"` and `r#"..."#` after whitespace normalization, with no literal
  present on only one side. (The other 5 cases build IR structs directly or call
  `expand_entry`, so they carry no XML.)

### `PropagatorFlagPropagation` documents a mechanism the code does not implement

Worth recording because the suite is otherwise readable as documentation of
propagator behavior. The C++ case's comment block states that `BasicPatch`
"becomes NotUsable" via flag propagation and the group therefore resolves on
iteration 2. It does not. Rule 1 of `propagate` carries the guard

```cpp
bool dynamic_without_context = (!context && !plugin.type_patterns.empty());
if (eff == PluginType::NotUsable) { if (!dynamic_without_context) { ... } }
```

(`FomodPropagator.cpp:140-143`, ported verbatim at
`fomod_propagator.rs:230-233`), and `BasicPatch` has non-empty `type_patterns`
with `context == nullptr`, so the NotUsable outcome is explicitly NOT allowed to
prune. What actually eliminates `BasicPatch` is rule 2 (file evidence): its unique
dest `basic.esp` is absent from the target tree, so it takes `NoFileEvidence` and
the `SelectExactlyOne` group resolves on `usable_count == 1`.

Confirmed empirically during the port by adding `basic.esp` to the target tree:
the run then yields `fully_resolved == false` with domains `[[[true]], [[true,
true]]]`, i.e. the flag-driven NotUsable never prunes. Both languages carry the
same guard, so the assertions hold identically on both sides; the port keeps the
C++ case and its comment verbatim rather than rewriting either.

### Overlap with existing Rust tests (intentional, not redundant)

Some cases restate behavior already covered by earlier tasks' unit tests, e.g.
`ExpandEntry_FolderSkipsArchiveShippedMetaIni` overlaps
`fomod_inference_atoms.rs`'s `folder_branch_skips_top_level_meta_ini_only`, and
`ConditionalDest_FlagSetByLaterGroup_ReachesExact` overlaps the solver's
`lower_bound_skips_a_dest_a_later_group_can_still_produce`. The duplicates are
kept: the task's contract is a 1:1 port of the C++ suite, and keeping the C++
case names makes it possible to diff the two suites case-by-case when the C++
side changes.

### Test-suite delta

`cargo test`: 470 -> 485 (the 15 new cases). `cargo clippy --all-targets
-- -D warnings` and `cargo fmt --check` clean.

## Task 14 - FileOperations + FomodService install replay

Milestone 7 begins. `src/FileOperations.hpp`/`.cpp` ->
`src/file_operations.rs`, `src/FomodService.hpp`/`.cpp` ->
`src/fomod_service.rs`, and the `FileOperation` / `FileOpType` / `InstallResult`
structs from `src/Types.hpp` -> `src/types.rs`. This is the install REPLAY: it
turns a parsed `FomodInstaller` IR plus a JSON selections document into the
ordered `FileOperation` queue and copies the files. It shares the Task 5
dependency evaluator (`evaluate_condition` / `evaluate_plugin_type`) and Task 3
utilities (`is_safe_destination`, `to_lower`). The `install` /
`installWithConfig` exports still return the Milestone-1 stub - wiring the
services to the ABI is Task 15.

### `types.rs` additions

`FileOpType` (File/Folder, `#[default] File`), `FileOperation`
(op_type/source/destination/priority/document_order, all defaulting like the C++
aggregate), and `InstallResult` (success/mod_path/error). No behavior, pure data;
`FomodDependencyContext` and `PluginType` were already present from Tasks 3/5.

### FileOperations (`file_operations.rs`)

- **Two executors, two sort keys - both real, both reproduced.**
  `FileOperations::execute` (the instance/queue method) stable-sorts by
  `priority` ALONE (`FileOperations.cpp:59-62`) and leans on the stable sort to
  keep insertion order among equal priorities. `FomodService::execute_file_operations`
  (the free fn in `fomod_service.rs`) sorts by `(priority, document_order)`
  (`FomodService.cpp:691-698`). The FOMOD replay path uses the LATTER; the former
  exists for non-queued callers (Task 15's `InstallationService`). Rust
  `Vec::sort_by_key` / `sort_by` are stable, matching `std::stable_sort`. Pinned
  by `equal_priority_keeps_insertion_order_and_ignores_document_order` (the queue
  method ignores `document_order`) and `execute_sorts_by_priority_then_document_order`
  (the free fn uses it).
- **Non-throwing contract.** Every C++ entry point is documented "does not
  throw": each I/O step is wrapped in `try`/`catch (const fs::filesystem_error&)`
  that logs and returns-early-or-continues. The port returns `()` from every
  function and swallows `std::io::Error` at exactly the same points, so no
  `Result` and no panic reaches FFI.
- **Disk-full detection.** C++ compares the caught error against the portable
  `std::errc::no_space_on_device` (`FileOperations.cpp:24-27`); MSVC's Win32
  mapping folds both `ERROR_DISK_FULL` (112) and `ERROR_HANDLE_DISK_FULL` (39)
  into that. Rust has no `error_condition`, so `is_disk_full` tests
  `io::ErrorKind::StorageFull` (std maps the same two codes to it) OR the raw OS
  code, with the accepted list `cfg`-gated per platform (`{112, 39}` on Windows,
  `ENOSPC` 28 elsewhere) so a Windows `ERROR_OUT_OF_PAPER` (also 28) cannot be
  mistaken for a full disk. The sticky `g_disk_full` atomic becomes a `static
  AtomicBool` (Relaxed, matching the C++ memory order). Pinned by
  `is_disk_full_maps_the_platform_error_codes`; the set-from-I/O path cannot be
  provoked from a unit test (a real ENOSPC), so only the mapping is exercised.
- **`copy_folder` uses an explicit stack** in place of
  `fs::recursive_directory_iterator(src, skip_permission_denied)`. Parity points:
  a directory entry is created at its destination BEFORE its children are visited
  (pre-order), so empty subdirectories are reproduced; `DirEntry::file_type` does
  not follow symlinks (matches `entry.symlink_status()`), and symlinked entries
  are skipped; a `PermissionDenied` iteration error is skipped
  (`skip_permission_denied`), while ANY OTHER iteration error aborts the whole
  copy (the C++ outer `catch`); per-entry copy/mkdir errors are logged and the
  loop continues (the C++ inner `catch`). Sibling order within a directory is
  unspecified in both. Pinned by `copy_folder_reproduces_nested_tree_and_empty_dirs`,
  `copy_folder_skips_symlinks` (skips itself when the platform refuses symlink
  creation without privilege), `copy_folder_overwrites_existing_files_and_keeps_unrelated_ones`.
- **`copy_directory_contents` quirks reproduced:** NO source-existence check (a
  missing `src` still creates `dst`, then fails at iteration), and the
  `create_directories(dst)` failure path does NOT consult the disk-full flag even
  though the sibling `move_directory_contents` does. `fs::is_directory(entry)`
  FOLLOWS symlinks, so a symlink-to-directory takes the folder branch
  (`Path::is_dir` follows too). Pinned by
  `copy_directory_contents_creates_dst_even_when_src_is_missing`.
- **`move_directory_contents`:** `fs::rename` per child first, copy+remove
  fallback on ANY rename error EXCEPT disk-full (which sets the sticky flag and
  skips the child). Best-effort source cleanup mirrors `fs::remove_all` with an
  ignored error_code (symlink_status semantics: a symlink is unlinked as a link,
  never recursed). A real directory recurses via `remove_dir_all`; everything
  else - a real file, a file symlink, OR a directory symlink/junction, all of
  which report `is_dir() == false` under `symlink_metadata` - is unlinked with
  `remove_file(...).or_else(|_| remove_dir(...))`. The fallback is NOT cosmetic:
  a Windows directory reparse point is directory-attributed, so `remove_file`
  (DeleteFileW) cannot delete it - only `remove_dir` (RemoveDirectoryW) can (a
  Task 14 review finding; an earlier revision routed the directory-symlink case
  to `remove_file` alone and left the link behind on Windows). On Unix the
  `remove_file` unlink already removes any symlink, so the fallback never runs.
  Not wired into any path in Task 14 (Task 15's `InstallationService` uses it for
  the `unfomod -> mod_path` step); ported and tested now. Pinned by
  `move_directory_contents_falls_back_to_copy_when_rename_fails` and
  `move_directory_contents_removes_a_directory_symlink_child_on_fallback` (which
  skips where directory-symlink creation needs a privilege the session lacks).
- **Dropped logging (Task 17).** Every `Logger::instance().log*` call is dropped;
  the branch that produced it is kept with a `// dropped log site` comment so
  Task 17 restores it verbatim.

### FomodService (`fomod_service.rs`)

- **Exceptions become `Result`.** C++ `process_optional_files` and
  `validate_json_selections` read step/group names with `json.value("name", "")`,
  which THROWS `type_error 306` on a non-object element and `302` on a
  present-but-non-string `name` (only an ABSENT key uses the default). Those
  throws escape to the caller and fail the whole install;
  `process_optional_files` first rolls its queued operations back. The port
  returns `Result<_, SelectionsError>` and the private `name_field` reproduces
  `value()`'s exact tri-state (same as `fomod_inference_service::name_field`, see
  Task 12). PLUGIN names stay tolerant via `read_plugin_name`. Pinned by
  `non_string_step_name_aborts_and_rolls_back`,
  `non_object_step_and_non_string_group_name_abort`,
  `name_field_reproduces_nlohmann_value_tri_state`,
  `validate_propagates_the_name_type_error`.
- **The `FomodService.hpp` class doc comment is STALE - the CODE is ported, not
  the doc.** The header claims plugin entries are read via `get<std::string>()`
  and that a non-string entry throws. The actual code
  (`FomodService.cpp:22-33` `read_plugin_name`) is schema-tolerant: it accepts
  schema-v1 strings AND schema-v2 objects and returns `""` for anything else so
  the caller SKIPS it. Pinned by
  `read_plugin_name_accepts_both_schemas_and_skips_the_rest` and
  `schema_v1_and_v2_produce_identical_operations`.
- **Rollback contract.** On `Err`, `ops` is truncated back to its entry length
  and `next_doc_order` is restored (`FomodService.cpp:623-624`). The C++ does NOT
  roll back `plugin_flags_` mutations, and neither does the port - moot because
  the caller aborts the whole install on the re-throw. The
  `initial_ops_size`/`initial_doc_order` are captured at the top BEFORE the
  has-steps check, matching the C++.
- **Occurrence-based matching quirks**, each pinned: a JSON step with no `groups`
  array still CONSUMES a step-name occurrence before skipping
  (`step_without_groups_array_still_consumes_an_occurrence`); a missing IR step
  does NOT consume a second occurrence
  (`missing_ir_step_does_not_consume_a_second_occurrence`); a missing IR GROUP
  does NOT skip the plugin loop - the loop still runs and still advances the
  plugin-occurrence counters, every lookup simply misses
  (`group_without_plugins_array_still_consumes_an_occurrence`,
  `duplicate_plugin_names_bind_by_occurrence`). The three passes (explicit
  selections + per-step Required, catch-all Required for steps absent from the
  JSON, alwaysInstall/installIfUsable from unselected plugins) are ported
  statement-for-statement with visibility re-checked in each pass against the
  flags as they stand at that point.
- **Free functions vs private members.** `enqueue_entry` /
  `enqueue_plugin_files` / `make_plugin_key` are free functions (the C++ has them
  as private members, but none touch instance state); this lets
  `process_optional_files` split-borrow `installer` (read) and `plugin_flags`
  (write).
- **`enqueue_entry` rooted-destination hole, reproduced not fixed.** The guard
  `is_safe_destination` checks the NORMALIZED destination while the join uses the
  RAW one, so a rooted destination like `/etc/passwd` passes (normalization
  strips the leading slash) and then REPLACES the base during the join - the
  root-component rule is identical in `std::filesystem::operator/` and
  `Path::join`. Pinned by `enqueue_entry_reproduces_the_rooted_destination_hole`.
  `..` segments are stripped by `normalize_path` and accepted (as in the C++),
  pinned by `enqueue_entry_skips_traversal_destination`.
- **`execute_file_operations` failed-counter is always 0 in practice**, in both
  languages: `copy_file`/`copy_folder` swallow every I/O error internally (the
  C++ statics are documented and implemented non-throwing), so the `try`/`catch`
  this loop mirrors can never fire. The count is ported anyway because it is
  observable through the return value. The body is factored behind
  `execute_file_operations_with(ops, copy_fn)` so tests can observe execution
  order and the failure counting without touching disk
  (`execute_counts_failures_without_aborting`).

### Install-replay end-to-end oracle (`tests/fomod_service_install_fixtures.rs`)

Corpus-gated: for each committed case with an existing `source_archive_path`,
extract the real archive, parse its `ModuleConfig.xml`, replay the install driven
by the committed schema-v2 `expected.json` selections (the realistic
`installWithConfig` path AND free coverage of the v2 consumer over real
documents), then diff the tree the replay writes to disk against the golden
`target_tree.json`. Skipped as a no-op on CI (no corpus), same convention as
`archive_service_fixtures.rs`. Assertion strength is taken from the case's own
`expected.json`: `exact_match: true` cases are byte-checked (paths + reachable
sizes); `exact_match: false` cases are PATH-checked, since the reference
implementation does not itself claim size equality there. The non-exact path
check asserts real `missing.len() <= repro.missing` (the produced tree and the
simulated tree the `repro` counts come from target the same dest set, so the
replay must not DROP more dests than predicted - this catches under-production,
e.g. an optional pass that enqueues nothing, which a bare `missing == 0` guard
missed for `rar_exactlyone_heel_volume` at `repro.missing == 35`; a Task 14
review finding) plus `extra.is_empty()` when `repro.extra == 0`.

#### Stale golden vs committed archive - the simulator/installer/scan three-way split

The end-to-end oracle initially FAILED on `sevenz_2step_nec_feet`: 12 mesh files
"replayed at the wrong size" (e.g. `femalefeet_0.nif` got 825938, want 786505).
Root cause, fully traced, is NOT a replay defect:

- The committed archive contains exactly ONE source for each of those 12 dests
  (no conflict to resolve), and the replay faithfully copies that source's real
  bytes. `femalefeet_0.nif` is 825938 in `Base File\meshes\...`; the golden wants
  786505, a size that appears NOWHERE in the committed `.7z`. The installed mod's
  `.nif`s are simply a DIFFERENT build than the committed archive revision.
- The C++ inference nonetheless stamps the case `exact_match: true`. That is a
  false positive from the Task 12 entry-size under-population bug: the archive
  paths carry uppercase and backslashes (`Base File\meshes\...`), so their atoms
  keep `file_size == 0`, and a zero size compares as a size-compatibility
  wildcard in `find_contested_dests`/`compare_trees`, so the SIMULATED tree never
  registers a size mismatch. The simulator (atom-based) and the byte-level replay
  (`FileOperation`-based) therefore disagree only because the simulator is
  looking at size-0 atoms.
- Two further committed cases carry the same class of stale-golden file, for
  different real-world reasons: `zip_11step_cbbe_3ba` (3 shared `.tri` morph
  files; the archive ships 3048-byte stubs, the installed mod has 645644-byte
  versions - overwritten by a later mod, or a different build) and
  `zip_exactlyone_mu_joint_fix` (`skse/plugins/mujointfix.log`; the archive ships
  an empty 0-byte placeholder, the golden captured the 31378-byte RUNTIME log).
  All three are "the installed file at this dest is not what THIS archive
  produces."

Fix is to the ORACLE, not the replay (the replay is correct): the exact-case size
assertion now byte-checks only files whose golden size the committed archive can
actually produce (`golden_size ∈ set(archive entry sizes)`), and skips + counts
files whose golden size is unreachable. This is safe by construction - a genuine
wrong-winner produces SOME archive size, so if the correct golden size were
reachable the assertion still fires; only sizes NO archive source can produce are
excused. Confirmed across the whole committed corpus: every produced-vs-golden
size difference is an unreachable stale-golden file (16 files across 3 cases);
there is not a single reachable-dest disagreement, i.e. the replay's conflict
resolution reproduces the golden at every dest the archive can actually build.
Recorded here rather than "fixed" in the corpus because the golden bytes are the
authoritative capture of the installed mod, stale or not, and the Task 12
under-population bug that hides the discrepancy from the simulator is itself a
faithfully-reproduced C++ divergence.

Note the two salma conflict models this exposed: the forward SIMULATOR overwrites
on `new.priority >= existing.priority` and relies on phase + step/group/plugin
APPLICATION ORDER for the tiebreak (`FomodForwardSimulator.cpp:11-14`), while the
real INSTALLER sorts individual `FileOperation`s by `(priority, document_order)`
and copies folders atomically. They agree on every reachable dest in this corpus,
but they are not the same algorithm; a genuinely-contested folder-overlap dest
could in principle split them (a latent C++ characteristic, reproduced on both
sides).

### Scratch removed

A temporary `tests/zz_diag.rs` (a one-off print harness used while tracing the
`sevenz_2step_nec_feet` conflict above) was deleted before commit, per its own
header and the plan's git-hygiene rule.

### Test-suite delta

60 new unit tests (39 in `fomod_service.rs`, 21 in `file_operations.rs`), bringing
the `src/lib.rs` unittest binary to 476, plus the corpus-gated
`tests/fomod_service_install_fixtures.rs` (1 end-to-end test, a no-op on CI).
`cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check`
all clean.

## Task 15 - InstallationService + ModStructureDetector + ArchiveResolver + ABI wiring

Milestone 7 completes. `src/InstallationService.hpp`/`.cpp` (682 LOC) ->
`src/installation_service.rs`, `src/ModStructureDetector.hpp`/`.cpp` (148) ->
`src/mod_structure_detector.rs`, `src/FomodArchiveResolver.hpp`/`.cpp` (98) ->
`src/archive_resolver.rs`, plus the `install` / `installWithConfig` /
`resolveModArchive` wiring in `capi.rs`. With this task every one of the eight
C ABI exports is backed by real engine code; the only remaining hole is logging
(Task 17).

Module naming: the plan's architecture sketch spells these `archive_resolver.rs`
and `mod_structure_detector.rs`, while the crate's other modules mirror their C++
TU name verbatim (`FomodArchiveResolver.cpp` would give
`fomod_archive_resolver.rs`). The sketch is explicitly "e.g."; the plan's own
names were kept and the mapping is stated in each module's header.

### API mapping (C++ -> Rust)

| C++ | Rust |
| --- | --- |
| `InstallationService::install_mod` | `InstallationService::install_mod` |
| `InstallationService::find_fomod_folder` | `find_fomod_folder` (free fn) |
| `InstallationService::handle_non_fomod_install` | `handle_non_fomod_install` (free fn) |
| `InstallationService::handle_fomod_install` | `handle_fomod_install` (free fn) |
| `InstallationService::resolve_json_path` | `resolve_json_path` (free fn) |
| `ModStructureDetector::has_mod_structure` | `has_mod_structure` |
| `ModStructureDetector::find_main_mod_folders` | `find_main_mod_folders` |
| `mo2core::resolve_mod_archive` | `resolve_mod_archive` |
| the inline candidate vector (`FAR:26-44`) | `build_candidates` (extracted, see below) |

The private members become free functions so the in-module tests can drive each
stage directly, the same shape Task 12 used for `FomodInferenceService`.
`build_candidates` is a Rust-only split: it takes the downloads directory as a
parameter instead of reading `SALMA_DOWNLOADS_PATH`, so the search ORDER is
testable without mutating process-global environment state, which would race the
other tests in the same binary.

### Exceptions-return-Result contract

The C++ throws `std::runtime_error` for every fatal condition and
`CApi::install` returns `e.what()` verbatim. `install_mod` returns
`Result<String, InstallError>` where `InstallError` is a newtype over the exact
`what()` string, and `capi` hands it straight back. The seven salma-authored
messages are byte-exact:

| C++ site | Message |
| --- | --- |
| `IS:45` | `Archive file not found: {path}` |
| `IS:126` | `Install aborted: disk full while copying files. Free space and retry.` |
| `IS:257` | `Multiple mod folders detected but no moduleName in JSON to disambiguate.` |
| `IS:275` | `moduleName '{lowercased}' did not match any folder.` |
| `IS:276` | `moduleName '{lowercased}' matched multiple folders.` |
| `IS:319` | `Cannot parse XML ({description})` |
| `IS:413` | `Module-level dependencies not met - installation cannot proceed` |

Messages that originate in a LIBRARY (bit7z / libarchive extraction failures,
pugixml's `xml_parse_result::description()`, `std::filesystem_error`) cannot be
byte-matched: the Rust backends carry their own wording. The FAILURE is parity,
the TEXT is not. This is visible to a caller only as a different human-readable
error string, never as a different success/failure verdict.

### Accepted divergences (Task 15)

- **`parent_path` trailing separator - FIXED, not accepted.** Worth recording
  because `Path::parent()` is the obvious and WRONG mapping. C++ path iteration
  appends an empty final element for a trailing separator, so
  `parent_path("C:\MO2\mods\")` is `"C:\MO2\mods"`; Rust's `.parent()` discards
  the separator and yields `"C:\MO2"`, one level too high, silently shifting
  candidates 4-6 of the resolver chain. Callers do not normalize
  (`Mo2FomodController.cpp:439`, `scripts/mo2-salma.py:931`), so this was
  reachable. `archive_resolver::parent_path` models the C++ rule and is pinned
  by `parent_path_keeps_the_directory_when_it_ends_in_a_separator` and
  `trailing_separator_mods_dir_shifts_the_whole_chain`.
- **UNC root-name decomposition is NOT modeled.** MSVC's `_Parse_root_name`
  treats `\\server` as the root name; Rust's `Prefix::UNC` claims
  `\\server\share`, so a UNC `mods_dir` decomposes differently. No caller
  produces one and a UNC MO2 mods directory is exotic. Verbatim `\\?\` paths are
  likewise out of scope.
- **Malformed selections JSON yields a NULL document, not a partial one.**
  nlohmann's DOM parser writes the successfully parsed prefix into the target
  before throwing, so the C++ `config` can be PARTIALLY populated after the
  caught `parse_error` (`IS:190-206`, `:324-341`). `crate::json::parse` is
  all-or-nothing, so the port uses `Value::Null`. Observable only for a
  truncated document whose valid prefix already carried `moduleName` /
  `gamePath` / `gameVersion` / a usable `steps` array. Reproducing it would mean
  emulating nlohmann's incremental DOM construction.
- **`fs::exists` error handling.** The C++ uses throwing overloads: in
  `ModStructureDetector::has_mod_structure` a non-not-found error propagates and
  aborts the enclosing scan, and in the resolver a throwing candidate aborts the
  whole chain (caught at `CApi.cpp:158`), potentially masking a later candidate
  that would have hit. `Path::exists()` reports `false` for every error and both
  loops continue. Unreachable for well-formed inputs; modeling it would require
  mapping raw OS error codes.
- **`fs::relative` vs lexical prefix strip** in the installed-file scan: the C++
  RESOLVES symlinks, the port strips a prefix lexically. Same divergence already
  recorded for the Task 12 scan.
- **Non-ASCII paths.** MSVC constructs `fs::path` from a narrow `std::string`
  through the ACTIVE ANSI CODE PAGE, so a UTF-8 path arriving over the C ABI is
  mangled by the C++ DLL under a non-UTF-8 ACP and such archives fail to resolve
  or install. Rust keeps the UTF-8 bytes and converts to UTF-16 correctly, so it
  SUCCEEDS where C++ fails. A behavioral superset, not parity; deliberately not
  reproduced.
- **Invalid UTF-8 at the boundary** maps to `Unknown fatal error during
  installation` for install and `""` for resolve, where C++ forwards the raw
  bytes. Pre-existing Milestone-1 divergence, now also applied to `jsonPath`.
- **OOM.** `_strdup` can return NULL, which the Python consumer maps to `""`;
  `CString::into_raw` cannot, so Rust aborts instead. Unreachable in practice.

### C++ bugs and doc drift reproduced, not fixed

- **`installed_files` is populated with ORIGINAL-CASE relative paths**
  (`IS:395-397` applies only backslash -> slash), but
  `FomodDependencyEvaluator::evaluate_file_dependency` looks them up through
  `normalize_path`, which lowercases. Any installed file whose relative path is
  not already all-lowercase can therefore NEVER match a `fileDependency`, so
  re-install detection silently falls through to the archive-root probe. Latent
  C++ bug, reproduced. Pinned by
  `installed_file_scan_preserves_case_and_uses_forward_slashes`. Do NOT "fix"
  it: lowercasing would flip fileDependency results on re-installs and change
  which files get installed.
- **`find_fomod_folder` has NO shallowest-path preference**, unlike
  `FomodInferenceService.cpp:850-853` which explicitly tracks `best_depth` with
  the comment "prefer shallowest path". Install takes the first pre-order DFS
  hit. For a multi-FOMOD archive, INFER and INSTALL can therefore disagree about
  which `ModuleConfig.xml` is authoritative. Reproduced.
- **`src_base` and `context.archive_root` diverge for a nested FOMOD**
  (`IS:306` vs `:345`), so a `fileDependency` resolves against the extraction
  root while file copies source from the fomod's parent. Reproduced; the two
  roots are deliberately not unified.
- **The moduleName guard is a WEAKER hand-rolled duplicate of
  `is_safe_mod_name`** (`IS:209-235` vs `Utils.cpp:214-256`, whose comment even
  says "Mirrors the list in InstallationService.cpp"). It misses the empty,
  whitespace, absolute, dot, and trailing-dot rejections, so a moduleName of `.`
  or a space-padded name survives. The port calls neither `is_safe_mod_name` nor
  `is_safe_destination` here - using them would reject inputs the C++ accepts.
- **The `..` rejection is a raw substring search** over the lowercased name, so
  a benign `up..down` is rejected. Reproduced.
- **`resolve_json_path` does ZERO validation of a caller-supplied path** - no
  existence, extension, `is_inside`, or traversal check (`IS:473-475`). The
  `is_inside` guard applies only to the DERIVED path, where it is nearly always
  trivially true. `InstallationService.hpp:180-181` claims it "Validates with
  is_inside" without that qualification. All caller-side validation lives in
  `InstallationController.cpp:483-513`, which is mo2-server, NOT part of this
  port. Reproduced.
- **A bare-filename `archive_path` derives a RELATIVE JSON path** resolved
  against the process CWD, and skips the `is_inside` guard entirely because
  `parent_dir` is empty (`IS:479-490`). Reproduced.
- **`fs::path::stem` strips only the LAST extension**, so `mod.tar.gz` derives
  `mod.tar.json`. Rust's `file_stem()` matches.
- **The temp directory LEAKS when either `create_directories` fails**, because
  both run before the `try` (`IS:60`, `:65` vs `try` at `:68`). Reproduced: the
  port returns before its cleanup for the same two calls.
- **`has_mod_structure` probes with `fs::exists`, not `is_directory`**, so a
  plain FILE named `textures` marks a directory as a mod root
  (`ModStructureDetector.cpp:29`). The header describes it as a folder check.
  Reproduced and pinned by `a_file_named_like_a_mod_folder_also_counts`.
- **`find_main_mod_folders` returns the PARTIAL list** collected before a
  filesystem error, because `results` is declared outside the `try`
  (`MSD.cpp:38` vs `:41`). `ModStructureDetector.hpp:78-79` claims it "Returns
  empty on filesystem iteration errors". Code wins; reproduced.
- **`InstallationService.hpp:91` lists "Invalid JSON in selections file" as a
  FATAL error**, but the code CATCHES `json::parse_error` and only logs a
  warning (`IS:195-199`, `:334-338`). Code wins; a malformed selections file is
  not fatal. Pinned by `malformed_json_config_reads_as_null_not_an_error`.
- **`CApi.hpp:252-256` documents `installSucceeded` as "true if the last result
  was a valid mod path"**, but `CApi.cpp:51-53` / `:87-89` store `true`
  unconditionally on the non-throwing return WITHOUT inspecting the string. The
  two agree only because `install_mod` returns `mod_path` on every non-throwing
  path; they diverge for `Ok("")`, reachable by passing an empty `modPath`
  (`create_directories("")` does not throw, and Rust's `create_dir_all("")` is
  likewise `Ok`). The port implements the CODE predicate: the flag is true iff
  `install_mod` returned `Ok(_)`, whatever the value.
- **`CApi.hpp:177-182` claims an empty `jsonPath` means "optional steps are
  skipped because no selections exist"**, but an empty `json_path` triggers the
  archive-stem derivation in `resolve_json_path`, so an archive with a sibling
  `<stem>.json` is installed WITH those selections. Code wins.
- **`resolveModArchive` null-checks only two of its three pointers**: a NULL
  `modsDir` is coerced to an empty path while a NULL `installationFile` or
  `modFolder` short-circuits to `""` (`CApi.cpp:147-150` vs `:155`). Asymmetric
  but intentional per the header; reproduced exactly.
- **`FomodArchiveResolver.hpp:20-29` advertises a 6-step "first hit wins" chain,
  but step 1 is an EARLY RETURN**: an absolute-but-missing `archive_value`
  returns empty without evaluating steps 2-6. No observable consequence (an
  absolute right-hand side replaces the base in every join, so all five
  candidates would be identical and would all miss), but the structure differs
  from the doc. Pinned by `absolute_missing_value_does_not_fall_through`.
- **`mod_folder` is joined with NO emptiness guard while `mods_dir` has one**
  (`FAR:34` vs `:35`), so an empty `modFolder` turns candidate 3 into a probe
  relative to the process CWD - non-deterministic inside a DLL loaded by MO2.
  Reproduced; no guard added.
- **`FomodArchiveResolver.hpp:35` claims an "Absolute resolved path on hit"**,
  but nothing absolutizes: a relative `mod_folder` or `mods_dir` yields a
  relative result handed back through the ABI as such. Reproduced.
- **A first-level `mods_dir` like `C:\mods` makes candidate 6 byte-identical to
  candidate 5**, because C++ `parent_path("C:\")` is `"C:\"` and keeps
  `has_parent_path()` true. A redundant stat, harmless. Pinned by
  `candidate_order_respects_empty_and_shallow_mods_dir`.

### The plugin's legacy fallback is dead code (engine finding)

`scripts/mo2-salma.py:238` gates on `hasattr(lib, "resolveModArchive")`, and
`hasattr` on a ctypes `CDLL` resolves the symbol via `GetProcAddress`. Because
the Rust DLL genuinely EXPORTS the symbol, the plugin has always taken the DLL
branch and returned whatever it got - which, while the export was a stub, was
`""` for every input. The `SALMA_DOWNLOADS_PATH` fallback at `:246-254` is
therefore unreachable and could not rescue it. A stub that returns a
valid-looking empty answer is worse than a missing export, which would at least
have failed the `hasattr` check and reached the fallback. Fixed by implementing
the export; `scripts/` is a never-modify path and was not touched.

### Verification

- 41 new unit tests: 16 in `installation_service.rs`, 16 in `archive_resolver.rs`,
  9 in `mod_structure_detector.rs`, bringing the `src/lib.rs` unittest binary from
  476 to 517 (587 total across the workspace). The three obsolete Milestone-1
  stub assertions in `capi.rs` were replaced with real ones, including an
  end-to-end install through the ABI that builds a zip fixture, installs it,
  asserts the returned string is `mod_path`, asserts the success flag flips true,
  and asserts the flag is NOT cleared by a subsequent `inferFomodSelections`.
- **Differential install gate (the real oracle).** Both DLLs loaded in one
  process via ctypes, each curated case installed with each DLL from the same
  archive and the same committed schema-v2 `expected.json` selections, then the
  produced trees diffed by relative path and size:
  **16 SAME, 0 DIFF, 0 SKIP** across all 16 committed cases, covering zip / 7z /
  rar, the non-FOMOD content-root fallback (`sevenz_empty_no_moduleconfig`), and
  the 177-file 11-step FOMOD (`zip_11step_cbbe_3ba`). Each DLL frees only its own
  strings, since the two use different allocators. The script is scratch, not
  committed; Task 16 turns this into `tools/run_harness.py`.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test --release` (587 passed, 0 failed) all clean; `smoke_ctypes.py`
  passes against the release DLL.

## Task 16 - Round-trip validation with the Rust DLL

Milestone 7 gate. `tools/run_harness.py` drives the repo's UNMODIFIED
`test_all.py` / `test_one.py` against the Rust DLL and, for comparison, against
the C++ DLL.

### Beating the stale-DLL trap

`scripts/common.py::find_dll` searches, in order:

1. `$SALMA_DEPLOY_PATH/salma/mo2-salma.dll`
2. `build/bin/Release/mo2-salma.dll` (relative to the CWD)
3. `./mo2-salma.dll`

On a developer box candidate 1 exists and holds the DEPLOYED C++ DLL, so a naive
run silently validates the wrong binary. `run_harness.py` copies the DLL under
test to `target/harness/salma/mo2-salma.dll` and points `SALMA_DEPLOY_PATH`
at `target/harness` for the SUBPROCESS ONLY, so candidate 1 becomes the
staged file. No repo script is touched and no env change escapes the child.

Which binary actually ran is proven three ways, not assumed:

- **Path**: `test_all.py` logs `DLL: <path>`; the runner parses it and requires
  it to equal the staged path. (`test_one.py` logs no path, so that mode falls
  back to staging precedence plus the hash, and says so rather than skipping
  silently.)
- **Content**: the file at that path is re-hashed after the run and must match
  the SHA-256 of the DLL that was staged.
- **Behavior**: a pre-flight registers a log callback and runs one failing
  inference. The C++ `Logger` fires the callback (observed: 5 lines); the Rust
  port has no logger yet (Task 17) and fires 0. `getApiVersion` is useless as a
  discriminator here because both report `1.2.0`.

### TEMP pinning (required, not cosmetic)

The first full baseline run FILLED the system drive and died with the C++
engine's own `Install aborted: disk full while copying files`, leaving a 22 GB
orphaned `%TEMP%\salma-bit7z-batch-*`. `run_harness.py` therefore pins the
child's `TEMP`/`TMP` to `<mods drive>\salma_harness_tmp` (overridable with
`--tmp-base`) and wipes it before and after each run, the same rule
`gen_golden.py` already applies per mod.

### Results

Both DLLs were run over the same span, mod indices 1-300 of 309, with the
default byte-for-byte content compare enabled:

| | C++ `mo2-salma.dll` | Rust `mo2_salma_rs.dll` |
| --- | --- | --- |
| PASS | 52 | 52 |
| FAIL | 0 | 0 |
| SKIP | 248 | 248 |
| per-mod status disagreements | - | **0** |

The comparison is per-INDEX, not just per-total: every one of the 300 mods lands
on the same verdict on both sides, with the same skip reason (185 "no FOMOD /
scan returned empty", 63 "archive not found").

`test_one.py --full` on three representative mods, one per archive format, all
PASS with zero missing / extra / size / content mismatches:

| Archive | Mod | Files |
| --- | --- | --- |
| `.7z` | (CVEO) by LDD - Aretuza Eyes Remastered | 181 |
| `.zip` | 001_Gunslicer Animations All in One OAR | 1186 |
| `.rar` | 001_Schlongs_of_Skyrim_SE v1.1.4 | 144 |

### Timing (52 tested mods, same machine, sequential runs)

| Stage | C++ | Rust | Delta | Ratio |
| --- | --- | --- | --- | --- |
| scan (infer) | 428.7s | 670.9s | +242.3s | 1.57x |
| install (replay) | 288.7s | 537.2s | +248.6s | 1.86x |
| total | 899.5s | 1414.1s | +514.7s | 1.57x |

The port is CORRECT but ~1.6x slower. Two already-documented deferrals are the
likely contributors and were explicitly left for this task: the
`thread_local SimulatedTree` scratch that the C++ reuses on the solver hot path
was replaced by a fresh allocation per call (Task 9 note), and the archive
backends buffer whole entries rather than streaming (Task 11 note). Neither was
profiled here; that is optimization work, not parity work, and is left open.

### Corpus hazard at index 301, and a CONFIRMED memory divergence

Mod 301, `Zaki Tattoos General 8K Addon 1.2.1.7z`, is 43.3 MB compressed and
expands to over 63 GB of 8K textures. It ends every unbounded run on this
corpus, on BOTH engines, but it ends them DIFFERENTLY:

- **C++** streams contested entries through bit7z into
  `%TEMP%\salma-bit7z-batch-*` on DISK. Observed: 63 GB of scratch, which filled
  the system drive.
- **Rust** buffers entries in MEMORY (`read_to_end` in all three backends).
  Observed: 39.4 GB resident with 8 GB of 63.7 GB RAM left and still climbing;
  the run was killed to protect the machine.

This UPGRADES the Task 11 note, which said the in-memory model was "acceptable
for FOMOD content, revisited only if Task 16 surfaces a giant loose entry". Task
16 has now surfaced exactly that, on a real corpus mod, so the divergence is
confirmed reachable rather than theoretical:

- The C++ degrades to a disk-space failure, which its own `disk_full_encountered`
  guard turns into a clean `Install aborted` error.
- The Rust degrades to memory exhaustion, which has NO equivalent guard and
  would take the host process (MO2) down with it.

Recorded, not fixed: the fix is to stream extraction block-by-block like the C++
`copy_data`, which is a rework of `archive_service.rs` and belongs with the
performance pass, not the parity gate. Until then a mod of this shape is a
hard-failure risk on the Rust DLL that it is not on the C++ DLL. This is the
single most consequential open divergence in the port and should gate cutover.

Because the run stops there on both sides, mods 301-309 (9 mods, all sorting
after "Z") are NOT covered by these numbers. That is a coverage gap in the
harness result, stated rather than papered over.

### Gate results

- `test_all.py`, mods 1-300, Rust DLL: 52 passed, 0 failed, 248 skipped, 0
  per-mod disagreements vs the C++ baseline.
- `test_one.py --full`: 3 of 3 PASS.
- No repo script was modified; `git diff main -- src tests CMakeLists.txt
  scripts test_all.py test_one.py` stays empty.

## Task 17 - Logger parity, packaging, CUTOVER.md

Milestone 8. `src/Logger.hpp`/`.cpp` (593 LOC) -> `src/logger.rs`, plus
`tools/package.py`, `tools/smoke_plugin.py`, and `CUTOVER.md`.

### Logger mechanism (at parity)

| C++ | Rust |
| --- | --- |
| Meyer singleton `Logger::instance()` | `OnceLock` behind `Logger::instance()` |
| `std::atomic<LogCallback> callback_` | `AtomicUsize` holding the fn-pointer address |
| `std::mutex mutex_` + `ofstream` + `bytes_written_` | `Mutex<FileState>` grouping all three |
| `thread_local g_in_callback` | `thread_local IN_CALLBACK: Cell<bool>` |
| `write_log_unlocked` | `FileState::write_line` |
| `rotate_if_needed` | `FileState::rotate_if_needed` |

Behaviors reproduced exactly:

- **Anchoring.** `logs/` resolves next to the MODULE that owns the code, via
  `GetModuleHandleExW(FROM_ADDRESS)` on an address inside the DLL, not the host
  executable. MO2 runs as ModOrganizer.exe with the DLL under its plugins tree,
  so anchoring on the exe would put the log in the wrong place. Verified: a
  ctypes-driven install from a Python process wrote
  `target/release/logs/salma.log`, beside the DLL.
- **Line format** `YYYY-MM-DD HH:MM:SS.mmm LEVEL message`, LOCAL time, zero
  padded in every field including 3-digit milliseconds. Diffed against real C++
  `build/bin/Release/logs/salma.log` lines; identical.
- **Local time** comes from `GetLocalTime`, the same OS source `localtime_s`
  uses, so timezone and DST rules match. `std` has no local-time conversion, so
  there is no portable alternative; the non-Windows fallback is UTC and exists
  only so the crate still builds off-Windows.
- **Routing.** Under the lock, snapshot the callback and write the file line
  ONLY when no callback is registered. Outside the lock, echo the RAW message
  (no timestamp, no level) to stdout for info/warning and stderr for error,
  whether or not a callback exists. Then invoke the callback. A registered
  callback therefore REPLACES file logging rather than duplicating it, and
  `setLogCallback(null)` restores it.
- **Re-entrancy.** A callback that logs is dropped with
  `[Logger] Re-entrant callback dropped: ...` instead of recursing.
  `catch_unwind` around the call mirrors the C++ `catch (...)`.
- **Rotation** at 10 MiB keeping `salma.log.1`-`.3`: `.3` deleted, `.2`->`.3`,
  `.1`->`.2`, current->`.1`. Including the subtle part: when the final rename
  FAILS (an antivirus or log viewer pinning the file), the counter is
  deliberately NOT reset, so the file reopens in append mode and grows past the
  cap rather than losing entries.
- **Append-mode seeding**: the rotation counter starts from the existing file
  size, so a restarted process does not think the log is empty.

### Log message coverage (complete, with a catalogued exception list)

Both the MECHANISM and the message COVERAGE are at parity. Every C++ engine log
call site is reproduced except the ones listed under "Sites with no Rust
counterpart" below, each of which is unreachable here or describes a library
this port does not link.

An earlier revision of this section claimed the gap was "~95 call sites in three
files". That count was wrong in both directions and is corrected here: it omitted
`ArchiveService` (30 sites, and the module had neither logging nor deferral
markers, so nothing flagged it), `CApi` (8), `FomodDependencyEvaluator` (3) and
`InferenceDiagnostics` (1), and it counted `FomodCSPSolver` as 17 rather than the
35 it and `FomodCSPSolverPhases` carry together. The real starting gap was ~155
sites across seven files.

Call-site counts. A Rust count can differ from the C++ in EITHER direction: a
multi-line C++ `std::format` can map to more than one Rust branch, and a shared
Rust helper can cover several identical C++ sites.

| Module | C++ | Rust | State |
| --- | --- | --- | --- |
| `FileOperations` | 17 | 21 | complete |
| `ModStructureDetector` | 2 | 3 | complete |
| `InstallationService` | 43 | 41 | complete |
| `FomodCSPOptions` | 3 | 3 | complete |
| `FomodPropagator` | 1 | 1 | complete |
| `FomodIRParser` | 3 | 3 | complete |
| `FomodInferenceAtoms` | 3 | 4 | complete |
| `FomodDependencyEvaluator` | 3 | 3 | complete |
| `InferenceDiagnostics` | 1 | 1 | complete |
| `FomodService` | 43 | 42 | complete |
| `FomodCSPSolver` + `Phases` | 35 | 30 | complete |
| `FomodInferenceService` | 53 | 49 | complete |
| `CApi` | 8 | 5 | complete |
| `ArchiveService` | 30 | 11 | see below |

Where a Rust count is lower, a shared helper covers several C++ sites:
`log_phase_metrics` serves the five `[solver] After <phase>:` lines,
`save_checkpoint` serves both C++ checkpoint-limit sites (one of which,
`SelectionCheckpoint::save`, is dead code with no callers in either language),
`install_impl`'s tagged error branch serves both `install` and
`installWithConfig`, and `safe_output_path` / `note_extracted` serve the
traversal-skip and per-100-progress lines for all three archive backends.

Verification method: for each module, extract every `"[tag] ..."` literal from
both sides, collapse `{...}` placeholders, sort and diff. The only residual
differences are C++ string literals split across source lines (same emitted
text) and the exceptions below.

### The CSP progress bar

`FomodCSPSolver`'s narrative includes a tqdm-style progress bar. `SolverProgress`
was already ported in full (Task 9) but nothing wrote to `estimated_total` /
`pass_start_*` / `last_progress_*`, and the four formatters were absent. All four
(`format_count`, `format_duration`, `format_option_cap`, `build_tqdm_bar`) are
now ported and the fields are maintained.

The per-node progress check in `evaluate_candidate` sits behind the same
`estimated_total > 1` guard the C++ uses, so a pass that never sets an estimate
does not read the clock; where the estimate IS set, this port now does exactly
the work the C++ already did. It is therefore not expected to widen the ~1.57x
gap recorded in "Task 16", though that has not been re-measured.

Two bar behaviors were reproduced from the C++ index arithmetic rather than
re-derived: the `>` head OVERWRITES the cell after the filled run (so 0% renders
`>...................`, and a full bar has no head), and the closing per-pass bar
uses the nodes ACTUALLY explored as its denominator rather than the estimate, so
every pass ends at exactly 100%.

### `[archive]` lines: backend names substituted

`ArchiveService` is the one module where a faithful transcription would be
false. The C++ names its libraries in the log text ("via bit7z", "falling back
to libarchive", "Using libarchive for extraction"); this port links neither,
using `zip` / `sevenz_rust2` / `unrar`. Emitting the C++ strings verbatim would
make a deployed DLL report libraries it does not contain, which actively misleads
anyone debugging from a log.

The resolution, chosen deliberately: keep the C++ line shape, tag and position,
and name the crate that actually ran. `[archive] list_entries: 42 entries, 42
sizes via sevenz_rust2 (13ms)`. The backend-agnostic lines (`Extracting archive`,
`Skipping path-traversal entry`, `Extracted N files...`, `extract_filtered:
extracted N entries`, and the two `create_zip` warnings) are verbatim.

This forced one small structural change: the C++ `extract` and `extract_filtered`
are separate entry points with separate narratives, but this port implements the
former via the latter. A shared silent `extract_counted` now carries the routing
and returns the entry count, and each public entry point owns its own lines, so
`extract` does not emit `extract_filtered`'s closing line.

### Sites with no Rust counterpart

Each is unreachable in this port or describes machinery it does not have.

| C++ site | Why absent |
| --- | --- |
| `[archive] 7z library: {} ({})`, `[archive] 7z.dll not found in SEVENZIP_PATH...` | `7z.dll` discovery does not exist; the backends are statically linked crates |
| `[archive] Write header warning`, `Copy data warning`, `copy_data failed for entry: code {}` | libarchive write-disk handle warnings; no counterpart (this port has no handle that can warn without failing) |
| the four `... falling back to libarchive` lines | there is no bit7z-vs-libarchive fallback to report |
| `[infer] Error after {}ms: {}` | the C++ outer `catch`; there are no exceptions here, and every failure path already logs its own cause before returning `""` |
| `[infer] Failed to allocate {} bytes for hashing: {}` | Rust aborts on allocation failure rather than unwinding, so the `bad_alloc` handler has no equivalent |
| `[solver] Checkpoint limit reached` (the `SelectionCheckpoint::save` copy) | dead code in the C++ - the struct has no callers; the live lambda site IS ported |
| `[infer] Fatal error: {}` / `[resolveModArchive] Fatal error: {}` (the `catch (const std::exception&)` arms) | unreachable in BOTH languages: `infer_selections` and `resolve_mod_archive` swallow internally and never propagate. The sibling `catch (...)` arms ARE ported, onto the panic guards |

### Sites whose text differs

Every one is a place where the C++ interpolates a caught exception's `what()`,
which has no counterpart. The trigger and the recovery match; the trailing reason
does not.

| Site | C++ interpolates | This port interpolates |
| --- | --- | --- |
| `[fomod] Exception during optional file processing...` | `nlohmann` `type_error::what()` | `SelectionsError`'s `Display` |
| `[fomod] Failed to execute file operation: {} -> {}: {}` | `ex.what()` | nothing - the trailing `: reason` is dropped (the back end reports failure as a `bool`). Unreachable in both languages |
| `[fomod] Malformed version component "{}": {}` | MSVC `stoi` (`"invalid stoi argument"`) | `ParseIntError` (`"cannot parse integer from empty string"`) |
| `[infer] XML parse failed: {}` | pugixml `xml_parse_result::description()` | roxmltree's error `Display` |
| `[infer] Failed to parse fomod-plus JSON: {}` | `nlohmann::parse_error::what()` | this port's JSON parser error; ill-formed UTF-8 (nlohmann error 316) reads `invalid UTF-8 in value` |
| `[install]` / `[installWithConfig] Fatal error: {}` | `ex.what()` | `InstallError`'s `Display` (same text for every salma-authored message) |
| `[infer] Error iterating {}: {}` | the root mod path (one recursive iterator spans the tree) | the directory that actually failed (the walk is per-directory) |

Two engine messages carry tags that do not match the rest of the subsystem, and
are kept exactly as the C++ has them: the condition-depth warning in
`FomodDependencyEvaluator` is tagged `[fomod-ir]`, not `[fomod]`, and its
unknown-file-dependency-state warning has NO tag at all (the C++ builds that one
by string concatenation rather than `std::format`).

### A behavioral divergence surfaced while restoring these sites

`ArchiveService::create_zip`: where the C++ cannot read a file's size it warns,
SKIPS that entry and continues; this port propagates the error and abandons the
whole archive. Same for a write error mid-entry. The warnings are now emitted at
both points, but the control flow was NOT changed, because altering it is outside
a logging task. Low impact: `create_zip` has no callers anywhere in the C++
engine and only a Rust unit test exercises it here.

### `installSucceeded` semantics

Unchanged from Task 15, restated here because the plan lists it under this task:
the flag is true if and only if `install_mod` RETURNED, without inspecting the
returned string. That is the C++ CODE's predicate (`CApi.cpp:51-53`, `:87-89`),
not the looser one `CApi.hpp:252-256` describes. Task 17 did not alter it.

Also resolved here: the `black_box` in `capi::setLogCallback` is gone. It
existed only because nothing read the stored callback, letting the release
optimizer delete the store and ICF-fold the emptied function. There is a real
reader now, so the hack is unnecessary.

### Packaging

`tools/package.py` builds release and stages the DLL to
`target/package/mo2-salma.dll`, printing size and SHA-256. The rename from
`mo2_salma_rs.dll` happens ONLY here: during the parity phase the two names stay
distinct so a stray copy can never be mistaken for the C++ build.
`--keep-rust-name` skips the rename, `--no-build` stages an existing build.

The artifact directory holds the DLL and nothing else. The engine has no runtime
data files and `logs/` is created next to the DLL on first use.

### Plugin-loader smoke test

`tools/smoke_plugin.py` copies `scripts/mo2-salma.py` VERBATIM into a
staging tree (asserting a byte-identical copy), places the packaged DLL at the
plugin's own first search candidate (`<plugin dir>/salma/mo2-salma.dll`), stubs
`mobase` and `PyQt6` (MO2-only imports, stubbed at module scope so every line of
salma's own code still runs), then drives the plugin's real `find_dll`,
`load_dll`, `_configure_dll`, `_check_api_version` and `_call_owned_string`.

12 checks, all passing: the plugin finds the staged DLL, configures and
version-checks it, round-trips an owned string through its own free helper,
sees `resolveModArchive` via `hasattr`, reads `installSucceeded` as false, and
`logs/salma.log` appears beside the DLL with a correctly-shaped first line.

The log assertion had to be rewritten once the inference sites landed. It
originally required the FIRST line to contain `" INFO [install] "`, which held
only while an inference run emitted nothing: the script exercises
`inferFomodSelections` BEFORE `install`, so the first line is now the `[infer]`
banner. It now matches the line SHAPE
(`^date time.mmm (INFO|WARNING|ERROR) [tag] `) and separately asserts that an
`[install]` line appears somewhere in the file - 12 checks instead of 11. The
DLL behavior was correct throughout; the assertion had encoded the gap it was
written against.

**The live MO2 installation was NOT touched.** The plan permits deploying into
`SALMA_DEPLOY_PATH` when it exists, and it does exist here
(`D:\Nolvus\Instance\MO2\plugins`), holding the user's working C++ DLL.
Overwriting a live modding setup's engine is an outward-facing change that the
parity phase does not require, so the documented headless alternative was used
instead. CUTOVER.md carries the manual steps for when that decision is made
deliberately.

### `deploy.bat` needs no changes

`deploy.bat:34-35` already prefers a `mo2-salma.dll` at the repo root over
`build\bin\Release\`, so the Rust DLL can be deployed through the unmodified
script by staging it there. Two hazards, both documented in CUTOVER.md: the
override is sticky (every later `deploy.bat` keeps shipping the Rust build), and
the repo root was NOT git-ignored for that name, so a 2.5 MB binary could have
been committed by accident. An ANCHORED `/mo2-salma.dll` rule was added to
`.gitignore` to close the second one; anchored specifically so it cannot hide a
DLL elsewhere in the tree, which is the trap recorded for the old unanchored
`logs/` pattern.

### Tests

+7 unit tests in `logger.rs` covering the line format (including zero padding
and an empty message), the level strings, the rotation constants, the local-time
stamp, and the log-directory anchoring. The `capi` callback test now asserts the
export reaches the logger. Callback registration is asserted in exactly ONE test
across the binary, because the callback is process-global and cargo runs tests
in parallel. Suite: 587 -> 594, 0 failures; `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` clean.

## Layout, scripts and pipelines

The crate reached the repo root in two steps. It began as a one-member cargo
workspace under `rust/`, which put sources three levels below the root:

| Originally | Then | Now |
| --- | --- | --- |
| `rust/Cargo.toml` (workspace) + `rust/mo2-salma-rs/Cargo.toml` (package) | one `rust/Cargo.toml` | `Cargo.toml` |
| `rust/mo2-salma-rs/src/` | `rust/src/` | `src/`, beside the C++ |
| `rust/mo2-salma-rs/tests/` | `rust/tests/` | `tests/`, beside the C++ |
| `rust/mo2-salma-rs/build.rs` | `rust/build.rs` | `build.rs` |
| `rust/tools/` | unchanged | `tools/` |
| `rust/PARITY-NOTES.md`, `rust/CUTOVER.md` | unchanged | root |

`rust/` no longer exists. The two languages now share `src/` and `tests/`
without colliding, because every tool is already extension-scoped: cargo reads
only `*.rs`, CMake takes an explicit file list, and clang-format and doxide glob
`*.cpp`/`*.hpp`. Since the port is file-for-file, `src/FomodService.cpp` and
`src/fomod_service.rs` sort adjacently, which makes the mapping visible.

Merging the two `tests/` directories is safe for the same reason plus one more:
cargo only treats `.rs` files DIRECTLY under `tests/` as integration targets, so
`tests/golden/` (data) and `tests/common/` (a shared module) are skipped by the
target scanner, and the C++ `tests/*.cpp` are invisible to it.

The Python tools needed two rounds of fixing. In the first move nothing changed,
because all of them resolve from the REPO ROOT rather than the crate directory.
In the second they all broke at once: each walked up three parents to find the
root (`tools/x.py` -> `rust/tools` -> `rust` -> root) and now needs two, and
every `rust/` path segment inside them had to go.

Four crate-relative fixture paths broke in the FIRST move, and only one was
caught by a compile error - the other three were runtime `env!` joins that
surfaced as a failing assertion. `tests/common/mod.rs`,
`tests/fomod_atoms_fixtures.rs`, `tests/fomod_ir_fixtures.rs` and `src/utils.rs`
all joined `CARGO_MANIFEST_DIR` with `../tests/golden/cases`; dropping the `../`
fixed them. The second move needed no further change there, because the manifest
and the corpus moved together.

One real bug slipped through the second move and was caught in the Task 18
review rather than by any gate: `deploy.bat` still pointed at
`rust\target\package\mo2-salma.dll`. That path no longer exists, so the script
would have fallen through to its C++ fallback and silently deployed the WRONG
ENGINE. No test covers `deploy.bat` (it writes into a live MO2 install), which
is exactly why it survived a green gate run.

### Scripts

`build.bat`, `test.bat` and `deploy.bat` were repurposed in place rather than
gaining `rust-` siblings.

- `build.bat` - `cargo fmt` -> `cargo clippy --all-targets --release -D warnings`
  -> `cargo build --release` -> `package.py --no-build`.
- `test.bat` - `cargo test --release` -> `smoke_ctypes.py` -> `smoke_plugin.py`.
  All three are corpus-free; the corpus-backed checks are named in its header.
- `deploy.bat` - prefers `target/package/mo2-salma.dll`, falls back to the
  C++ `build/bin/Release/mo2-salma.dll`, and a repo-root `mo2-salma.dll` still
  overrides both. It now prints a cutover warning, because MO2 loads whatever
  sits at the deploy path and `getApiVersion` reports `1.2.0` for both engines.
- `purge.bat` - untouched. It removes deployed files from MO2 and never cared
  which engine produced them.

Both scripts default `CARGO_BUILD_JOBS=4` when it is unset, and say why in
their header: cargo otherwise runs one job per core, and on a 32-core host the
parallel rustc + link peak took the toolchain down with a rustc
`STATUS_HEAP_CORRUPTION` and a cc-rs failure building the unrar sources. Both
recurred only at full parallelism. Set the variable to override.

**The C++ engine is no longer built by any script.** It still compiles, is
untouched, and remains the parity oracle that `gen_golden.py` and
`run_harness.py` compare against, but building it is now the two documented
cmake commands (recorded in `build.bat`'s header and in CLAUDE.md).

### Pipelines

`build.yml` became the Rust pipeline (replacing the C++ build) and gains the corpus-free checks after its existing fmt/clippy/build/test
sequence: `package.py`, `smoke_ctypes.py`, `smoke_plugin.py`, and an artifact
upload of `mo2-salma.dll` so a cutover candidate is downloadable from a green
run. Its path filter now also covers `build.bat`, `test.bat` and
`scripts/mo2-salma.py`, since the smoke test drives the plugin verbatim and
would not otherwise re-run when the plugin changes.

The corpus-backed gates deliberately stay out of CI: `compare_infer.py` and
`run_harness.py` need the mod archives and the C++ oracle DLL, neither of which
exists on a clean runner.

`lint.yaml` became `eslint.yaml`, named for the tool it actually runs, and
gained a `web/**` path filter. It was already scoped to `web/` by
`working-directory`, but fired on every push to `main` including C++ and
Rust-only changes. README gains `rust` and `eslint` badges beside `build` and
`tests`.

`sonar.yml` now scans BOTH engines. Three things were needed, and only the first
is obvious:

1. `sonar.sources` and `sonar.tests` cover Rust. After the root move both
   languages live in `src/` and `tests/`, so the original single entries suffice.
2. The Rust analyzer runs Clippy ITSELF (`sonar.rust.clippy.enabled` defaults
   to true) by shelling out to cargo, so the runner needs the toolchain plus the
   clippy component. `sonar.yml` installs them. Without this the scan does not
   fail, it silently reports zero Rust files, which reads as a clean pass.
3. It looks for `Cargo.toml` in the PROJECT ROOT by default, and ours is at
   the crate manifest, so `sonar.rust.cargo.manifestPaths` names it explicitly.
   That is the analyzer's own default now that `Cargo.toml` is at the root, but
   it is stated anyway so a future move cannot silently drop Rust from the scan.

`tests/golden/**` is excluded: it is committed fixture DATA (XML and JSON),
not code, and would otherwise be scanned as source.

None of the three is verifiable from a local checkout - they need a real
SonarCloud run to confirm - so treat the first green scan as the actual proof.

`build.yml` and `test.yml` are unchanged. They still gate the C++ and the web
frontend correctly, and needed nothing for the Rust work. They only trigger on
`main`, so they do not run on this branch's pushes. Whether the C++ gates should
survive the merge is a cutover decision, not a layout one, and is left open
deliberately.

Their triggers were deliberately NOT path-filtered the way `eslint.yaml` was. A
Rust-only PR does run the full vcpkg C++ build for nothing, but if either is
configured as a required status check in branch protection, a path filter makes
the check never report and the PR waits on it forever. Adding the filters is
safe only together with changing branch protection, which cannot be done from
the repo.

### `build.yml` absorbed the Rust pipeline

`rust.yml` was deleted and `build.yml` replaced with its content, so there is one
`build` badge rather than a `build` + `rust` pair. Two jobs: `engine (rust)` (the
former rust.yml sequence) and `web dashboard` (`npm run build`, kept because it
is tsc type-check plus vite, and the SPA is served by the C++ `mo2-server` which
the Rust port does not replace).

What the old C++ `build.yml` gated and nothing gates now: the clang-format
diff check over `src` + `tests`, and the C++ Release build itself. The build is
still covered indirectly, because `test.yml` configures and builds the C++ on
its own before running `ctest`; the format check is genuinely gone. Neither
matters while nobody edits the C++, and both would come back with it if the
oracle is ever removed.

## Removing the C++ engine

With Task 18 signed off, the C++ engine was deleted. What remains of the C++ is
the Crow server behind the web dashboard, which the Rust port never covered.

### The server had to be rewired first

The engine could not simply be deleted. `mo2-server` reached it as in-process
C++ classes, and while only THREE engine headers were included directly
(`InstallationService.hpp`, `FomodInferenceService.hpp`,
`FomodArchiveResolver.hpp`), their transitive closure was the whole tree: 65 of
68 files. `InstallationService.hpp` pulls `FomodService.hpp` -> `FomodIR.hpp` ->
the dependency evaluator, and `FomodInferenceService.hpp` pulls the entire CSP
chain. Counting direct includes says "3 files"; counting what the compiler
needs says "everything".

`src/SalmaEngine.{hpp,cpp}` replaces those three with the flat C ABI the MO2
Python plugin already used, loading `mo2-salma.dll` through
`LoadLibraryW`/`GetProcAddress`. With the includes cut, the closure collapsed
and **39 files** were deleted.

Two behaviors the bridge has to preserve, neither obvious:

- **Install failure must THROW.** The former `InstallationService::install_mod`
  threw, and both controller call sites are wrapped in `catch (std::exception)`.
  The C ABI instead returns the error text and sets `installSucceeded()` false,
  so the bridge converts that back into a `std::runtime_error` carrying the
  engine's own message. Returning an error string would have made every failed
  install look like a success to the dashboard.
- **`installSucceeded()` is a process-global flag**, and the server runs
  installs on overlapping background jobs. The bridge serializes the call and
  its flag read under a mutex. The old in-process service needed no such guard
  because each call had its own instance.

### What was kept

`Utils.cpp`, `Logger.cpp`, `SecurityContext.cpp` (as the new `salma-support`
STATIC library) plus the 13 server translation units. `salma-support` is
deliberately NOT a shared library named `mo2-salma`: that name belongs to the
Rust artifact now, and two different DLLs claiming it is how the wrong engine
gets shipped.

`Export.hpp`'s `MO2_API` now expands to nothing, since static linkage needs no
decoration. The dllexport/dllimport spellings are kept behind `MO2_CORE_SHARED`
in case a DLL target ever returns. Leaving `MO2_API` as `dllimport` against a
static library is what the first build attempt failed on.

`Utils` lost `get_ordered_nodes` and `xml_bool_attribute_true`: pugixml-typed
helpers that only the deleted XML parser used. Removing them dropped pugixml,
libarchive and bit7z from `vcpkg.json` entirely, which is why the C++ now
configures in ~20s instead of a multi-minute vcpkg build.

### The oracle is gone, and two tools would have lied about it

`build/bin/Release/mo2-salma.dll` used to be the C++ oracle. It is now where
CMake copies the RUST DLL so `mo2-server` can load it at runtime. Anything still
treating that path as the oracle would compare the port against itself:

- `run_harness.py --baseline` now exits with an explanation instead of staging
  the Rust DLL and labelling it "baseline". It would have reported a flawless
  self-comparison.
- `gen_golden.py`'s default `--dll` is annotated for the same reason.
  Regenerating the corpus now would bless the port's own output as the
  reference.

The committed golden cases under `tests/golden/cases/` are unaffected: they are
captured C++ output from before the deletion, so `compare_infer.py --curated`
still validates against a genuine oracle. The gitignored 197-fixture full corpus
cannot be regenerated without checking out a commit that still has the C++.

### Coverage for the new bridge

`tests/salma_engine_test.cpp` is the only coverage of the boundary every
dashboard install/infer/resolve request crosses: DLL load, ABI version match
against `src/capi.rs`, the empty-result contracts for infer and resolve, and the
throw-on-failure contract for install. The tests fail loudly rather than
skipping when the DLL is absent, because a server that cannot load its engine is
not a working server.

CMake copies `target/package/mo2-salma.dll` next to `mo2-server.exe` as a
post-build step and warns when it is missing. Without that the dashboard starts
fine and then fails every engine request at runtime.

### Numbers

C++ went from 68 files to 31 (`src/`) and from 4 test files to 3. `salma_tests`
went from 106 assertions to 76; the drop is the deleted engine suites, offset by
the 5 new bridge tests. `cargo test` is unchanged at 594.
