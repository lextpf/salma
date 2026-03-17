#pragma once

#include "Export.h"
#include "FomodIR.h"
#include "Types.h"

#include <cstdint>
#include <pugixml.hpp>
#include <string>
#include <unordered_map>

namespace mo2core
{

/**
 * @class FomodDependencyEvaluator
 * @brief Evaluates FOMOD dependency conditions.
 * @author Alex (https://github.com/lextpf)
 * @ingroup FomodDependencyEvaluator
 *
 * Recursively evaluates `<dependencies>` trees from FOMOD XML.
 * These appear in three places: `<moduleDependencies>` (top-level
 * gate), `<visible>` (step visibility), and `<conditionalFileInstalls>`
 * pattern matching.
 *
 * ## :material-file-tree-outline: Dependency Types
 *
 * |      Type |     XML element      | Checks against                         |
 * |-----------|----------------------|----------------------------------------|
 * |      Flag |  `<flagDependency>`  | Plugin flags set during install        |
 * |      File |  `<fileDependency>`  | Files in mod dir, archive, or game dir |
 * |      Game |  `<gameDependency>`  | Game version string                    |
 * |    Plugin | `<pluginDependency>` | Installed plugins (.esp/.esm)          |
 * |     FOMOD | `<fomodDependency>`  | Previously installed FOMODs            |
 * |      FOMM |  `<fommDependency>`  | FOMM version (hardcoded 0.13.21)       |
 * | FOSE/SKSE |  `<foseDependency>`  | Script extender version                |
 *
 * Dependency nodes can be combined with `operator="And"` (default)
 * or `operator="Or"`. Nested `<dependencies>` nodes are evaluated
 * recursively, supporting arbitrary boolean trees.
 *
 * ## :material-compare: Version Comparison
 *
 * Version strings are split on `.` and compared component-wise as
 * integers (non-numeric characters stripped). The internal
 * `compare_versions()` helper supports `Equal`, `GreaterThan`,
 * `LessThan`, `GreaterThanOrEqual`, and `LessThanOrEqual`.
 *
 * **Current per-type behavior** (version operators are not read from XML):
 * - **gameDependency**: hardcodes `GreaterThanOrEqual` (actual >= required),
 *   matching MO2's semantics.
 * - **fommDependency**: hardcodes `<=` (required <= hardcoded "0.13.21").
 * - **foseDependency**: always returns met (no version comparison).
 * - **fileDependency**: checks file existence and evaluates the `state`
 *   attribute (`Active`/`Inactive`/`Missing`). For `Inactive`, plugin
 *   files (.esp/.esm/.esl) are checked against `installed_plugins`.
 * - **pluginDependency**: supports `type` attribute (`Active`/`Inactive`).
 *   Checks both presence in `installed_plugins` and file existence on disk.
 *
 * ## :material-information-outline: Standalone Mode
 *
 * When no FomodDependencyContext is provided (or its fields are
 * empty), game and FOSE/SKSE dependencies default to **met**,
 * allowing the installer to run without a live game environment.
 * Plugin and FOMOD dependencies return **not met** without context
 * (they require the installed-plugins/installed-FOMODs sets).
 * Flag and file dependencies are always evaluated strictly.
 *
 * ## :material-code-tags: Usage Example
 *
 * ```cpp
 * std::unordered_map<std::string, std::string> flags;
 * FomodDependencyEvaluator eval(flags, &context);
 * if (eval.are_dependencies_met(xml_node)) {
 *     // condition satisfied
 * }
 * ```
 *
 * ## :material-alert-circle-outline: Unknown Node Types
 *
 * Unrecognized child elements inside a `<dependencies>` node
 * default to **met** (`true`). Under `operator="And"` this is
 * harmless (a `true` value does not change the conjunction).
 * Under `operator="Or"`, however, a single unknown node causes
 * the entire dependency block to evaluate as met -- effectively
 * short-circuiting the check. This means a malformed or future
 * XML element will not block installation, but callers should be
 * aware that unknown nodes are not neutral in Or contexts.
 *
 * ## :material-format-letter-case-lower: Normalization Expectations
 *
 * - `installed_files` in the context should contain normalized paths
 *   (lowercase, forward-slash separators) for reliable matching.
 * - `installed_plugins` should contain lowercased `.esp`/`.esm`
 *   filenames (no directory prefix).
 *
 * ## :material-help: Thread Safety
 *
 * Instances are **not** thread-safe. The evaluator holds a mutable
 * reference to the plugin flags map.
 *
 * @see FomodService
 */
class FomodDependencyEvaluator
{
public:
    /**
     * @brief Construct an evaluator with access to plugin flags and dependency context.
     *
     * @param plugin_flags Mutable reference to the flag map maintained
     *        by FomodService. Flag dependencies are evaluated against
     *        this map.
     * @param context Optional dependency context providing installed
     *        files, plugins, game version, etc. May be `nullptr` if
     *        only flag dependencies are needed.
     */
    FomodDependencyEvaluator(std::unordered_map<std::string, std::string>& plugin_flags,
                             const FomodDependencyContext* context = nullptr);

    /**
     * @brief Evaluate a `<dependencies>` node recursively.
     *
     * Supports `And` / `Or` operators and nested `<dependencies>`.
     * Returns `true` if the node is null or empty (no dependencies
     * means always met).
     *
     * @param dependencies_node The XML `<dependencies>` element.
     * @return `true` if all (And) or any (Or) child conditions are met.
     */
    bool are_dependencies_met(const pugi::xml_node& dependencies_node);

private:
    bool evaluate_flag_dependency(const pugi::xml_node& dep_node);
    bool evaluate_file_dependency(const pugi::xml_node& dep_node);
    bool evaluate_game_dependency(const pugi::xml_node& dep_node);
    bool evaluate_plugin_dependency(const pugi::xml_node& dep_node);
    bool evaluate_fomod_dependency(const pugi::xml_node& dep_node);
    bool evaluate_fomm_dependency(const pugi::xml_node& dep_node);
    bool evaluate_fose_dependency(const pugi::xml_node& dep_node);
    bool compare_versions(const std::string& actual,
                          const std::string& required,
                          const std::string& op);

    std::unordered_map<std::string, std::string>&
        plugin_flags_;                       ///< Reference to FomodService's flag map
    const FomodDependencyContext* context_;  ///< External dependency context (may be null)
};

/// External dependency override mode used during inference.
enum class ExternalConditionOverride : uint8_t
{
    Unknown = 0,
    ForceFalse = 1,
    ForceTrue = 2,
};

/// Evaluate a FomodCondition IR node against a flag map and optional context.
/// Same semantics as FomodDependencyEvaluator::are_dependencies_met but
/// operates on the compiled IR instead of raw XML nodes.
MO2_API bool evaluate_condition(const FomodCondition& condition,
                                const std::unordered_map<std::string, std::string>& flags,
                                const FomodDependencyContext* context = nullptr);

/// Evaluate condition for inference: flag conditions evaluated normally,
/// all external conditions (File, Plugin, Fomod) follow override mode.
/// - ForceTrue: external conditions evaluate to true
/// - ForceFalse: external conditions evaluate to false
/// - Unknown: evaluate using standalone/default semantics (no forced value)
MO2_API bool evaluate_condition_inferred(const FomodCondition& condition,
                                         const std::unordered_map<std::string, std::string>& flags,
                                         ExternalConditionOverride external_override,
                                         const FomodDependencyContext* context = nullptr);

/// Determine a plugin's effective type given the current flag state.
/// Checks type_patterns in order; first match wins. Falls back to default type.
MO2_API PluginType evaluate_plugin_type(const FomodPlugin& plugin,
                                        const std::unordered_map<std::string, std::string>& flags,
                                        const FomodDependencyContext* context = nullptr);

}  // namespace mo2core
