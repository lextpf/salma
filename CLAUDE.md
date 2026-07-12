# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Rules

Ask when unclear. If intent, architecture, or requirements are ambiguous, ask before coding.

Flag uncertainty. If an approach, dependency, or technical detail is uncertain, say so before proceeding.

Challenge bad direction. If my request conflicts with settled practice or likely long-term maintainability, point it out and suggest a better path.

End with omissions. After each task, state what you changed and what you intentionally did not do.

## What this is

salma is a wizardless FOMOD installer, processor, and selection-inference engine for Skyrim modding (Mod Organizer 2 integration). C++23 backend + React frontend, Windows-only (MSVC 2022).

## Build, test, run

VCPKG_ROOT must be set (the CMake preset reads it for the toolchain). Dependencies are vcpkg manifest-mode (`vcpkg.json`) on the `x64-windows-static-md` triplet, supplied by the overlay in `cmake/`. First configure can take several minutes while vcpkg builds the cache.

```powershell
.\build.bat        # full pipeline: clang-format -> configure -> clang-tidy -> Release build -> docs
.\test.bat         # build salma_tests and run it
.\deploy.bat       # copy DLL + mo2-salma.py into the MO2 plugins dir
```

For a fast iteration loop, configure once then scope the build to a target:

```powershell
cmake --preset default                                    # Release configure (preset "debug" for Debug)
cmake --build build --config Release --target mo2-core    # the DLL (mo2-salma.dll), no HTTP
cmake --build build --config Release --target mo2-server  # the EXE (Crow HTTP server)
cmake --build build --config Release --target salma_tests # GoogleTest binary
.\build\bin\Release\mo2-server.exe                        # run the server on 127.0.0.1:5000
```

Build outputs land in `build/bin/Release/`: `mo2-salma.dll`, `mo2-server.exe`, `salma_tests.exe`.

### Tests

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

- `build.yml` runs `clang-format -i` over `src` + `tests` and fails if `git diff` is non-empty. Formatting is a hard gate: run the formatter, never hand-adjust layout. It also builds the web frontend (`npm ci; npm run build`) and the C++ Release targets.
- `test.yml` runs `ctest --preset ci`.
- `sonar.yml` runs a SonarCloud scan.
- clang-tidy runs only in local `build.bat` (fails the build on any issue), not in CI.

## Conventions and gotchas

- Project headers are `.hpp`, not `.h`; files are PascalCase with paired `.hpp`/`.cpp` (`Logger.hpp`/`Logger.cpp`). Most recent git history is the `.h` -> `.hpp` rename sweep, so new headers MUST be `.hpp`.
- Doc comments in headers use `/** */` block style, not `///`. `.cpp` files use `//`. Headers are processed by `doxide build` (scans `src/*.{hpp,cpp}`); attach symbols to the doc groups in `doxide.yml` with `@ingroup <Group>`. The commands `@file`, `@defgroup`, `@def`, `@fn`, `@var`, `@internal`, `@short` are unsupported and break the doc build.
- No em-dashes anywhere (code, comments, docs, prose). Use plain hyphens.
- Commits use a gitmoji prefix + short imperative subject, one concern each (`🚚 Rename ... to .hpp`, `🛂 ...` for security, `💄 Restyle ...`). No trailing period. Do not mix formatting-only churn with feature/bug work.
- C++ style (`.clang-format`, Google base): 4-space indent, Allman braces, 100 columns, left-aligned pointers/refs, sorted includes. `#pragma once` (no guards), braces on all control-flow bodies, `explicit` single-arg ctors, prefer return values over out-params, named structs over `std::pair`/`std::tuple`, no `using namespace` in headers.
- New frontend backend calls go through `web/src/api.ts` (typed `fetch` wrapper that injects the `X-Salma-Csrf` header and retries on 403), not ad-hoc `fetch`. Shared TS types live in `web/src/types.ts`. Reusable components are in `web/src/comps/` (the old `web/src/components/` was removed). ESLint allows zero warnings; unused vars must be prefixed `_`.
- `docs/` and `site/` are generated (doxide -> `scripts/_clean_docs.py` -> mkdocs). Do not commit them; only `docs/main.html` (the theme override) is tracked.
- Respect existing user edits, do not revert unrelated changes, and prefer the repo's `.bat` scripts over ad hoc commands.

### Environment variables

- `VCPKG_ROOT` - required for configure (toolchain path).
- `SALMA_BIND_ADDR` - override the default `127.0.0.1` server bind; non-loopback values log a security warning.
- `SALMA_MODS_PATH`, `SALMA_DEPLOY_PATH`, `SALMA_DOWNLOADS_PATH` - MO2 mods dir, deploy target, and downloads root. Required for `deploy.bat`/`purge.bat` and the Python round-trip tests.

## Architecture

salma ships three artifacts over one shared core library. Improving inference once benefits both the MO2 plugin and the web UI; there is no duplicated logic to keep in sync.

- `mo2-core` (SHARED, output `mo2-salma.dll`) - all engine logic in namespace `mo2core`, plus the flat `extern "C"` ABI. No Crow, no HTTP. MO2 loads it through `scripts/mo2-salma.py` via ctypes.
- `mo2-server` (EXE) - links `mo2-core` and adds the Crow HTTP layer in namespace `mo2server`. Serves the SPA and the `/api/*` REST endpoints.
- `web/dist/` - the Vite-built React SPA, served by `mo2-server`.

`Export.hpp` defines the `MO2_API` macro (dllexport when building mo2-core, dllimport for consumers).

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

`CApi.hpp`/`CApi.cpp` (namespace `CApi`) is the flat `extern "C"` surface used by the Python plugin and tests. Each function is a thin wrapper that returns a heap string the caller must release with `freeResult()`. Key exports: `install`, `installWithConfig` (primary plugin install path, with a selections JSON), `inferFomodSelections`, `resolveModArchive`, `setLogCallback`, `getApiVersion`, `installSucceeded`.

### HTTP server

`main.cpp` builds a `crow::App<mo2server::SecurityMiddleware>`, instantiates the controllers and a `StaticFileHandler` (serves `web/dist` anchored to the exe dir, with SPA index.html fallback), bridges Crow logging into the salma `Logger`, and binds `127.0.0.1:5000`.

- `InstallationController` - `/api/installation/upload|install|status`; parses multipart, runs installs async on a `BackgroundJob`.
- `Mo2Controller` - one class whose handlers are split across TUs by concern: `Mo2ConfigController.cpp` (`/api/config`), `Mo2FomodController.cpp` (status/fomods/scan), `Mo2LogController.cpp`, `Mo2PluginController.cpp` (deploy/purge), `Mo2TestController.cpp`. Shared helpers in `Mo2Helpers.*`.
- `SecurityMiddleware` enforces an Origin allowlist and requires the `X-Salma-Csrf` header on state-changing methods; `/api/csrf-token` issues the token. `SecurityContext` (in mo2-core so tests can link it without Crow) holds the CSRF token and allowlist.

### Install replay and cross-cutting

- `InstallationService::install_mod` - top-level install orchestrator: extract to temp -> detect FOMOD -> `FomodService` (replay) or `ModStructureDetector` (non-FOMOD content-root copy) -> cleanup. A fresh instance per C-API call (not thread-safe by design).
- `FomodService` - install replay: dependency checks -> required/optional/conditional file passes -> `execute_file_operations` (stable sort by priority then document order). Uses `FomodDependencyEvaluator` to evaluate `FomodCondition` trees, shared with the propagator and simulator.
- `Logger` (Meyer singleton, thread-safe) - writes to `logs/salma.log` next to the DLL with 10 MiB rotation, plus a lock-free atomic callback into the host (MO2 Python plugin). Subsystem tags by convention: `[infer]`, `[install]`, `[server]`, `[crow]`.
- `BackgroundJob<T>` (header-only) - generic async runner backing installs, scans, and plugin actions; safe to detach on shutdown via a shared-ptr-held state.
- `ConfigService` - reads/writes `salma.json` next to the exe (only persisted key: `mo2ModsPath`) via atomic write-then-rename; derives `fomod_output_dir` from the mods path.
- `Utils.hpp` - shared `to_lower`, `normalize_path`, `random_hex_string`, `get_ordered_nodes`, FNV-1a hashing, and path-safety guards (`is_safe_destination`, `is_inside`, `is_safe_mod_name`).
- Security guardrails worth knowing before touching extraction or upload code: 256 MiB per-archive-entry cap (`kMaxEntrySize`), 512 MiB upload cap (`kMaxUploadBytes`, returns 413 early), path-traversal rejection in `ArchiveService`, and shell-metachar/whitelist sanitization in the deploy/purge/test controllers before any `cmd.exe` spawn.
