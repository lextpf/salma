#pragma once

#include <nlohmann/json.hpp>
#include <pugixml.hpp>
#include <string>
#include <unordered_map>
#include <vector>
#include "Types.h"

namespace mo2core
{

/**
 * @class FomodService
 * @brief Core FOMOD installation logic and plugin flag evaluation.
 * @author Alex (https://github.com/lextpf)
 * @ingroup FomodService
 *
 * Interprets a parsed FOMOD `ModuleConfig.xml` document together with
 * a JSON configuration describing the user's (or inferred) selections,
 * and produces a sequence of FileOperation entries that realize the
 * install.
 *
 * ## :material-list-status: Processing Pipeline
 *
 * InstallationService calls the methods below in order:
 *
 * 1. **check_module_dependencies** -- gate the entire installer on
 *    top-level `<moduleDependencies>`.
 * 2. **validate_json_selections** -- check that group-type constraints
 *    are satisfied (e.g. SelectExactlyOne has exactly one selection).
 * 3. **process_required_files** -- enqueue `<requiredInstallFiles>`.
 * 4. **process_optional_files** -- walk each selected plugin, extract
 *    flags, and enqueue its `<files>` / `<folders>` entries. Also
 *    auto-installs Required plugins per step and alwaysInstall /
 *    installIfUsable files from unselected plugins (second pass).
 * 5. **process_conditional_files** -- evaluate `<conditionalFileInstalls>`
 *    patterns against the collected flags.
 * 6. **execute_file_operations** -- sort by priority and copy.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * flowchart LR
 *     classDef step fill:#1e3a5f,stroke:#3b82f6,color:#e2e8f0
 *     classDef check fill:#134e3a,stroke:#10b981,color:#e2e8f0
 *     classDef out fill:transparent,stroke:#94a3b8,color:#e2e8f0,stroke-dasharray:6 4
 *
 *     A[moduleDependencies]:::check
 *     B[validate JSON]:::check
 *     C[requiredInstallFiles]:::step
 *     D[optional files + flags]:::step
 *     E[conditionalFileInstalls]:::step
 *     F[execute in priority order]:::out
 *
 *     A --> B --> C --> D --> E --> F
 * ```
 *
 * ## :material-flag: Plugin Flags
 *
 * FOMOD plugins can set named flags (`<conditionFlags>`) during
 * installation. These flags are collected in an internal map and
 * later evaluated by `<conditionalFileInstalls>` and step-visibility
 * conditions via FomodDependencyEvaluator.
 *
 * ## :material-priority-high: Priority and Document Order
 *
 * Each file operation carries a `priority` (from the XML attribute,
 * default 0) and a `document_order` tiebreaker assigned in the order
 * nodes are encountered. When execute_file_operations() sorts the
 * queue, later operations at the same priority win - matching MO2's
 * behavior.
 *
 * ## :material-code-tags: Usage Example
 *
 * ```cpp
 * FomodService fomod;
 * fomod.check_module_dependencies(doc, &ctx);
 * fomod.validate_json_selections(doc, config);
 * fomod.process_required_files(doc, src, dst, &ctx);
 * fomod.process_optional_files(doc, config, src, dst, &ctx);
 * fomod.process_conditional_files(doc, src, dst, &ctx);
 * fomod.execute_file_operations();
 * ```
 *
 * ## :material-json: JSON Selection Resilience
 *
 * Most missing JSON fields are handled gracefully:
 * - A missing `steps` array means no optional files are processed.
 * - Missing group names within a step are skipped with a log warning.
 * - If a plugin name from JSON does not match any XML node after
 *   the cascading XPath lookup, the selection is logged and skipped.
 * - **Caveat:** plugin entries in the `plugins` array are read via
 *   `get<std::string>()`, which throws `nlohmann::json::type_error`
 *   if an entry is not a string (e.g. an object or number). Callers
 *   must ensure plugin arrays contain only string values.
 *
 * ## :material-help: Thread Safety
 *
 * Instances are **not** thread-safe. Each installation should use
 * its own FomodService.
 *
 * @see FomodDependencyEvaluator, InstallationService, FileOperations
 */
class FomodService
{
public:
    /**
     * @brief Evaluate top-level `<moduleDependencies>`.
     *
     * @param doc Parsed ModuleConfig.xml.
     * @param context Dependency context (installed files, game version, etc.).
     * @return `true` if dependencies are met or absent.
     */
    bool check_module_dependencies(const pugi::xml_document& doc,
                                   const FomodDependencyContext* context);

    /**
     * @brief Enqueue files from `<requiredInstallFiles>`.
     *
     * These are always installed regardless of user selections.
     *
     * @param doc Parsed ModuleConfig.xml.
     * @param src_base Source base path (extracted archive root).
     * @param dst_base Destination mod directory.
     * @param context Dependency context.
     */
    void process_required_files(const pugi::xml_document& doc,
                                const std::string& src_base,
                                const std::string& dst_base,
                                const FomodDependencyContext* context);

    /**
     * @brief Validate JSON selections against the XML schema.
     *
     * Checks that group-type constraints (SelectExactlyOne,
     * SelectAtMostOne, etc.) are satisfied for each group containing
     * selected plugins. Violations are logged as warnings but do
     * **not** throw - installation proceeds regardless.
     *
     * Skipped entirely if the JSON has no `steps` array.
     *
     * @param doc Parsed ModuleConfig.xml.
     * @param config_json User or inferred selections.
     */
    bool validate_json_selections(const pugi::xml_document& doc, const nlohmann::json& config_json);

    /**
     * @brief Process selected plugins: extract flags and enqueue file operations.
     *
     * Walks the JSON config's step -> group -> plugin selections,
     * locates each plugin in the XML using a cascading XPath strategy
     * (full path -> step+plugin -> group+plugin -> plugin-only ->
     * case-insensitive value match), collects `<conditionFlags>`,
     * and enqueues file/folder operations with their priorities.
     *
     * Also auto-installs Required-type plugins not explicitly listed
     * in the JSON. After processing all steps, a second pass enqueues
     * files with `alwaysInstall="true"` or `installIfUsable="true"`
     * attributes from unselected plugins (skipping NotUsable plugins
     * for installIfUsable).
     *
     * @param doc Parsed ModuleConfig.xml.
     * @param config_json User or inferred selections.
     * @param src_base Source base path (extracted archive root).
     * @param dst_base Destination mod directory.
     * @param context Dependency context.
     */
    void process_optional_files(const pugi::xml_document& doc,
                                const nlohmann::json& config_json,
                                const std::string& src_base,
                                const std::string& dst_base,
                                const FomodDependencyContext* context);

    /**
     * @brief Evaluate `<conditionalFileInstalls>` patterns.
     *
     * Iterates `<pattern>` nodes, evaluates their `<dependencies>`
     * against the current flag state, and enqueues matching file
     * operations.
     *
     * @param doc Parsed ModuleConfig.xml.
     * @param src_base Source base path.
     * @param dst_base Destination mod directory.
     * @param context Dependency context.
     */
    void process_conditional_files(const pugi::xml_document& doc,
                                   const std::string& src_base,
                                   const std::string& dst_base,
                                   const FomodDependencyContext* context);

    /**
     * @brief Sort queued operations by priority and execute all copies.
     *
     * Uses stable sort on its own `file_operations_` vector (ascending
     * priority, then document order), then copies each entry via
     * FileOperations::copy_file() / copy_folder().
     */
    void execute_file_operations();

private:
    void extract_flags(const pugi::xml_node& plugin_node);
    void process_files_node(const pugi::xml_node& files_node,
                            const std::string& src_base,
                            const std::string& dst_base);
    void process_plugin_node(const pugi::xml_node& plugin_node,
                             const std::string& src_base,
                             const std::string& dst_base,
                             const FomodDependencyContext* context);
    int get_priority(const pugi::xml_node& node);
    std::string normalize_string(const std::string& value);
    bool validate_group_selection(const pugi::xml_node& group_node,
                                  const std::unordered_set<std::string>& selected_plugins);
    PluginType evaluate_plugin_type(const pugi::xml_node& plugin_node,
                                    const FomodDependencyContext* context);
    void auto_install_required_for_step(const pugi::xml_document& doc,
                                        const std::string& step_name,
                                        std::unordered_set<std::string>& processed_keys,
                                        const std::string& src_base,
                                        const std::string& dst_base,
                                        const FomodDependencyContext* context);
    void process_auto_install_plugins(const pugi::xml_document& doc,
                                      const std::unordered_set<std::string>& processed_keys,
                                      const std::string& src_base,
                                      const std::string& dst_base,
                                      const FomodDependencyContext* context);
    void process_files_node_filtered(const pugi::xml_node& files_node,
                                     const std::string& src_base,
                                     const std::string& dst_base,
                                     PluginType plugin_type);
    std::string make_plugin_key(const std::string& step,
                                const std::string& group,
                                const std::string& plugin);

    std::unordered_map<std::string, std::string>
        plugin_flags_;  ///< Accumulated condition flags from selected plugins
    std::vector<FileOperation> file_operations_;  ///< Queued file operations awaiting execute
    int next_document_order_ = 0;  ///< Monotonic counter for XML document order tiebreaker
};

}  // namespace mo2core
