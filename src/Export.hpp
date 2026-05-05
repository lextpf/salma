#pragma once

/**
 * @brief DLL export/import macro.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * `salma-support` (Utils, Logger, SecurityContext) is a static library linked
 * directly into `mo2-server` and `salma_tests`. Static linkage needs no
 * decoration, so `MO2_API` expands to nothing in every build this repository
 * produces. The engine DLL (`mo2-salma.dll`) is built from the Rust crate at
 * the repo root and exports its own flat C ABI, not these symbols.
 *
 * The shared-library spellings are kept behind `MO2_CORE_SHARED` so these
 * headers still compile if a DLL target is ever added.
 *
 * **Expansion**
 *
 * | MO2_CORE_SHARED | _WIN32 | MO2_CORE_EXPORTS | MO2_API expands to                     |
 * |-----------------|--------|------------------|----------------------------------------|
 * | not defined     | any    | any              | nothing (the current build)            |
 * | defined         | yes    | yes              | __declspec(dllexport)                  |
 * | defined         | yes    | not defined      | __declspec(dllimport)                  |
 * | defined         | no     | any              | __attribute__((visibility("default"))) |
 *
 * **Notes**
 *
 * A shared build needs `MO2_CORE_SHARED` defined in every consumer of these
 * headers, plus `MO2_CORE_EXPORTS` in the library target itself. Defining
 * `MO2_CORE_SHARED` in only some consumers produces a `dllimport` declaration
 * linked against a `dllexport` definition.
 *
 * The non-Windows spelling is `__attribute__((visibility("default")))` so the
 * symbol survives `-fvisibility=hidden`. Nothing targets non-Windows platforms,
 * but the branch keeps these headers compiling under GCC and Clang for static
 * analysis.
 *
 * `doxide.yml` defines `MO2_API` to the empty string so declarations parse
 * without the decoration. Renaming this macro means renaming it there too.
 */
#ifdef MO2_CORE_SHARED
#ifdef _WIN32
#ifdef MO2_CORE_EXPORTS
#define MO2_API __declspec(dllexport)
#else
#define MO2_API __declspec(dllimport)
#endif
#else
#define MO2_API __attribute__((visibility("default")))
#endif
#else
#define MO2_API
#endif
