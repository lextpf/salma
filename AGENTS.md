# Repository guidance

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

## Repository map

salma infers FOMOD selections from an installed mod and replays installations from archives.
The Rust engine is shared by a Python Mod Organizer 2 plugin and a C++ HTTP server with a React UI.

- `src/*.rs`: Rust 2024 engine (`mo2_salma_rs`). `lib.rs` declares modules; `capi.rs` owns the
  flat C ABI. Archive handling, XML parsing, inference, and installation belong here.
- `src/*.{hpp,cpp}`: C++23 Crow server and support code. `main.cpp` starts the server;
  `SalmaEngine` loads the engine DLL; controllers expose HTTP endpoints.
- `scripts/mo2-salma.py`: MO2 plugin. Other scripts handle packaging, scanning, installation,
  comparison, smoke checks, and documentation processing.
- `web/src/`: React and TypeScript dashboard. `api.ts` owns HTTP calls, `types.ts` defines
  shared frontend types, hooks manage behavior, and `comps/` contains reusable components.
- `tests/`: Rust integration tests and C++ GoogleTest sources. Rust unit tests also live in
  their source modules. `test_all.py` and `test_one.py` exercise mod installation round trips.
- `CMakePresets.json`, `Cargo.toml`, `web/package.json`, and `.github/workflows/`: build and
  validation configuration. Rust and C++ sources deliberately share `src/` and `tests/`.

## Build and validation

Commands below run from the repository root in PowerShell unless another directory is specified.
The supported application environment is Windows x64. The engine requires Rust 1.85+ and the
MSVC toolchain; the server requires VS 2022, CMake 3.20+, and `VCPKG_ROOT`. The default CMake
preset uses the `x64-windows-static-md` triplet. Frontend dependencies are managed in `web/`.

Choose validation for the affected layers. Add regression coverage for non-trivial behavior
changes, especially parsing, serialization, inference, installation, and public API changes.
Report checks actually run and any skipped stages or missing prerequisites.

### Rust engine and ABI

```powershell
cargo fmt --check
cargo clippy --all-targets --release -- -D warnings
cargo test --release
cargo build --release
python scripts/package.py --no-build
python scripts/smoke_ctypes.py target/release/mo2_salma_rs.dll
python scripts/smoke_plugin.py
```

Cargo produces `target/release/mo2_salma_rs.dll`; packaging stages
`target/package/mo2-salma.dll`. Build the DLL before running smoke checks.
For focused tests, use `cargo test --release -- utils::` or
`cargo test --release --test inference_diagnostics_test` as appropriate.

### C++ server

Build and package the Rust DLL before configuring CMake so its DLL staging hooks are installed.

```powershell
cmake --preset default
cmake --build --preset default
ctest --preset default
```

Use `cmake --build --preset tests-only` for the C++ test target alone. Engine copies beside
executables can remain stale after Rust-only builds; verify the DLL used by the host under test.

### Frontend

Run these commands from `web/`:

```powershell
npm ci
npm run lint
npm run build
```

The build runs TypeScript checking and Vite. There is no `npm test` script. For interactive
development, start the backend on port 5000 before `npm run dev` starts Vite on port 3000.

### Repository scripts and integration tests

- `./build.bat` runs formatting, Clippy, engine build and packaging, server build, frontend
  build, and documentation generation. It formats Rust in place and can skip optional tool
  stages; inspect its output. It does not build the C++ test target.
- `./test.bat` runs release Rust tests and both Python smoke checks. Build the DLL first.
  Python checks can be skipped when Python is unavailable. Run CTest separately for C++ changes.
- `./run.bat` starts the dashboard after a build; it does not build the application.
- Use `python scripts/run_harness.py` for live mod round trips. It stages and verifies the
  DLL under test; direct `test_all.py` can pick up an older deployed DLL. These tests need
  configured MO2 data paths, including `SALMA_MODS_PATH` and `SALMA_DOWNLOADS_PATH`.
  Use dedicated test data and scratch space: the harness deletes its `--tmp-base` directory.
- `setup.bat` persists local `SALMA_*` paths. `deploy.bat` writes the plugin into
  `SALMA_DEPLOY_PATH`; these are environment setup and deployment steps, not routine checks.

## Cross-language contracts

- Keep archive processing, FOMOD logic, and installation behavior in the Rust engine. Both
  hosts should use the ABI rather than implement separate versions of that behavior.
- Coordinate ABI changes in `src/capi.rs`, the C++ loader, Python consumers, and smoke tests.
  The ABI version in `capi.rs` controls the shipped version; Cargo's package version is separate.
- Free owned non-null ABI string results through `freeResult`. The pointer from `getApiVersion`
  is static and must not be freed. Preserve panic containment at the ABI boundary.
- Keep an install call and its `installSucceeded` read serialized: installation status is
  process-global. Preserve callback lifetimes and result ownership across host boundaries.
- Keep schema-v2 selection output, install replay, and frontend consumers consistent when
  changing JSON. Preserve archive path-safety checks and HTTP Origin/CSRF enforcement.
  Frontend API requests should use the existing helpers in `web/src/api.ts`.

## Style and documentation

- Follow `.clang-format` for C++ and rustfmt for Rust. Match the surrounding language's naming
  and structure; avoid unrelated renames or whole-repository formatting changes.
- C++ uses four-space indentation, Allman braces, and a 100-column limit. Headers start with
  `#pragma once`; group the matching header, project headers, external headers, then standard
  headers. Use braces for control-flow bodies and make ownership explicit through RAII.
- Follow the shared Doxygen-style annotations in `CONTRIBUTING.md` across languages. Document
  the canonical declaration once, with plain line comments for implementation notes.
- Rust documentation uses `/*! ... */` for module overviews and `/** ... */` for items,
  with prefixed content lines. Python uses docstrings; C++ and TypeScript use structured blocks.
  Callable documentation includes `@fn`, `@brief`, `@author`, and applicable parameter/return tags.
- Keep documentation in sentence case, briefs plain text, and documentation lines within
  100 columns. Use `@return`, source-aligned tables, and Material icon section headings.
  Rust unsafe contracts use `### :material-shield-lock: **Safety**`. Preserve useful diagrams
  and explanations of algorithm invariants.
- Edit source comments and documentation tooling. C++ docs are generated by `doxide build`,
  `python scripts/_clean_docs.py`, then `mkdocs build`. Rust docs use
  `cargo doc --no-deps --release --document-private-items`. When assembling the combined site,
  copy rustdoc output into `site/rust/` after MkDocs, which clears `site/`.

## Working tree hygiene

Preserve existing user changes and keep edits within the requested scope. Generated outputs
include `build/`, `build-*/`, `target/`, `web/dist/`, `site/`, and most of `docs/`.
`docs/main.html` is tracked source and must be preserved. Keep temporary investigation files
in `.tmp/` and avoid committing generated output, logs, local paths, or environment files.
Before finishing, review the task's diff and run `git diff --check` for the changed files.
