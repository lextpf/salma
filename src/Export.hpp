#pragma once

/**
 * @brief DLL export/import macro.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * The C++ no longer produces a shared library. The engine DLL
 * (`mo2-salma.dll`) is built from the Rust crate at the repo root, and what
 * remains here (`salma-support`: Utils, Logger, SecurityContext) is a STATIC
 * library linked directly into `mo2-server` and `salma_tests`. Static linkage
 * needs no decoration, so `MO2_API` expands to nothing by default.
 *
 * The shared-library spellings are kept behind `MO2_CORE_SHARED` so these
 * headers still compile if a DLL target is ever reintroduced: define
 * `MO2_CORE_SHARED` in every consumer, plus `MO2_CORE_EXPORTS` in the library
 * target itself.
 *
 * On non-Windows shared builds, `MO2_API` resolves to
 * `__attribute__((visibility("default")))` so the symbol survives
 * `-fvisibility=hidden`. The project does not currently target non-Windows
 * platforms, but the macro is defined for portability so headers compile
 * cleanly under GCC/Clang for static analysis.
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
