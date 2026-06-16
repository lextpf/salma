# CLAUDE.md

## Rules

1. Ask when unclear. If intent, architecture, or requirements are ambiguous, ask before coding.
2. Flag uncertainty. If an approach, dependency, or technical detail is uncertain, say so before proceeding.
3. Challenge bad direction. If my request conflicts with settled practice or likely long-term maintainability, point it out and suggest a better path.
4. End with omissions. After each task, state what you changed and what you intentionally did not do.

## Documentation

Four rules everywhere in comments and docstrings.

1. What, why, how, in that order, and only as much as is needed. Never restate the signature; add units, ranges and nullability, or say nothing.
2. No shouting. Prose is lowercase. Capitals are for acronyms, identifiers copied from the code.
3. No archaeology. Do not write prose about the history of a class throughout the life cycle of this repository or what an earlier implementation did.
4. Concise. Short sentences, active voice, one idea each, one term per concept per file. A set of cases wants a table; a flow wants a diagram.

Document the code using ASD-STE100-inspired Simplified Technical English: use short, direct sentences, one term per concept, active voice, explicit conditions, and avoid idioms, unnecessary synonyms, or ambiguous wording. Focus documentation on intent, constraints, side effects, and non-obvious behavior;
Write for an engineer who knows the language but not this system.

## What this is

salma is a wizardless FOMOD installer, processor, and selection-inference engine for Skyrim modding, with Mod Organizer 2 integration. Windows-only (MSVC 2022 for the C++ half).

**The engine is Rust.** `src/*.rs` is the crate `mo2_salma_rs`, which builds `mo2-salma.dll`. The C++ that remains in `src/*.{hpp,cpp}` is the Crow HTTP server behind the web dashboard plus a small support library; the Rust port does not cover HTTP, which is why the C++ is still here. The C++ engine it was ported from has been deleted.

`src/` and `tests/` deliberately hold BOTH languages side by side. In `src/`, snake_case `.rs` files are the engine and PascalCase `.hpp`/`.cpp` pairs are the server. In `tests/`, cargo picks up `*.rs` and CMake picks up `*.cpp`; neither sees the other.

## Build, test, run

The root `.bat` scripts are the intended entry points and each one is a pipeline, not a one-liner.

```powershell
.\setup.bat        # ONCE: prompts for the three MO2 paths, setx-es SALMA_MODS/DEPLOY/DOWNLOADS_PATH
.\build.bat        # cargo fmt -> clippy (-D warnings) -> cargo build --release -> package the DLL
                   #   -> cmake mo2-server -> npm run build -> rustdoc + doxide/mkdocs into site\
.\test.bat         # cargo test --release -> smoke_ctypes.py -> smoke_plugin.py
.\run.bat          # start mo2-server.exe on :5000 and the Vite dev server on :3000, open the browser
.\deploy.bat       # copy the packaged DLL + scripts\mo2-salma.py into the MO2 plugins dir
.\purge.bat        # remove the deployed plugin, the FOMOD output dir, and the MO2-side log
```

`build.bat` steps 5 (cmake), 6 (npm) and 7 (doxide/mkdocs) SKIP silently when their tool is missing or `VCPKG_ROOT` is unset. If `run.bat` cannot find `mo2-server.exe`, that skip is why. `cargo doc` is NOT optional: a broken intra-doc link fails the build, because `Cargo.toml`'s `[lints.rustdoc]` denies it.

Both `build.bat` and `test.bat` default `CARGO_BUILD_JOBS=4` when unset. Cargo otherwise runs one job per core, and on a high-core host the parallel rustc + link peak has taken the toolchain down (rustc `STATUS_HEAP_CORRUPTION`, cc-rs failures building the unrar sources). Set the variable to override.

### Fast Rust loop

```powershell
cargo build --release                                    # target\release\mo2_salma_rs.dll
cargo test  --release                                    # the whole suite
cargo clippy --all-targets --release -- -D warnings
cargo fmt --check                                        # what CI enforces; build.bat rewrites in place
python scripts\package.py --no-build                     # stage target\package\mo2-salma.dll (+ sha256)
```

Running one test or one module:

```powershell
cargo test --release -- utils::                          # a module's unit tests
cargo test --release -- fomod_csp_options::tests::       # one module's tests, narrower
cargo test --release --test inference_diagnostics_test   # the only Rust integration test file
```

### The C++ server

`VCPKG_ROOT` must be set; the CMake preset reads it for the toolchain. Dependencies are vcpkg manifest-mode (`vcpkg.json`) on `x64-windows-static-md`, with the overlay triplet in `cmake/`. Only Crow, nlohmann-json and gtest remain, so configure takes ~20s; libarchive, pugixml and bit7z went with the engine.

```powershell
cmake --preset default                                     # Release configure (preset "debug" for Debug)
cmake --build build --config Release                       # everything
cmake --build build --config Release --target mo2-server   # the EXE
cmake --build build --config Release --target salma_tests  # the GoogleTest binary
cmake --preset compile-db                                  # Ninja sidecar, ONLY to emit compile_commands.json

.\build\bin\Release\salma_tests.exe --gtest_filter=SalmaEngine.*    # one suite
.\build\bin\Release\salma_tests.exe --gtest_list_tests              # enumerate
ctest --preset ci                                                   # what test.yml runs
```

Adding a C++ test means creating `tests/<subject>_test.cpp` AND adding it to the explicit source list in `add_executable(salma_tests ...)` in the root `CMakeLists.txt`. There is no `tests/CMakeLists.txt` and no glob, so a new file is invisible to the build until listed.

`cpack` from `build/` produces `salma-mo2-<ver>.zip` with two components (MO2_PLUGIN, DASHBOARD). The version is parsed out of `MO2_SALMA_API_VERSION` in `src/capi.rs` by a regex in `CMakeLists.txt`; changing that constant's spelling breaks configure with a FATAL_ERROR.

### Frontend (`web/`)

```powershell
cd web
npm install        # one-time
npm run dev        # Vite on :3000, proxies /api -> http://localhost:5000
npm run build      # tsc (type-check) THEN vite build -> web/dist/
npm run lint       # eslint . --max-warnings 0
```

`mo2-server.exe` must be up on :5000 or every `/api/*` call fails through the proxy. `run.bat` starts both halves.

### Round-trip harness against a live MO2 (not the unit tests)

A separate Python suite that infers selections, replays the install into a temp dir, and diffs the produced tree against the real installed mod. It needs the `SALMA_*` env vars and a real MO2 instance; nothing it reads lives in the repo.

```powershell
python scripts\run_harness.py                       # test_all.py, with the stale-DLL trap handled (see below)
python scripts\run_harness.py --limit 25
python test_all.py --no-full --separator "NAME"     # direct, if you know which DLL you are loading
python test_one.py <archive> <installed-mod-folder> --full    # one mod, byte-for-byte
```

## CI gates

- **`build.yml`** is the primary gate and builds the RUST engine only: `cargo fmt --check`, clippy `-D warnings`, release build, `cargo test --release`, then `scripts/package.py`, `scripts/smoke_ctypes.py`, `scripts/smoke_plugin.py`, and an artifact upload of the deployable DLL. A second job runs `npm run build` in `web/`, which is the frontend's type gate. Path-triggered, runs on any branch.
- **`test.yml`** runs `ctest --preset ci` and is the ONLY workflow that builds the C++. `main` only.
- **`eslint.yaml`** runs `npm run lint` over `web/`, on `web/**` changes to `main` only.
- **`sonar.yml`** scans both languages out of `src/`. The Rust analyzer shells out to cargo + clippy itself, so the workflow installs the toolchain, and `sonar.rust.cargo.manifestPaths=Cargo.toml` is stated explicitly so a future move cannot silently drop Rust from the scan.
- clang-tidy and clang-format run NOWHERE automatically any more. Both used to be `build.bat`/`build.yml` steps; those scripts now drive Rust. Run them by hand if you touch C++.

## Architecture

### The C ABI is the only boundary

`src/capi.rs` exports exactly eight `extern "C"` symbols, and BOTH consumers go through them: the MO2 plugin `scripts/mo2-salma.py` via ctypes, and `mo2-server` via `src/SalmaEngine.cpp` (`LoadLibrary`). Neither consumer can drift from the other, and improving inference once benefits both.

`install`, `installWithConfig`, `inferFomodSelections`, `resolveModArchive`, `setLogCallback`, `getApiVersion`, `installSucceeded`, `freeResult`.

Every non-null string return except `getApiVersion` is heap-allocated and must be released with `freeResult`. No panic may unwind across the boundary: every export routes through `guard()`, mirroring the C++ `catch (...)`. Export names are camelCase and `#![allow(non_snake_case)]` is on the module, because `#[unsafe(no_mangle)]` emits the Rust identifier verbatim as the symbol.

`SalmaEngine` restores two behaviors the flat ABI erases: a failed install THROWS (the Crow controllers catch `std::exception`, whereas the ABI only returns an error string plus a false `installSucceeded()`), and calls are mutex-serialized because `installSucceeded()` is a process-global flag while the server runs installs on overlapping background jobs. Covered by `tests/salma_engine_test.cpp`.

### Inference pipeline (the core feature)

Single entry point: `FomodInferenceService::infer_selections(archive_path, mod_path)` in `src/fomod_inference_service.rs`. It compares an archive's FOMOD options against an already-installed mod to recover which options were originally selected, and returns schema-v2 JSON, or `""` on ANY failure (no `Result` crosses the FFI boundary). Stages, in order, with the module that owns each:

1. **Tier-1 `meta.ini` shortcut** - `try_fomod_plus_json()` reads a cached fomod-plus JSON blob from `<mod>/meta.ini`. Treated as a candidate only: it is forward-simulated and discarded if it does not reproduce the installed tree.
2. **List archive entries** - `archive_service.rs::list_entries_with_sizes`. Backend by extension: `zip` for ZIP, `sevenz_rust2` for `.7z` and `.001`, `unrar` (which links the proprietary unRAR C sources) for RAR. Entry ORDER and path separators are per-format and load-bearing; the module doc explains why each is what it is.
3. **Locate and read `fomod/ModuleConfig.xml`**, preferring the shallowest path.
4. **Parse XML to the IR** - `fomod_ir_parser.rs` (roxmltree) produces the `FomodInstaller` IR in `fomod_ir.rs`: Installer -> Step -> Group -> Plugin -> FileEntry, with recursive `FomodCondition` trees. The IR is the central data structure; parsing happens once and every later stage consumes it.
5. **Expand atoms** - `fomod_inference_atoms.rs` turns folder entries into per-file `FomodAtom`s (`ExpandedAtoms`, plus an `AtomIndex` from dest to atoms), preserving document order for conflict resolution.
6. **Build the target tree** - scan installed files into a `TargetTree` (dest -> size + hash). FNV-1a hashing is lazy and only for contested files, behind a bounded mutex-guarded cache.
7. **Constraint propagation** - `fomod_propagator.rs`, a deterministic fixpoint (plugin-type, file-evidence, cardinality rules) narrowing each group's plugin domain. The service NEVER branches on `fully_resolved`; it always calls the solver, and the skip/seed logic lives inside the solver.
8. **Multi-phase CSP solve** - `solve_fomod_csp()` in `fomod_csp_solver.rs` returns a `[step][group][plugin]` selection grid. Split across modules: `fomod_csp_precompute.rs` (the read-only `Precompute`), `fomod_csp_types.rs` (solver datatypes), `fomod_csp_options.rs` (per-group option enumeration and SelectAny caps). Five phases, each short-circuiting on an exact match: greedy/local-search/repair -> component decomposition -> residual repair -> focused search -> global fallback with progressive SelectAny widening.
9. **Forward simulation as the scoring oracle** - `fomod_forward_simulator.rs::simulate()` replays a candidate selection in memory using the SAME priority and document-order conflict resolution as the real installer (`fomod_service.rs::execute_file_operations`), then diffs against the `TargetTree`. The solver and the Tier-1 check both score through this.
10. **Assemble JSON** - `assemble_json()` walks the IR plus the selection grid into schema-v2; `inference_diagnostics.rs` accumulates per-decision reason codes and confidence.

### Install replay and cross-cutting

- `installation_service.rs` - top-level orchestrator: extract to temp -> detect FOMOD -> `fomod_service` (replay) or `mod_structure_detector` (non-FOMOD content-root copy) -> cleanup. A fresh instance per C-API call, not thread-safe by design.
- `fomod_service.rs` - install replay: dependency checks -> required/optional/conditional file passes -> `execute_file_operations` (stable sort by priority then document order). Uses `fomod_dependency_evaluator.rs` for `FomodCondition` trees, shared with the propagator and the simulator.
- `archive_resolver.rs` - the `installationFile` -> archive fallback chain behind `resolveModArchive`.
- `json.rs` - the schema-v2 output must be byte-identical to the C++ `nlohmann::json::dump(2)`. The owned `Value` model stays (its `Int`/`UInt`/`Double` split is load-bearing: the same zero is `0` as a count and `0.0` as a confidence component), but `parse()`/`dump()` now delegate to `serde_json`, verified byte-identical on the output path. The module doc now states the delegation correctly and points at `PARITY-NOTES.md` "Replacing the hand-written JSON with serde_json", which remains the account of record.
- `logger.rs` (and C++ `Logger.cpp` for the server) - writes `logs/salma.log` NEXT TO the owning module, rotating at 10 MiB and keeping `.1` through `.3`, plus a lock-free atomic callback into the host. Subsystem tags by convention: `[infer]`, `[install]`, `[solver]`, `[archive]`, `[fomod]`, `[server]`, `[crow]`.
- `utils.rs` / `Utils.hpp` - `to_lower`, `normalize_path`, `random_hex_string`, FNV-1a, and the path-safety guards `is_safe_destination`, `is_inside`, `is_safe_mod_name`. The C++ copy kept only what the server uses.
- Security guardrails to know before touching extraction or upload code:
  - 256 MiB per-archive-entry cap, on the six read helpers only. `extract*` carries no such rejection, and only the ZIP path bounds even the up-front allocation (`src/archive_service.rs`, `MAX_ENTRY_SIZE`).
  - 8 GiB upload cap, enforced solely by `kMaxUploadBytes` in `InstallationController::parse_and_validate_upload`. It is not an early-out: Crow bounds no request body, so `req.body` is buffered whole before the check runs, and the 413 saves the multipart parse and the temp-file write but not the memory. `main.cpp`'s `stream_threshold` holds the same value but bounds a response.
  - path-traversal rejection in the archive layer.
  - a shell-metachar denylist (not a whitelist) over `deploy_path` and `mods_path` in the deploy/purge/test controllers. Those two travel in the child's environment block, where metacharacters mean nothing; the screening covers what the batch script does with them. `script_path` is the only value on the `cmd.exe` command line and is deliberately unscreened, on the precondition that it is `<exe dir>/deploy.bat` or `purge.bat`.

### HTTP server (`src/*.cpp`, namespace `mo2server`)

`main.cpp` builds a `crow::App<SecurityMiddleware>`, instantiates the controllers and a `StaticFileHandler` (serves `web/dist` anchored to the exe dir, with SPA index.html fallback), bridges Crow logging into the salma `Logger`, and binds `127.0.0.1:5000`.

- `InstallationController` - `/api/installation/upload|install|status/<id>`; parses multipart, runs installs async on a `BackgroundJob<T>` (header-only generic async runner, safe to detach on shutdown).
- `Mo2Controller` - one class whose handlers are split across TUs by concern: `Mo2ConfigController.cpp` (`/api/config`), `Mo2FomodController.cpp` (`/api/mo2/status|fomods|fomods/scan`), `Mo2LogController.cpp` (`/api/logs*`), `Mo2PluginController.cpp` (`/api/plugin/deploy|purge|status`), `Mo2TestController.cpp` (`/api/test/*`). Shared helpers in `Mo2Helpers.*`.
- `SecurityMiddleware` enforces an Origin allowlist and requires `X-Salma-Csrf` on state-changing methods; `/api/csrf-token` issues it. `SecurityContext` lives in `salma-support` so tests can link it without Crow.
- `ConfigService` reads/writes `salma.json` next to the exe (only persisted key: `mo2ModsPath`) via atomic write-then-rename.

`Export.hpp`'s `MO2_API` expands to nothing because `salma-support` is static; the dllexport/dllimport spellings survive behind `MO2_CORE_SHARED` in case a DLL target returns.

### Frontend (`web/src/`)

React 18 + react-router 7 + Tailwind v4 (via `@tailwindcss/vite`, no config file; tokens are declared in an `@theme` block in `index.css`). Four pages under one `Layout`: `InstallPage` (index), `LibraryPage` (`/fomods`, `/fomods/:name`), `LogsPage`, `SettingsPage`.

- All backend calls go through `web/src/api.ts`, a typed `fetch` wrapper that injects `X-Salma-Csrf`, retries once on a 403 whose body says the token is invalid, and applies a 30s timeout. Do not add ad-hoc `fetch`.
- Shared types in `web/src/types.ts`. Reusable components are FLAT in `web/src/comps/` (the earlier `comps/{chrome,install,library,logs,settings}/` nesting and the older `web/src/components/` are both gone). Hooks are `useThing.ts` at the `web/src/` root.
- `index.css` owns design tokens, resets, and the interaction states inline styles cannot express (`:hover`, `:focus-visible`, animations). Component styling lives in inline style objects in the TSX. The file's header comment states the five rules the visual system rests on; read it before restyling anything.
- `features.ts` holds flags for inspector tabs whose data the backend cannot produce yet (`conflicts`, `stateFlags`); the UI renders them as labelled placeholders.
- ESLint allows zero warnings; unused vars must be prefixed `_`.

## Conventions and gotchas

- **No em-dashes anywhere** (code, comments, docs, prose). Use plain hyphens.
- **Commits**: gitmoji prefix plus a short imperative subject, one concern each (`🚚 Rename ... to .hpp`, `🛂 ...` for security, `💄 Restyle ...`). No trailing period, no `Co-Authored-By` trailer. Do not mix formatting-only churn with feature or bug work.
- **Rust style**: whatever `cargo fmt` produces, and clippy `-D warnings` clean. Module and item docs are `//!` and `///` (the `/** */` rule below is C++-only). Module names still mirror the C++ translation units they were ported from, snake_cased (`FomodIRParser.cpp` -> `fomod_ir_parser.rs`), which is how PARITY-NOTES cross-references resolve. Keep it that way.
- **Parity discipline**: `PARITY-NOTES.md` is the authoritative record of every divergence from the deleted C++, organised by task. Read the relevant section before changing engine behavior. Many surprising-looking constructs are deliberate reproductions of C++ bugs and say so in a comment; a reproduction that does not say so will get "fixed" by someone and break parity. `CUTOVER.md` covers deploying the Rust DLL and rolling back, including the one open blocker (memory use on archives that expand enormously).
- **C++ style**: headers are `.hpp`, never `.h`; PascalCase with paired `.hpp`/`.cpp`. Headers use `/** */` blocks for file, type and function documentation, `///` one-liners where a block is overkill, and `///<` for trailing member docs - `///<` is the only trailing form, never `/**< */`; `.cpp` files use plain `//` and carry no Doxygen commands except `@author`. `.clang-format` (Google base) gives 4-space indent, Allman braces, 100 columns, left-aligned pointers, sorted includes, `#pragma once`. `CONTRIBUTING.md` is the full style guide.
- **Doxide**: every group in `doxide.yml` MUST have at least one `@ingroup <Group>` in `src/*.hpp`, or it silently generates nothing while `mkdocs.yml`'s nav points at a page that never appears. Tag from a header, never a `.cpp`: doxide scans `src/*.cpp` too, but a Doxygen command in a `.cpp` breaks the header/source split, and none is there today. `@file`, `@defgroup`, `@def`, `@fn`, `@var`, `@internal`, `@short` are unsupported and break the build. A namespace cannot carry `@ingroup`; tag its types individually.
- **Doxide is not Doxygen, and the difference is silent.** doxide 0.9.0 does NOT support `@par`, `@f[`/`@f$`, `@name`/`@{`/`@}`, `\htmlonly` or `@code{.cpp}`: it emits them as literal text on the page, and `@}` additionally corrupts the following member's description cell. `@brief` and `@author` also leak, and only survive because `scripts/_clean_docs.py` strips them. Use markdown instead - `## Heading`, `$$ math $$`, and fenced `cpp`/`text`/`mermaid` blocks all pass through untouched, and `mkdocs.yml` already registers the mermaid superfence and MathJax. `CONTRIBUTING.md` documents the full substitution table. The sibling `rift` repo uses real Doxygen, so its header style cannot be copied verbatim.
- **`docs/` and `site/` are generated** (doxide -> `scripts/_clean_docs.py` -> mkdocs, plus rustdoc copied into `site/rust/`). Only `docs/main.html` (the theme override) and the `.gitkeep`s are tracked. `docs/` is not cleaned automatically, so when changing groups or nav, wipe it first (keeping `main.html`) and rebuild: stale pages satisfy nav entries that a fresh clone cannot.
- **Two stale-DLL traps.** (1) `scripts/common.py::find_dll` searches `$SALMA_DEPLOY_PATH/salma/mo2-salma.dll` FIRST, which on a dev box is the DEPLOYED build, so a naive `python test_all.py` can validate a binary you did not just build. `scripts/run_harness.py` exists to solve exactly this: it stages the DLL under test, redirects `SALMA_DEPLOY_PATH` for the child process only, and re-hashes the file the harness reports loading. (2) CMake copies `target/package/mo2-salma.dll` next to `mo2-server.exe` only when the C++ is rebuilt, so a plain `build.bat` can leave the server on an old engine; `run.bat` byte-compares them and warns.
- **The cargo artifact is `mo2_salma_rs.dll`; the deploy name is `mo2-salma.dll`.** `scripts/package.py` does the rename, deliberately as an explicit auditable step rather than something the build does silently.
- **Do not commit fixtures derived from real mods.** The golden corpus built from a real mod list was removed along with the tests that consumed it: those tests existed to prove byte parity against the C++ engine, which no longer exists to compare against. Keep new tests self-contained.
- `.gitignore` uses `build/*` and `site/*` with `!`-negated `.gitkeep`s. Watch unanchored directory patterns: an entry like `assets/` matches at ANY depth, which has previously hidden web source from a clean checkout so that a local build passed while Linux CI failed with "Cannot find module".
- In batch scripts, `exit /b N` inside a parenthesized block that has already run a `CALL` is swallowed. Use a `GOTO` to a labelled exit instead; `build.bat` and `run.bat` are written that way.
- `README.md`'s Architecture section was rewritten to match the Rust engine and no longer describes a `mo2-core` library, libarchive or bit7z. `README.md`, this file, `PARITY-NOTES.md` and `CUTOVER.md` are all current; if they disagree in future, `PARITY-NOTES.md` wins on engine behavior and `CUTOVER.md` on deployment.
- Prefer the repo's `.bat` scripts over ad hoc commands, and do not revert unrelated user edits.

### Environment variables

- `VCPKG_ROOT` - required to configure the C++ (toolchain path). Unset means `build.bat` skips the server.
- `SALMA_MODS_PATH`, `SALMA_DEPLOY_PATH`, `SALMA_DOWNLOADS_PATH` - MO2 mods dir, plugins dir, downloads root. Required by `deploy.bat`, `purge.bat` and every Python harness script; `scripts/common.py` reads the first two at IMPORT time and exits 2 with setup guidance if unset. Run `setup.bat` once.
- `SALMA_BIND_ADDR` - override the server's `127.0.0.1` bind; non-loopback values log a security warning.
- `CARGO_BUILD_JOBS` - see above; the scripts default it to 4.
- `SALMA_NO_PAUSE=1` - suppress the trailing `pause` in `test.bat`, `deploy.bat`, `purge.bat` and `setup.bat`. Set it when invoking them non-interactively.
