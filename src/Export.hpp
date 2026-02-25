#pragma once

/**
 * @brief DLL export/import macro.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * When building the `mo2-core` shared library (`MO2_CORE_EXPORTS`
 * defined by CMake), `MO2_API` expands to `__declspec(dllexport)`.
 * Consumer targets see `__declspec(dllimport)` instead, so the
 * linker resolves symbols from the DLL at load time.
 *
 * On non-Windows targets, `MO2_API` resolves to
 * `__attribute__((visibility("default")))` so the symbol is exported
 * even when the build uses `-fvisibility=hidden`. The project does
 * not currently target non-Windows platforms, but the macro is
 * defined for portability so headers compile cleanly under GCC/Clang
 * for static analysis.
 */
#ifdef _WIN32
#ifdef MO2_CORE_EXPORTS
#define MO2_API __declspec(dllexport)
#else
#define MO2_API __declspec(dllimport)
#endif
#else
#define MO2_API __attribute__((visibility("default")))
#endif
