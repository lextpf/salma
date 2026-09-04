#pragma once

/**
 * @brief controls public symbol visibility for shared support builds.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * `MO2_API` is empty for the repository's static build. shared builds must define
 * `MO2_CORE_SHARED` for the library and every consumer. the library also defines
 * `MO2_CORE_EXPORTS` while it builds.
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
