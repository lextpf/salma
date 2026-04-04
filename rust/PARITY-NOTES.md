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
