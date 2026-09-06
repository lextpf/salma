# CLAUDE.md

## Rules

1. Ask when unclear. If intent, architecture, or requirements are ambiguous, ask before coding.
2. Flag uncertainty. If an approach, dependency, or technical detail is uncertain, say so before proceeding.
3. Challenge bad direction. If my request conflicts with settled practice or likely long-term maintainability, point it out and suggest a better path.
4. End with omissions. After each task, state what you changed and what you intentionally did not do.

## Documentation

Four rules everywhere in comments and docstrings.

1. What, why, how, in that order, and only as much as is needed. In prose, never restate the signature; add units, ranges and nullability, or say nothing.
2. Sentence case. Capitalize sentence starts, command descriptions, and section titles after icons. Preserve the case of commands, identifiers, URLs, and icon shortcodes.
3. No archaeology. Do not write prose about the history of a class throughout the life cycle of this repository or what an earlier implementation did.
4. Concise. Short sentences, active voice, one idea each, one term per concept per file. A set of cases wants a table; a flow wants a diagram.

Document the code using ASD-STE100-inspired Simplified Technical English: use short, direct sentences, one term per concept, active voice, explicit conditions, and avoid idioms, unnecessary synonyms, or ambiguous wording. Focus documentation on intent, constraints, side effects, and non-obvious behavior;
Write for an engineer who knows the language but not this system.
Read [CONTRIBUTING.md](CONTRIBUTING.md) before writing or reviewing code documentation.

## What this is

salma is a wizardless FOMOD installer and selection-inference engine for Mod Organizer 2.
Given a mod archive and an already-installed mod directory, it infers which FOMOD options the
install was built from, and it can replay an install without showing a wizard.

Windows-only. Three deliverables, built by three different toolchains:

| Artifact | Built by | Language |
|---|---|---|
| `target/package/mo2-salma.dll` (the engine) | Cargo | Rust 2024, 1.85+ |
| `build/bin/Release/mo2-server.exe` (HTTP host) | CMake + vcpkg | C++23 |
| `web/dist/` (dashboard SPA) | Vite | React 18 + TypeScript |

## Commands

```powershell
.\build.bat     # fmt -> clippy -D warnings -> cargo build -> package.py -> cmake -> npm build -> docs
.\test.bat      # cargo test --release + smoke_ctypes.py + smoke_plugin.py
.\run.bat       # starts mo2-server.exe (:5000) and the Vite dev server (:3000). Does not build.
.\setup.bat     # one-time: persists SALMA_MODS_PATH / SALMA_DEPLOY_PATH / SALMA_DOWNLOADS_PATH
.\deploy.bat    # copies the DLL + mo2-salma.py into a live MO2 instance (needs SALMA_DEPLOY_PATH)
```

Iterating on one half:

```powershell
cargo build --release && python scripts\package.py --no-build   # engine only, then stage it
cmake --preset default                                          # once; reads VCPKG_ROOT
cmake --build build --config Release --target mo2-server
cmake --build build --config Release --target salma_tests
cd web && npm run dev      # needs mo2-server already up on :5000, Vite proxies /api there
```

Running a subset of tests:

```powershell
cargo test --release -- utils::                          # one module's inline unit tests
cargo test --release --test inference_diagnostics_test   # the only Rust integration test file
cargo test --release fomod_csp_solver::tests::name_of_test
.\build\bin\Release\salma_tests.exe --gtest_filter=SalmaEngine.*
ctest --preset ci                                        # what test.yml runs
python scripts\run_harness.py                            # live round-trip over SALMA_MODS_PATH
cd web && npm run lint                                   # eslint --max-warnings 0
```

Nearly all Rust tests live in `mod tests` blocks inside each `src/*.rs`; `tests/` holds one Rust
integration file plus the three GoogleTest `.cpp` files. New C++ test files must be added to the
`salma_tests` source list in `CMakeLists.txt`.

## Architecture

### One flat `src/`, two languages

`src/` mixes both halves with no subdirectories. `*.rs` is the Rust engine, `*.cpp`/`*.hpp` is the
C++ server. They share a directory but never link: the server loads the engine at runtime.

### The C ABI is the only boundary

`src/capi.rs` exports exactly eight `extern "C"` functions: `getApiVersion`, `setLogCallback`,
`install`, `installWithConfig`, `inferFomodSelections`, `installSucceeded`, `freeResult`,
`resolveModArchive`. Two hosts bind them:

- `scripts/mo2-salma.py`, the MO2 plugin, via `ctypes` (`find_dll` / `load_dll` /
  `_check_api_version`).
- `src/SalmaEngine.cpp`, via `LoadLibrary` + `GetProcAddress`, wrapped for the Crow controllers.

The export names are camelCase and the crate sets `#![allow(non_snake_case)]` for that reason:
`#[unsafe(no_mangle)]` emits the Rust identifier verbatim, so renaming one breaks both binders.

Ownership rule: every non-null `*const c_char` result must be released with `freeResult`, except
the static pointer from `getApiVersion`.

### Engine pipelines

Two paths through the engine, entered from `installation_service.rs` and
`fomod_inference_service.rs`:

```
inference: archive list -> fomod_ir_parser -> fomod_atom expansion -> installed scan + hash
           -> fomod_propagator (domain narrowing) -> fomod_csp_solver -> inference_diagnostics
install:   archive_service extraction -> fomod_service replay, or mod_structure_detector
           content-root copy -> file_operations
```

The CSP solver runs phased search (greedy/local repair, component search, residual repair, focused
search, widening global search) and stops on an exact match or a deadline. `fomod_propagator` only
narrows domains ahead of it; it never replaces the solve. `inference_diagnostics.rs` turns the
result into the schema-v2 JSON both hosts consume, including the confidence formula.

### Server and dashboard

`src/main.cpp` registers roughly two dozen `/api/*` Crow routes across the `Mo2*Controller` and
`InstallationController` files, behind `SecurityMiddleware` (Origin + CSRF). `StaticFileHandler`
serves `web/dist/`. `salma-support` is a small static lib (`Utils`, `Logger`, `SecurityContext`)
linked into both the server and the tests.

## Invariants and traps

- **Version has one source.** `MO2_SALMA_API_VERSION` in `src/capi.rs` owns the shipped version.
  `CMakeLists.txt` regex-parses that exact line for `VERSION` and CPack, and the plugin checks the
  major digit at load. `Cargo.toml`'s `version` is not the shipped version. Changing the spelling
  of that constant breaks the CMake parse, which hard-errors.
- **DLL staging happens before configure.** CMake copies `target/package/mo2-salma.dll` next to
  `mo2-server` and `salma_tests` only if the file `EXISTS` at *configure* time. Order is always
  `cargo build --release` -> `python scripts/package.py --no-build` -> `cmake --preset ...`. All
  three CI workflows do exactly this; skipping it makes `SalmaEngine.*` tests run with no engine.
- **The engine can silently go stale.** `build.bat` refreshes the copy beside the exe only when the
  C++ is rebuilt, so the dashboard can run an old engine. `run.bat` byte-compares the two copies
  and warns. `package.py` prints a SHA-256 because `getApiVersion` is identical across builds.
- **`find_dll` prefers the deployed DLL.** A plain `python test_all.py` may validate a binary you
  did not just build. Use `python scripts/run_harness.py`, which stages the DLL under test,
  redirects `SALMA_DEPLOY_PATH` for the child only, and re-hashes what the harness reports loading.
- **Process-global engine state.** `LAST_INSTALL_SUCCESS` in `capi.rs` and the sticky `DISK_FULL`
  flag in `file_operations.rs` are process-wide, so a host must serialize an install with its
  `installSucceeded` read. `SalmaEngine.cpp` holds a mutex across that pair. Inference and archive
  resolution are safe to run concurrently. Installs do not roll back partial output.
- **`ReasonCode` values are wire data.** Append codes in `inference_diagnostics.rs`; never renumber
  or repurpose existing ones.
- **Archive order differs by format.** ZIP keeps central-directory order with `/` paths; 7z, split
  7z, and RAR use case-insensitive order with `\` paths. Inference is sensitive to this.
- **`docs/` and `site/` are generated and gitignored** (except `docs/main.html`). Edit source
  comments, not generated pages. MkDocs clears `site/`, so rustdoc must be copied in last.
- **`.gitignore` has unanchored directory patterns** (`assets/`, `build-*/`). A new
  `web/src/assets/` would be silently untracked: the local build passes and Linux CI fails on a
  missing module.
- **CI floats the Rust toolchain** (`dtolnay/rust-toolchain@stable`, unpinned) while `Cargo.toml`
  only sets `rust-version = "1.85"`. To reproduce a CI-only clippy lint, install that specific
  toolchain and use `cargo +X.Y.Z clippy`; do not `rustup update` the default.

## Conventions

`CONTRIBUTING.md` is the authoritative house style; read it before writing anything non-trivial.
The parts most easily got wrong:

- **One documentation dialect for every language.** Doxygen-style annotations (`@brief`, `@author`,
  `@param`, `@return`, Material icon section headings, Mermaid diagrams) are used in Rust, C++,
  Python, and TypeScript alike. Do *not* switch to idiomatic per-language docs: Rust uses
  `/*! ... */` for module overviews and `/** ... */` on items, not `///` prose; Python does not use
  `Args:` sections.
- **Document the canonical declaration only.** For C++ that is the header; do not repeat the
  contract on the `.cpp` definition. Implementation notes inside bodies are plain `//` lines, and
  trailing member docs use `///<`.
- **Comment the reason, not the syntax.** Keep existing ASCII/Mermaid diagrams, worked examples and
  formulas in the algorithm-heavy engine files; they are house style, not clutter.
- Doc lines stay at or under 100 columns including the comment prefix.
- C++ naming: PascalCase files, types, namespaces and functions; camelCase locals and parameters;
  `m_` + PascalCase for class members; plain-data struct fields are unprefixed camelCase.
- Commit subjects are a single gitmoji plus one sentence, e.g.
  `🚨 Brace the void arrow body in Tabs`.
- Do not use em-dashes anywhere in this repository; use plain hyphens.
