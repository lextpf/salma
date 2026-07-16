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
