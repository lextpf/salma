#pragma once

#include <nlohmann/json.hpp>
#include <string>
#include <unordered_map>
#include <vector>
#include "FomodIR.h"
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
 * std::vector<FileOperation> ops;
 * int next_doc_order = 0;
 *
 * fomod.check_module_dependencies(doc, &ctx);
 * fomod.validate_json_selections(doc, config);
 * fomod.process_required_files(doc, src, dst, &ctx, ops, next_doc_order);
 * fomod.process_optional_files(doc, config, src, dst, &ctx, ops, next_doc_order);
 * fomod.process_conditional_files(doc, src, dst, &ctx, ops, next_doc_order);
 * FomodService::execute_file_operations(ops);
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
     * @brief Set the parsed FOMOD IR for this installation.
     *
     * Must be called before any other method. The caller is responsible
     * for parsing the XML via FomodIRParser::parse() and passing the result.
     *
     * @param installer Parsed FOMOD IR.
     * @throw Does not throw.
     */
    void set_installer(FomodInstaller installer);

    /**
     * @brief Evaluate top-level `<moduleDependencies>`.
     *
     * @param context Dependency context (installed files, game version, etc.).
     * @return `true` if dependencies are met or absent.
     * @pre set_installer() has been called.
     * @throw Does not throw. Dependency evaluation is non-throwing.
     */
    bool check_module_dependencies(const FomodDependencyContext* context);

    /**
     * @brief Enqueue files from `<requiredInstallFiles>`.
     *
     * These are always installed regardless of user selections.
     *
     * @param src_base Source base path (extracted archive root).
     * @param dst_base Destination mod directory.
     * @param ops Accumulated file operations vector (caller-owned).
     * @param next_doc_order Monotonic counter for XML document order tiebreaker (caller-owned).
     * @pre set_installer() has been called.
     * @throw Does not throw. Unsafe destinations are logged and skipped.
     */
    void process_required_files(const std::string& src_base,
                                const std::string& dst_base,
                                std::vector<FileOperation>& ops,
                                int& next_doc_order);

    /**
     * @brief Validate JSON selections against the IR schema.
     *
     * Checks that group-type constraints (SelectExactlyOne,
     * SelectAtMostOne, etc.) are satisfied for each group containing
     * selected plugins. Violations are logged as warnings but do
     * **not** throw.
     *
     * Skipped entirely if the JSON has no `steps` array.
     *
     * @return `true` if all groups pass validation, `false` if any
     *         constraint is violated.
     *
     * @param config_json User or inferred selections.
     * @pre set_installer() has been called.
     * @throw Does not throw. Malformed JSON structures are handled gracefully.
     */
    bool validate_json_selections(const nlohmann::json& config_json);

    /**
     * @brief Process selected plugins: extract flags and enqueue file operations.
     *
     * Walks the JSON config's step -> group -> plugin selections,
     * matches against the IR, collects condition flags, and enqueues
     * file/folder operations with their priorities.
     *
     * Also auto-installs Required-type plugins not explicitly listed
     * in the JSON. After processing all steps, a second pass enqueues
     * files with `alwaysInstall="true"` or `installIfUsable="true"`
     * attributes from unselected plugins (skipping NotUsable plugins
     * for installIfUsable).
     *
     * @param config_json User or inferred selections.
     * @param src_base Source base path (extracted archive root).
     * @param dst_base Destination mod directory.
     * @param context Dependency context.
     * @param ops Accumulated file operations vector (caller-owned).
     * @param next_doc_order Monotonic counter for XML document order tiebreaker (caller-owned).
     * @pre set_installer() has been called.
     * @pre process_required_files() should be called first to establish correct
     *      document_order sequencing.
     * @throw std::exception (re-thrown) if JSON traversal encounters a type error
     *        (e.g. a non-string entry in a plugins array). On exception, all
     *        operations enqueued by this call are rolled back from @p ops and
     *        @p next_doc_order is restored to its original value, so a retry
     *        with the same vector after fixing the JSON is idempotent.
     */
    void process_optional_files(const nlohmann::json& config_json,
                                const std::string& src_base,
                                const std::string& dst_base,
                                const FomodDependencyContext* context,
                                std::vector<FileOperation>& ops,
                                int& next_doc_order);

    /**
     * @brief Evaluate `<conditionalFileInstalls>` patterns from the IR.
     *
     * Iterates conditional patterns, evaluates their conditions
     * against the current flag state, and enqueues matching file
     * operations.
     *
     * @param src_base Source base path.
     * @param dst_base Destination mod directory.
     * @param context Dependency context.
     * @param ops Accumulated file operations vector (caller-owned).
     * @param next_doc_order Monotonic counter for XML document order tiebreaker (caller-owned).
     * @pre set_installer() has been called.
     * @pre process_optional_files() should be called first so that plugin flags
     *      are populated for condition evaluation.
     * @throw Does not throw. Condition evaluation and file enqueuing are non-throwing.
     */
    void process_conditional_files(const std::string& src_base,
                                   const std::string& dst_base,
                                   const FomodDependencyContext* context,
                                   std::vector<FileOperation>& ops,
                                   int& next_doc_order);

    /**
     * @brief Sort queued operations by priority and execute all copies.
     *
     * Uses stable sort on the provided operations vector (ascending
     * priority, then document order), then copies each entry via
     * FileOperations::copy_file() / copy_folder().
     *
     * @param ops File operations to sort and execute (cleared after execution).
     * @return Number of failed file operations (0 = all succeeded).
     * @post @p ops is cleared after execution regardless of individual failures.
     * @throw Does not throw. Individual copy failures are caught, counted, and
     *        logged; the return value indicates how many operations failed.
     */
    static int execute_file_operations(std::vector<FileOperation>& ops);

private:
    /** Convert an IR file entry into a FileOperation with absolute paths. */
    void enqueue_entry(const FomodFileEntry& entry,
                       const std::string& src_base,
                       const std::string& dst_base,
                       std::vector<FileOperation>& ops,
                       int& next_doc_order);

    /** Enqueue all file entries from a plugin. */
    void enqueue_plugin_files(const FomodPlugin& plugin,
                              const std::string& src_base,
                              const std::string& dst_base,
                              std::vector<FileOperation>& ops,
                              int& next_doc_order);

    /** Build a composite key for plugin dedup tracking (step\x1fgroup\x1fplugin, lowercased). */
    static std::string make_plugin_key(const std::string& step,
                                       const std::string& group,
                                       const std::string& plugin);

    FomodInstaller installer_; /**< Parsed FOMOD IR (set by parse_ir) */
    std::unordered_map<std::string, std::string>
        plugin_flags_; /**< Accumulated condition flags from selected plugins */
};

}  // namespace mo2core
