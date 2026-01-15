#pragma once

#include "Types.h"
#include "Utils.h"

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
 * ## :material-graph: Ownership Hierarchy
 *
 * ```mermaid
 * classDiagram
 *     FomodInstaller "1" --> "1..*" FomodStep : steps
 *     FomodInstaller "1" --> "0..*" FomodFileEntry : required_files
 *     FomodInstaller "1" --> "0..*" FomodConditionalPattern : conditional_patterns
 *     FomodStep "1" --> "1..*" FomodGroup : groups
 *     FomodStep "1" --> "0..1" FomodCondition : visible
 *     FomodGroup "1" --> "1..*" FomodPlugin : plugins
 *     FomodPlugin "1" --> "0..*" FomodFileEntry : files
 *     FomodPlugin "1" --> "0..*" FomodCondition : dependencies
 *     FomodConditionalPattern "1" --> "1" FomodCondition : condition
 *     FomodConditionalPattern "1" --> "1..*" FomodFileEntry : files
 * ```
 *
 * ## :material-gate-and: Condition Model
 *
 * Conditions form a recursive tree: leaf nodes test a single predicate (flag
 * value, file state, game version, etc.) while composite nodes combine children
 * with `And` / `Or` operators. The same `FomodCondition` struct is reused for
 * module dependencies, step visibility, plugin dependencies, type patterns, and
 * conditional install patterns.
 */

/** @brief Logical operator for combining child conditions. */
enum class FomodConditionOp
{
    And, /**< All child conditions must be true (logical conjunction) */
    Or   /**< At least one child condition must be true (logical disjunction) */
};

/** @brief Discriminator for the predicate a leaf condition tests. */
enum class FomodConditionType
{
    Flag,     /**< `<flagDependency>` -- tests a user-set condition flag */
    File,     /**< `<fileDependency>` -- tests whether a file is Active/Inactive/Missing */
    Game,     /**< `<gameDependency>` -- tests the game version */
    Plugin,   /**< `<pluginDependency>` -- tests whether a game plugin (.esp/.esm) is active */
    Fomod,    /**< `<fomodDependency>` -- tests whether a named FOMOD package is installed */
    Fomm,     /**< `<fommDependency>` -- tests the FOMM version */
    Fose,     /**< `<foseDependency>` -- tests the script extender (FOSE/SKSE/etc.) version */
    Composite /**< `<dependencies>` -- composite node combining children with And/Or */
};

/**
 * @struct FomodCondition
 * @brief Recursive condition tree node - either a leaf predicate or a composite.
 *
 * Leaf nodes use one set of fields depending on `type` (e.g. `flag_name` +
 * `flag_value` for `Flag`). Composite nodes ignore leaf fields and evaluate
 * `children` combined with `op`.
 *
 * Specifically: when `type != Composite`, the leaf fields (`flag_name`,
 * `flag_value`, `file_path`, `version`, `plugin_name`, `fomod_name`) are
 * populated according to the discriminator and `op` / `children` are
 * ignored. When `type == Composite`, only `op` and `children` are
 * meaningful and the leaf fields are ignored.
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
    std::string source;      /**< Archive-relative source path (normalized, with archive prefix) */
    std::string destination; /**< Mod-relative destination path (normalized) */
    int priority = 0;        /**< Overwrite priority; higher values win conflicts (default 0) */
    bool is_folder = false;  /**< True if this entry came from a `<folder>` element */
    bool always_install = false;    /**< XML `alwaysInstall` attribute */
    bool install_if_usable = false; /**< XML `installIfUsable` attribute */
};

/** @brief A condition that, when met, overrides a plugin's declared type. */
struct FomodTypePattern
{
    FomodCondition condition; /**< Condition that triggers this override */
    PluginType result_type =
        PluginType::Optional; /**< Plugin type to apply when condition is met */
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
    std::string name; /**< Display name from the `name` attribute */
    PluginType type =
        PluginType::Optional; /**< Base selection type (may be overridden by type_patterns) */
    std::vector<FomodTypePattern>
        type_patterns; /**< Conditional type overrides from `<dependencyType>/<patterns>` */
    std::vector<FomodFileEntry> files; /**< Files installed when this plugin is selected */
    std::vector<std::pair<std::string, std::string>>
        condition_flags; /**< Flag name/value pairs set when selected */
    std::optional<FomodCondition>
        dependencies; /**< Optional prerequisite conditions for this plugin */
};

/**
 * @enum FomodGroupType
 * @brief Selection cardinality constraint for a group of plugins.
 */
enum class FomodGroupType
{
    SelectExactlyOne, /**< User must select exactly one plugin in the group */
    SelectAtMostOne,  /**< User may select zero or one plugin (radio + none) */
    SelectAtLeastOne, /**< User must select one or more plugins (at least one required) */
    SelectAll,        /**< All plugins are force-selected; user cannot deselect any */
    SelectAny         /**< User may select any combination, including none (checkboxes) */
};

/** FomodConditionOp enum map specialization. */
template <>
inline constexpr auto enum_map<FomodConditionOp> = EnumStringMap<FomodConditionOp, 2>{
    std::array<std::pair<FomodConditionOp, std::string_view>, 2>{{
        {FomodConditionOp::And, "And"},
        {FomodConditionOp::Or, "Or"},
    }},
    FomodConditionOp::And,
};

/** FomodGroupType enum map specialization. */
template <>
inline constexpr auto enum_map<FomodGroupType> = EnumStringMap<FomodGroupType, 5>{
    std::array<std::pair<FomodGroupType, std::string_view>, 5>{{
        {FomodGroupType::SelectExactlyOne, "SelectExactlyOne"},
        {FomodGroupType::SelectAtMostOne, "SelectAtMostOne"},
        {FomodGroupType::SelectAtLeastOne, "SelectAtLeastOne"},
        {FomodGroupType::SelectAll, "SelectAll"},
        {FomodGroupType::SelectAny, "SelectAny"},
    }},
    FomodGroupType::SelectAny,
};

/** @brief A named group of plugins sharing a selection cardinality constraint. */
struct FomodGroup
{
    std::string name; /**< Display name from the `name` attribute */
    FomodGroupType type =
        FomodGroupType::SelectAny;    /**< Selection cardinality constraint for this group */
    std::vector<FomodPlugin> plugins; /**< Selectable options within this group */
};

/** @brief One wizard page presented to the user, optionally gated by a visibility condition. */
struct FomodStep
{
    std::string name;                      /**< Display name from the `name` attribute */
    int ordinal = 0;                       /**< Zero-based position in the wizard sequence */
    std::optional<FomodCondition> visible; /**< Visibility condition; step is shown only when met */
    std::vector<FomodGroup> groups;        /**< Option groups presented on this wizard page */
};

/** @brief Files installed when a condition is met, from `<conditionalFileInstalls>`. */
struct FomodConditionalPattern
{
    FomodCondition condition;          /**< Condition evaluated after all wizard steps complete */
    std::vector<FomodFileEntry> files; /**< Files installed when the condition is met */
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
    std::optional<FomodCondition>
        module_dependencies; /**< Global prerequisites from `<moduleDependencies>` */
    std::vector<FomodFileEntry>
        required_files; /**< Unconditionally installed files from `<requiredInstallFiles>` */
    std::vector<FomodStep> steps; /**< Ordered wizard pages from `<installSteps>` */
    std::vector<FomodConditionalPattern>
        conditional_patterns; /**< Post-wizard conditional installs from `<conditionalFileInstalls>`
                               */
};

/**
 * @brief Count the total number of plugins across all steps and groups.
 * @param installer The FOMOD installer IR to inspect.
 * @return Total plugin count, suitable for sizing flat index arrays.
 */
inline int total_flat_plugins(const FomodInstaller& installer)
{
    int total = 0;
    for (const auto& step : installer.steps)
        for (const auto& group : step.groups)
            total += static_cast<int>(group.plugins.size());
    return total;
}

/**
 * @brief Build a [step][group] -> flat plugin start index map.
 * @param installer The FOMOD installer IR to inspect.
 * @return 2D vector where `result[step_idx][group_idx]` is the flat index of that group's first
 * plugin.
 */
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
