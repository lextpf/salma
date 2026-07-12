# Repository Guidelines

## Project Structure & Module Organization

This repository contains a C++ backend/plugin and a React frontend. Core C++ code lives in `src/`, with service, controller, parser, solver, logging, and utility modules split into paired `.hpp`/`.cpp` files. C++ unit tests live in `tests/` and build into `salma_tests`. The web UI lives in `web/`, with reusable components in `web/src/comps/`; pages, hooks, and utilities sit at the `web/src/` root. Helper automation is in `scripts/`; generated documentation output is under `docs/` and `site/`. Build artifacts belong in `build/`, `build-local/`, or `build-sandbox/`.

## Build, Test, and Development Commands

- `.\build.bat`: format C++ files, configure CMake, build Release targets, and optionally generate docs.
- `cmake --preset default`: configure the Release build using `vcpkg` and `cmake/x64-windows-static-md.cmake`.
- `cmake --build build --config Release`: build the C++ targets.
- `.\test.bat`: build and run the `salma_tests` Google Test suite.
- `cd web; npm run dev`: start the Vite development server.
- `cd web; npm run build`: type-check and build the frontend.
- `cd web; npm run lint`: run ESLint with zero warnings allowed.

## Coding Style & Naming Conventions

C++ formatting is defined by `.clang-format`: Google-based style, 4-space indentation, Allman braces, 100-column limit, left-aligned pointers/references, and sorted includes. Run the formatter instead of hand-adjusting layout. Prefer focused service/controller classes and existing filename patterns such as `FomodService.cpp`.

Frontend code uses TypeScript, React, ESLint, and Vite. Components and pages use `PascalCase`; hooks use `useThing.ts`; utilities use descriptive camelCase names.

## Testing Guidelines

Use Google Test for C++ tests in `tests/`. Name files after the behavior under test, for example `utils_test.cpp` or `fomod_inference_test.cpp`. Add or update tests for solver logic, parser behavior, installation flows, and bug fixes. Run `.\test.bat` before C++ submissions and `npm run lint` plus `npm run build` in `web/` before frontend submissions.

## Commit & Pull Request Guidelines

Recent history uses short, imperative commits with an emoji prefix, for example `💄 Restyle StatusCard`, `🩹 Fix clang-format`, or `📝 Update README.md`. Keep commits scoped to one concern.

Pull requests should stay focused, describe the change and rationale, list tests run, link related issues when applicable, and include screenshots for UI changes. Avoid mixing formatting-only churn with feature work.

## Agent-Specific Instructions

Respect existing user edits and do not revert unrelated changes. Prefer repository scripts over ad hoc commands, and avoid committing generated `build/` or `site/` output unless explicitly requested.
