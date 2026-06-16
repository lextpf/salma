# AGENTS.md

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

## Project Structure & Module Organization

The root Cargo crate is the FOMOD engine; Rust modules are in `src/*.rs`, with the C ABI in `src/capi.rs`. The C++23 Crow server shares `src/` through paired `PascalCase.hpp`/`.cpp` files. `tests/` contains Rust integration tests and C++ Google Tests. The React/TypeScript dashboard lives in `web/src/`, with components under `web/src/comps/`; automation is in `scripts/`. Treat `target/`, `build/`, `web/dist/`, `docs/`, and `site/` as generated.

## Build, Test, and Development Commands

- `.\build.bat`: run Rust formatting, Clippy, release build, DLL packaging, and available C++, web, and documentation steps.
- `cargo fmt --check`, `cargo clippy --all-targets --release -- -D warnings`, and `cargo test --release`: reproduce Rust CI.
- `cmake --preset default` then `cmake --build --preset default`: build the C++ server; set `VCPKG_ROOT` first.
- `.\test.bat`: run Rust release tests and ABI/plugin-loader smoke tests. Run `cmake --build --preset tests-only` and `ctest --preset default` for C++ tests.
- In `web/`, use `npm ci`, `npm run dev`, `npm run build`, and `npm run lint` to install, develop, type-check/build, and lint.
- `.\run.bat`: launch the built server and Vite dashboard.

## Coding Style & Naming Conventions

Use `rustfmt`; Rust modules/functions are `snake_case`, while types and traits are `PascalCase`. C++ follows `.clang-format`: four spaces, no tabs, Allman braces, sorted includes, and 100 columns. C++ filenames/types are `PascalCase`; functions/variables use `snake_case`, and private members end in `_`. Frontend code uses two spaces and single quotes. React components are `PascalCase`, hooks begin with `use`, and utilities use `camelCase`. ESLint permits intentionally unused names only with an `_` prefix.

## Testing Guidelines

Test every non-trivial behavior change and regression. Keep Rust unit tests beside their module under `#[cfg(test)]`; name integration files `*_test.rs`. Name Google Test files `*_test.cpp` and use behavior-oriented cases. There is no numeric coverage threshold, but parser, inference, installation, security, serialization, and public-API changes need focused coverage.

## Commit & Pull Request Guidelines

History uses an emoji prefix and short imperative subject, such as `🐛 Stop ...`, `♻️ Back ...`, or `📝 Document ...`. Keep commits and PRs focused. PRs should explain what changed, why, tradeoffs, and validation; link issues and include screenshots for dashboard changes. Do not mix features with unrelated formatting or refactors.

## Configuration & Safety

Keep `VCPKG_ROOT` and machine-specific `SALMA_*` paths local. Do not commit credentials, MO2 paths, generated artifacts, or logs. Preserve path-traversal, upload-size, and archive-entry safeguards when changing file or archive handling.
