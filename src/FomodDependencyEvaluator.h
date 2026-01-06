#pragma once

#include "Export.h"
#include "FomodIR.h"
#include "Types.h"
#include "Utils.h"

#include <cstdint>
#include <string>
#include <unordered_map>

namespace mo2core
{

/** Maximum depth for recursive condition evaluation (guards against malformed XML). */
static constexpr int MAX_DEPENDENCY_DEPTH = 32;

/** External dependency override mode used during inference. */
enum class ExternalConditionOverride : uint8_t
{
    Unknown =
        0,  ///< External state cannot be determined; File/Plugin/Fomod conditions conservatively
            /** < return false, infrastructure conditions (Game/Fomm/Fose) return true. */
    ForceFalse = 1,  ///< Override forces external dependency to evaluate as unmet (false).
    ForceTrue = 2,   ///< Override forces external dependency to evaluate as met (true).
};

// ---------------------------------------------------------------------------
// IR-based condition evaluation (free functions)
// ---------------------------------------------------------------------------
//
// These free functions evaluate pre-compiled FomodCondition IR trees
// and are the single source of truth for FOMOD dependency semantics.
// They share core evaluation helpers (eval_file_dep, eval_game_dep,
// version comparison, etc.) defined in FomodDependencyEvaluator.cpp.
//
// Callers:
//   - FomodService (forward installation)
//   - FomodForwardSimulator (offline simulation)
//   - FomodCSPSolver (constraint solving)
// ---------------------------------------------------------------------------

/**
 * Evaluate a FomodCondition IR node against a flag map and optional context.
 *
 * @throw Does not throw. Filesystem errors during file-dependency checks are
 *        handled internally via non-throwing std::error_code overloads.
 */
MO2_API bool evaluate_condition(const FomodCondition& condition,
                                const std::unordered_map<std::string, std::string>& flags,
                                const FomodDependencyContext* context = nullptr);

/**
 * Evaluate condition for inference: flag conditions evaluated normally,
 * all external conditions (File, Plugin, Fomod) follow override mode.
 * - ForceTrue: external conditions evaluate to true
 * - ForceFalse: external conditions evaluate to false
 * - Unknown: external conditions (File, Plugin, Fomod) conservatively
 *   return false; infrastructure conditions (Game, Fomm, Fose) return true
 *
 * @throw Does not throw.
 */
MO2_API bool evaluate_condition_inferred(const FomodCondition& condition,
                                         const std::unordered_map<std::string, std::string>& flags,
                                         ExternalConditionOverride external_override,
                                         const FomodDependencyContext* context = nullptr);

/**
 * Determine a plugin's effective type given the current flag state.
 * Checks type_patterns in order; first match wins. Falls back to default type.
 *
 * @throw Does not throw. Delegates to evaluate_condition() /
 *        evaluate_condition_inferred(), both of which are non-throwing.
 */
MO2_API PluginType evaluate_plugin_type(const FomodPlugin& plugin,
                                        const std::unordered_map<std::string, std::string>& flags,
                                        const FomodDependencyContext* context = nullptr);

}  // namespace mo2core
