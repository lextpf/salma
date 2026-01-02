#pragma once

#include "Types.h"

#include <optional>
#include <string>
#include <vector>

namespace mo2core
{

/**
 * @class FomodIR
 * @brief Intermediate representation of a parsed FOMOD ModuleConfig.xml.
 * @author Alex (https://github.com/lextpf)
 * @ingroup FomodService
 *
 * This header defines a strongly-typed IR that mirrors the FOMOD XML schema.
 * The XML is parsed once into these structures, which are then consumed by the
 * dependency evaluator, forward simulator, and CSP solver without re-reading XML.
 *
 * ## :material-file-tree: IR Hierarchy
 *
 * | Level            | Struct                   | XML Element                    |
 * |------------------|--------------------------|--------------------------------|
 * | Root             | `FomodInstaller`         | `<config>`                     |
 * | Install step     | `FomodStep`              | `<installStep>`                |
 * | Option group     | `FomodGroup`             | `<group>`                      |
 * | Selectable option| `FomodPlugin`            | `<plugin>`                     |
 * | File mapping     | `FomodFileEntry`         | `<file>` / `<folder>`          |
 * | Condition tree   | `FomodCondition`         | `<dependencies>` / `<pattern>` |
 * | Conditional files| `FomodConditionalPattern` | `<pattern>` in `<conditionalFileInstalls>` |
 *
 * ## :material-gate-and: Condition Model
 *
 * Conditions form a recursive tree: leaf nodes test a single predicate (flag
 * value, file state, game version, etc.) while composite nodes combine children
 * with `And` / `Or` operators. The same `FomodCondition` struct is reused for
 * module dependencies, step visibility, plugin dependencies, type patterns, and
 * conditional install patterns.
 */

/// @brief Logical operator for combining child conditions.
enum class FomodConditionOp
{
    And,
    Or
};

/// @brief Discriminator for the predicate a leaf condition tests.
enum class FomodConditionType
{
    Flag,
    File,
    Game,
    Plugin,
    Fomod,
    Fomm,
    Fose,
    Composite
};

/**
 * @struct FomodCondition
 * @brief Recursive condition tree node - either a leaf predicate or a composite.
 *
 * Leaf nodes use one set of fields depending on `type` (e.g. `flag_name` +
 * `flag_value` for `Flag`). Composite nodes ignore leaf fields and evaluate
 * `children` combined with `op`.
 */
struct FomodCondition
{
    FomodConditionType type = FomodConditionType::Composite;
    FomodConditionOp op = FomodConditionOp::And;

    // Leaf fields (used by the specific type)
    std::string flag_name;
    std::string flag_value;
    std::string file_path;
    std::string file_state;   // Active, Inactive, Missing
    std::string version;      // gameDependency, fommDependency, foseDependency
    std::string plugin_name;  // pluginDependency
    std::string plugin_type;  // Active/Inactive for pluginDependency
    std::string fomod_name;   // fomodDependency

    // Composite children
    std::vector<FomodCondition> children;
};

/**
 * @struct FomodFileEntry
 * @brief A single source-to-destination file or folder mapping.
 *
 * Represents a `<file>` or `<folder>` element. Folder entries are expanded
 * into individual file atoms during installation planning.
 */
struct FomodFileEntry
{
    std::string source;       // archive-relative path (normalized)
    std::string destination;  // mod-relative path (normalized)
    int priority = 0;
    bool is_folder = false;
    bool always_install = false;
    bool install_if_usable = false;
};

/// @brief A condition that, when met, overrides a plugin's declared type.
struct FomodTypePattern
{
    FomodCondition condition;
    PluginType result_type = PluginType::Optional;
};

/**
 * @struct FomodPlugin
 * @brief A selectable option within a group, carrying files and condition flags.
 *
 * `type` (possibly overridden by `type_patterns`) controls selection
 * constraints. When selected, the plugin's `files` are added to the install
 * plan and its `condition_flags` are set for downstream condition evaluation.
 */
struct FomodPlugin
{
    std::string name;
    PluginType type = PluginType::Optional;
    std::vector<FomodTypePattern> type_patterns;
    std::vector<FomodFileEntry> files;
    std::vector<std::pair<std::string, std::string>> condition_flags;  // name, value
    std::optional<FomodCondition> dependencies;
};

/**
 * @enum FomodGroupType
 * @brief Selection cardinality constraint for a group of plugins.
 */
enum class FomodGroupType
{
    SelectExactlyOne,
    SelectAtMostOne,
    SelectAtLeastOne,
    SelectAll,
    SelectAny
};

/// @brief A named group of plugins sharing a selection cardinality constraint.
struct FomodGroup
{
    std::string name;
    FomodGroupType type = FomodGroupType::SelectAny;
    std::vector<FomodPlugin> plugins;
};

/// @brief One wizard page presented to the user, optionally gated by a visibility condition.
struct FomodStep
{
    std::string name;
    int ordinal = 0;
    std::optional<FomodCondition> visible;
    std::vector<FomodGroup> groups;
};

/// @brief Files installed when a condition is met, from `<conditionalFileInstalls>`.
struct FomodConditionalPattern
{
    FomodCondition condition;
    std::vector<FomodFileEntry> files;
};

/**
 * @struct FomodInstaller
 * @brief Top-level IR for a complete FOMOD installer definition.
 *
 * Holds the full parsed content of a ModuleConfig.xml: module-level
 * dependencies, unconditionally required files, the ordered wizard steps,
 * and any conditional install patterns evaluated after all steps complete.
 */
struct FomodInstaller
{
    std::optional<FomodCondition> module_dependencies;
    std::vector<FomodFileEntry> required_files;
    std::vector<FomodStep> steps;
    std::vector<FomodConditionalPattern> conditional_patterns;
};

/// Total number of plugins across all steps and groups.
inline int total_flat_plugins(const FomodInstaller& installer)
{
    int total = 0;
    for (const auto& step : installer.steps)
        for (const auto& group : step.groups)
            total += static_cast<int>(group.plugins.size());
    return total;
}

/// Build a [step][group] -> flat plugin start index map.
inline std::vector<std::vector<int>> compute_flat_starts(const FomodInstaller& installer)
{
    std::vector<std::vector<int>> flat_starts(installer.steps.size());
    int flat = 0;
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        flat_starts[si].resize(installer.steps[si].groups.size());
        for (size_t gi = 0; gi < installer.steps[si].groups.size(); ++gi)
        {
            flat_starts[si][gi] = flat;
            flat += static_cast<int>(installer.steps[si].groups[gi].plugins.size());
        }
    }
    return flat_starts;
}

}  // namespace mo2core
