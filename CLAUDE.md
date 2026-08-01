# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Rules

Ask when unclear. If intent, architecture, or requirements are ambiguous, ask before coding.

Flag uncertainty. If an approach, dependency, or technical detail is uncertain, say so before proceeding.

Challenge bad direction. If my request conflicts with settled practice or likely long-term maintainability, point it out and suggest a better path.

End with omissions. After each task, state what you changed and what you intentionally did not do.

## What this is

salma is a wizardless FOMOD installer, processor, and selection-inference engine for Skyrim modding (Mod Organizer 2 integration). C++23 backend + React frontend, Windows-only (MSVC 2022).

The engine is Rust (`src/*.rs`, built to `mo2-salma.dll`). The C++ that remains in `src/*.cpp` is the Crow HTTP server behind the web dashboard, which the Rust port does not cover; it reaches the engine through the same C ABI the MO2 plugin uses. See "Architecture" below.

## Build, test, run

**The root scripts drive the Rust build**, not the C++ one.

```powershell
.\build.bat        # fmt -> clippy (-D warnings) -> cargo build --release -> package the DLL
.\test.bat         # cargo test --release -> smoke_ctypes.py -> smoke_plugin.py
.\deploy.bat       # copy the packaged DLL + mo2-salma.py into the MO2 plugins dir (a CUTOVER, read CUTOVER.md)
.\purge.bat        # remove the deployed plugin from MO2 (engine-agnostic)
```

Both scripts default `CARGO_BUILD_JOBS=4` when unset. Cargo otherwise runs one job per core, and on a high-core host the parallel rustc + link peak has taken the toolchain down (rustc `STATUS_HEAP_CORRUPTION`, cc-rs failures building the unrar sources). Set the variable to override.

For a fast Rust iteration loop:

```powershell
cargo build --release   # the DLL: target\release\mo2_salma_rs.dll
cargo test  --release   # 533 tests
cargo clippy --all-targets --release -- -D warnings
```

### Building the C++ server

No script builds it any more. VCPKG_ROOT must be set (the CMake preset reads it for the toolchain). Dependencies are vcpkg manifest-mode (`vcpkg.json`) on the `x64-windows-static-md` triplet, supplied by the overlay in `cmake/`. Only Crow, nlohmann-json and gtest remain, so configure takes ~20s; libarchive, pugixml and bit7z went with the engine.

```powershell
cmake --preset default                                    # Release configure (preset "debug" for Debug)
cmake --build build --config Release                      # everything
cmake --build build --config Release --target salma-support # static helpers (Utils, Logger, SecurityContext)
cmake --build build --config Release --target mo2-server  # the EXE (Crow HTTP server)
cmake --build build --config Release --target salma_tests # GoogleTest binary
.\build\bin\Release\mo2-server.exe                        # run the server on 127.0.0.1:5000
```

Build outputs land in `build/bin/Release/`: `mo2-server.exe`, `salma_tests.exe`, plus the copied `mo2-salma.dll`. `mo2-server` loads `mo2-salma.dll` at runtime; CMake copies it beside the exe from `target/package/` as a post-build step, so run `build.bat` before building the C++ or the dashboard will start and then fail every engine request.

### Tests

Rust tests are the primary suite (533 at last count) and are all self-contained: unit tests inline in `src/*.rs` under `#[cfg(test)]`, plus `tests/inference_diagnostics_test.rs`.

```powershell
cargo test --release                       # everything
cargo test --release -- utils::            # a module's unit tests
```

**Do not commit fixtures derived from real mods.** The repo used to carry a golden corpus built from a real mod list, which described the setup through case names, third-party FOMOD documents and local archive paths. It was removed, along with the fixture-driven tests that consumed it: those tests existed to prove byte parity against the C++ engine, and that engine no longer exists to compare against. Keep new tests self-contained.

Real-install validation happens against a live MO2 instance and never touches the repo. It needs the `SALMA_*` env vars (`scripts\setup-env.bat`), and a locally generated corpus that `.gitignore` keeps out of the repo:

```powershell
python tools\gen_golden.py                                    # build the LOCAL corpus under tests/golden/full/
python tools\compare_infer.py target\release\mo2_salma_rs.dll # infer over that local corpus
python tools\run_harness.py                                   # test_all.py round-trip against your MO2
python test_one.py <archive> <installed-mod-folder> --full    # one mod, byte-for-byte
```

C++ unit tests are GoogleTest (linked via `GTest::gtest_main`), discovered with `gtest_discover_tests`.

```powershell
.\build\bin\Release\salma_tests.exe                                      # all tests
.\build\bin\Release\salma_tests.exe --gtest_filter=FomodInference.SelectAll_Deterministic   # single case
.\build\bin\Release\salma_tests.exe --gtest_filter=FomodInference.*       # whole suite
.\build\bin\Release\salma_tests.exe --gtest_list_tests                    # enumerate
ctest --preset ci                                                        # CTest wrapper (what test.yml runs)
```

Adding a test: create `tests/<subject>_test.cpp` (snake_case, named after the behavior) AND add it to the explicit source list in `add_executable(salma_tests ...)` in the root `CMakeLists.txt`. There is no `tests/CMakeLists.txt` and no glob, so a new file is invisible to the build until listed.

`test_all.py` / `test_one.py` are a separate Python round-trip harness (infer -> replay install -> diff the produced file tree against an installed mod). They are not the unit tests and need the `SALMA_*` env vars (see below); run `scripts\setup-env.bat` once to set them.

```powershell
python test_one.py <archive> <installed-mod-folder>   # debug one mod; add --full for byte-for-byte compare
```

### Frontend (`web/`)

```powershell
cd web
npm install        # one-time
npm run dev        # Vite dev server on :3000, proxies /api -> http://localhost:5000
npm run build      # tsc (type-check) THEN vite build -> web/dist/
npm run lint       # eslint . --max-warnings 0
```

Start `mo2-server.exe` on :5000 before `npm run dev`, or every `/api/*` call the SPA makes fails through the proxy. `npm run build` outputs to `web/dist/`, which the Crow server serves at `/`.

## CI gates (must pass before merge)

- `build.yml` is the primary gate and builds the RUST engine (the C++ is no longer built in CI): `cargo fmt --check`, `cargo clippy --all-targets --release -- -D warnings`, release build, `cargo test --release`, then the corpus-free Python checks (`package.py`, `smoke_ctypes.py`, `smoke_plugin.py`) and an artifact upload of the deployable `mo2-salma.dll`. Its second job builds the web dashboard (`npm run build` = tsc type-check then vite). Triggered by changes under `src/`, `tests/`, `tools/`, `web/`, the root cargo files, to `build.bat`/`test.bat`, or to `scripts/mo2-salma.py`.
- `test.yml` runs `ctest --preset ci` and is the ONLY workflow that builds the C++. It configures and builds independently. It gates the Crow server and `src/SalmaEngine.cpp`, the bridge from the server to the engine DLL.
- `eslint.yaml` runs `npm run lint` over `web/`, and only fires on `web/**` changes.
- `sonar.yml` runs a SonarCloud scan over both languages (`sonar.sources=src`, which holds the Rust engine and the C++ server). The Rust analyzer shells out to cargo + clippy itself, so the workflow installs the toolchain; and `sonar.rust.cargo.manifestPaths=Cargo.toml` is stated explicitly so a future move cannot silently drop Rust from the scan. `tests/golden/**` is excluded as test data.
- clang-tidy and clang-format no longer run anywhere automatically. clang-tidy was a `build.bat` step and clang-format was a `build.yml` step; both scripts now drive Rust. Run them by hand if you touch C++.

`build.yml` is path-triggered and runs on any branch. `test.yml`, `eslint.yaml` and `sonar.yml` trigger on `main` only, so they gate the merge rather than each push.

## Conventions and gotchas

- Project headers are `.hpp`, not `.h`; files are PascalCase with paired `.hpp`/`.cpp` (`Logger.hpp`/`Logger.cpp`). Most recent git history is the `.h` -> `.hpp` rename sweep, so new headers MUST be `.hpp`.
- Doc comments in headers use `/** */` block style, not `///`. `.cpp` files use `//`. Headers are processed by `doxide build` (scans `src/*.{hpp,cpp}`); attach symbols to the doc groups in `doxide.yml` with `@ingroup <Group>`. The commands `@file`, `@defgroup`, `@def`, `@fn`, `@var`, `@internal`, `@short` are unsupported and break the doc build.
- No em-dashes anywhere (code, comments, docs, prose). Use plain hyphens.
- Commits use a gitmoji prefix + short imperative subject, one concern each (`🚚 Rename ... to .hpp`, `🛂 ...` for security, `💄 Restyle ...`). No trailing period. Do not mix formatting-only churn with feature/bug work.
- C++ style (`.clang-format`, Google base): 4-space indent, Allman braces, 100 columns, left-aligned pointers/refs, sorted includes. `#pragma once` (no guards), braces on all control-flow bodies, `explicit` single-arg ctors, prefer return values over out-params, named structs over `std::pair`/`std::tuple`, no `using namespace` in headers.
- Rust style: whatever `cargo fmt` produces, and `cargo clippy --all-targets -- -D warnings` must be clean. One module per C++ translation unit, snake_cased. Module and item docs are `//!` and `///` (the `/** */` rule is C++-only). Every deliberate divergence from the C++ gets a comment naming what the C++ does and why this differs, plus an entry in `PARITY-NOTES.md`; a reproduction of a C++ bug must say that it is one, or someone will "fix" it and break parity.
- New frontend backend calls go through `web/src/api.ts` (typed `fetch` wrapper that injects the `X-Salma-Csrf` header and retries on 403), not ad-hoc `fetch`. Shared TS types live in `web/src/types.ts`. Reusable components are in `web/src/comps/` (the old `web/src/components/` was removed). ESLint allows zero warnings; unused vars must be prefixed `_`.
- `docs/` and `site/` are generated (doxide -> `scripts/_clean_docs.py` -> mkdocs). Do not commit them; only `docs/main.html` (the theme override) is tracked.
- Respect existing user edits, do not revert unrelated changes, and prefer the repo's `.bat` scripts over ad hoc commands.

### Environment variables

- `VCPKG_ROOT` - required for configure (toolchain path).
- `SALMA_BIND_ADDR` - override the default `127.0.0.1` server bind; non-loopback values log a security warning.
- `SALMA_MODS_PATH`, `SALMA_DEPLOY_PATH`, `SALMA_DOWNLOADS_PATH` - MO2 mods dir, deploy target, and downloads root. Required for `deploy.bat`/`purge.bat` and the Python round-trip tests.

## Architecture

salma ships three artifacts over one engine. Improving inference once benefits both the MO2 plugin and the web UI; there is no duplicated logic to keep in sync. **Both consumers reach the engine through the same flat `extern "C"` ABI**, so neither can drift from the other.

- `mo2-salma.dll` (RUST, `src/*.rs`) - all engine logic, plus the eight-symbol C ABI. No HTTP. MO2 loads it through `scripts/mo2-salma.py` via ctypes; `mo2-server` loads it via `LoadLibrary`.
- `mo2-server` (EXE, C++) - the Crow HTTP layer in namespace `mo2server`. Serves the SPA and the `/api/*` REST endpoints, and calls the engine through `src/SalmaEngine.cpp`.
- `salma-support` (STATIC, C++) - what is left of `mo2core`: `Utils`, `Logger`, `SecurityContext`. Linked into `mo2-server` and `salma_tests`. Deliberately not a DLL named `mo2-salma`; that name belongs to the Rust artifact.
- `web/dist/` - the Vite-built React SPA, served by `mo2-server`.

`Export.hpp` defines `MO2_API`, which now expands to nothing because `salma-support` is static. The dllexport/dllimport spellings survive behind `MO2_CORE_SHARED` in case a DLL target returns.

### The engine (`src/*.rs`)

The Rust crate at the repo root IS the engine; the C++ engine it was ported from has been deleted. It produces `mo2_salma_rs.dll`, and `tools/package.py` renames it to `mo2-salma.dll` for deployment. It does NOT cover `mo2-server` or the web layer; there is no Crow equivalent, which is why the C++ server remains.

The module names still mirror the C++ translation units they were ported from, snake_cased, which is how PARITY-NOTES cross-references resolve:

```
Cargo.toml          root, one package, no workspace
build.rs            Win32 link flags for the vendored unrar sources
src/*.rs            beside the C++ it mirrors (FomodIRParser.cpp -> fomod_ir_parser.rs)
tests/*.rs          self-contained integration tests, beside the C++ tests/*.cpp
tools/*.py          packaging, smoke tests, and the C++-vs-Rust parity harnesses
```

`PARITY-NOTES.md` is the authoritative record of every divergence from the C++, organised by task, and `CUTOVER.md` covers swapping the deployed DLL and rolling back. Read both before changing engine behavior: many surprising-looking constructs are deliberate reproductions of C++ bugs, and they say so.

### Inference pipeline (the core feature)

Single entry point: `mo2core::FomodInferenceService::infer_selections(archive_path, mod_path)` in `FomodInferenceService.cpp` (~2500 LOC). It compares an archive's FOMOD options against an already-installed mod to recover which options were originally selected, and returns schema-v2 JSON (or `""` on failure; all exceptions are caught internally). Per-run state lives in the private `InferenceContext`. Stages, in order, with the file that owns each:

1. Tier-1 meta.ini shortcut - `try_fomod_plus_json()` reads cached fomod-plus JSON from `<mod>/meta.ini`. Treated as a candidate: it is forward-simulated and discarded if it does not reproduce the installed tree.
2. List archive entries - `ArchiveService::list_entries_with_sizes()` (libarchive default; bit7z for `.7z`/`.rar`/`.001`).
3. Locate and read `fomod/ModuleConfig.xml`, preferring the shallowest path.
4. Parse XML to the IR - `FomodIRParser::parse(...)` produces the `FomodInstaller` IR (`FomodIR.hpp`: Installer -> Step -> Group -> Plugin -> FileEntry, with recursive `FomodCondition` trees). The IR is the central data structure; parsing happens once and every later stage consumes the IR.
5. Expand atoms - `FomodInferenceAtoms.*` turns folder entries into per-file `FomodAtom`s (`ExpandedAtoms`, `AtomIndex` dest->atoms), preserving document order for conflict resolution.
6. Build the target tree - scan installed files into a `TargetTree` (dest -> size+hash); FNV-1a hash only contested files (bounded, mutex-guarded cache).
7. Constraint propagation - `propagate(...)` in `FomodPropagator.*`, a deterministic fixpoint (plugin-type, file-evidence, cardinality rules) that narrows each group's plugin domain. If fully resolved, the CSP solve is skipped.
8. Multi-phase CSP solve - `solve_fomod_csp(...)` in `FomodCSPSolver.cpp` returns a `[step][group][plugin]` selection grid. Internals are split across TUs: `FomodCSPPrecompute.*` (the read-only `Precompute`), `FomodCSPTypes.hpp` (solver datatypes), `FomodCSPOptions.cpp` (per-group option enumeration + SelectAny caps), and `FomodCSPSolverPhases.cpp` (the 5 phases: greedy/local-search/repair -> component decomposition -> residual repair -> focused search -> global fallback, each short-circuiting on an exact match).
9. Forward simulation as the scoring oracle - `FomodForwardSimulator::simulate()` replays a candidate selection in-memory using the SAME priority/document-order conflict resolution as the real installer (`FomodService::execute_file_operations`), then `evaluate_candidate()` diffs against the `TargetTree`. The solver and the Tier-1 check both score through this.
10. Assemble JSON - `assemble_json(...)` walks the IR + selection grid into schema-v2; `InferenceDiagnostics.*` accumulates per-decision reason codes.

### C ABI boundary

`src/capi.rs` is the flat `extern "C"` surface, used by BOTH consumers: the MO2 Python plugin via ctypes, and `mo2-server` via `src/SalmaEngine.cpp`. Each function returns a heap string the caller must release with `freeResult()`. The eight exports: `install`, `installWithConfig` (primary plugin install path, with a selections JSON), `inferFomodSelections`, `resolveModArchive`, `setLogCallback`, `getApiVersion`, `installSucceeded`, `freeResult`.

`SalmaEngine` restores two behaviors the ABI flattens away: a failed install THROWS (the controllers catch `std::exception`, and the ABI only returns an error string plus a false `installSucceeded()`), and the call is mutex-serialized because `installSucceeded()` is a process-global flag while the server runs installs on overlapping background jobs. Covered by `tests/salma_engine_test.cpp`.

### HTTP server

`main.cpp` builds a `crow::App<mo2server::SecurityMiddleware>`, instantiates the controllers and a `StaticFileHandler` (serves `web/dist` anchored to the exe dir, with SPA index.html fallback), bridges Crow logging into the salma `Logger`, and binds `127.0.0.1:5000`.

- `InstallationController` - `/api/installation/upload|install|status`; parses multipart, runs installs async on a `BackgroundJob`.
- `Mo2Controller` - one class whose handlers are split across TUs by concern: `Mo2ConfigController.cpp` (`/api/config`), `Mo2FomodController.cpp` (status/fomods/scan), `Mo2LogController.cpp`, `Mo2PluginController.cpp` (deploy/purge), `Mo2TestController.cpp`. Shared helpers in `Mo2Helpers.*`.
- `SecurityMiddleware` enforces an Origin allowlist and requires the `X-Salma-Csrf` header on state-changing methods; `/api/csrf-token` issues the token. `SecurityContext` (in `salma-support` so tests can link it without Crow) holds the CSRF token and allowlist.

### Install replay and cross-cutting

- `installation_service.rs` - top-level install orchestrator: extract to temp -> detect FOMOD -> `fomod_service` (replay) or `mod_structure_detector` (non-FOMOD content-root copy) -> cleanup. A fresh instance per C-API call (not thread-safe by design).
- `fomod_service.rs` - install replay: dependency checks -> required/optional/conditional file passes -> `execute_file_operations` (stable sort by priority then document order). Uses `fomod_dependency_evaluator` to evaluate `FomodCondition` trees, shared with the propagator and simulator.
- `logger.rs` (and the C++ `Logger.cpp` for the server) - writes to `logs/salma.log` next to the owning module with 10 MiB rotation, plus a lock-free atomic callback into the host (MO2 Python plugin). Subsystem tags by convention: `[infer]`, `[install]`, `[solver]`, `[archive]`, `[fomod]`, `[server]`, `[crow]`.
- `BackgroundJob<T>` (header-only) - generic async runner backing installs, scans, and plugin actions; safe to detach on shutdown via a shared-ptr-held state.
- `ConfigService` - reads/writes `salma.json` next to the exe (only persisted key: `mo2ModsPath`) via atomic write-then-rename; derives `fomod_output_dir` from the mods path.
- `utils.rs` / `Utils.hpp` - shared `to_lower`, `normalize_path`, `random_hex_string`, FNV-1a hashing, and path-safety guards (`is_safe_destination`, `is_inside`, `is_safe_mod_name`). The C++ copy kept only what the server uses; its pugixml-typed XML helpers went with the parser.
- Security guardrails worth knowing before touching extraction or upload code: 256 MiB per-archive-entry cap (`kMaxEntrySize`), 512 MiB upload cap (`kMaxUploadBytes`, returns 413 early), path-traversal rejection in `ArchiveService`, and shell-metachar/whitelist sanitization in the deploy/purge/test controllers before any `cmd.exe` spawn.
