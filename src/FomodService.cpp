#include "FomodService.h"
#include <algorithm>
#include <format>
#include "FileOperations.h"
#include "FomodDependencyEvaluator.h"
#include "Logger.h"
#include "Utils.h"

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2core
{

namespace
{

// Schema-tolerant plugin name reader. Supports the legacy schema-v1 format
// where `plugins` is `[string, ...]` and the schema-v2 format where it is
// `[{"name": "...", ...}, ...]`. Returns the empty string when the entry
// cannot be parsed (caller should skip).
std::string read_plugin_name(const json& entry)
{
    if (entry.is_string())
    {
        return entry.get<std::string>();
    }
    if (entry.is_object() && entry.contains("name") && entry["name"].is_string())
    {
        return entry["name"].get<std::string>();
    }
    return {};
}

}  // namespace

// ---------------------------------------------------------------------------
// set_installer
// ---------------------------------------------------------------------------

void FomodService::set_installer(FomodInstaller installer)
{
    installer_ = std::move(installer);
}

// ---------------------------------------------------------------------------
// enqueue_entry: convert a single IR FomodFileEntry into a FileOperation
// ---------------------------------------------------------------------------

void FomodService::enqueue_entry(const FomodFileEntry& entry,
                                 const std::string& src_base,
                                 const std::string& dst_base,
                                 std::vector<FileOperation>& ops,
                                 int& next_doc_order)
{
    if (entry.source.empty())
        return;

    if (!is_safe_destination(entry.destination))
    {
        Logger::instance().log_warning(
            std::format("[fomod] Skipping path-traversal destination: {}", entry.destination));
        return;
    }

    auto src_path = (fs::path(src_base) / entry.source).string();
    auto dst_path = (fs::path(dst_base) / entry.destination).string();
    auto type = entry.is_folder ? FileOpType::Folder : FileOpType::File;
    ops.push_back({type, src_path, dst_path, entry.priority, next_doc_order++});
}

// ---------------------------------------------------------------------------
// enqueue_plugin_files: enqueue all file entries from a plugin
// ---------------------------------------------------------------------------

void FomodService::enqueue_plugin_files(const FomodPlugin& plugin,
                                        const std::string& src_base,
                                        const std::string& dst_base,
                                        std::vector<FileOperation>& ops,
                                        int& next_doc_order)
{
    for (const auto& entry : plugin.files)
    {
        enqueue_entry(entry, src_base, dst_base, ops, next_doc_order);
    }
}

// ---------------------------------------------------------------------------
// make_plugin_key
// ---------------------------------------------------------------------------

// Build a unique, case-insensitive lookup key for a plugin by its step/group/plugin names.
std::string FomodService::make_plugin_key(const std::string& step,
                                          const std::string& group,
                                          const std::string& plugin)
{
    // Use ASCII Unit Separator (0x1F) which cannot appear in XML text content,
    // eliminating the risk of key collisions from names containing the separator.
    return to_lower(step) + "\x1f" + to_lower(group) + "\x1f" + to_lower(plugin);
}

// ---------------------------------------------------------------------------
// check_module_dependencies  (Phase 1: IR-based)
// ---------------------------------------------------------------------------

bool FomodService::check_module_dependencies(const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    if (!installer_.module_dependencies)
    {
        logger.log("[fomod] No module-level dependencies found");
        return true;
    }

    logger.log("[fomod] Checking module-level dependencies...");
    bool met = evaluate_condition(*installer_.module_dependencies, plugin_flags_, context);

    if (!met)
    {
        logger.log(
            "[fomod] ERROR: Module-level dependencies not met - installation cannot proceed");
        return false;
    }

    logger.log("[fomod] Module-level dependencies satisfied");
    return true;
}

// ---------------------------------------------------------------------------
// process_required_files  (Phase 2: IR-based)
// ---------------------------------------------------------------------------

void FomodService::process_required_files(const std::string& src_base,
                                          const std::string& dst_base,
                                          std::vector<FileOperation>& ops,
                                          int& next_doc_order)
{
    auto& logger = Logger::instance();
    if (installer_.required_files.empty())
    {
        logger.log("[fomod] No required install files found");
        return;
    }

    logger.log(std::format("[fomod] Processing {} required install files...",
                           installer_.required_files.size()));

    int file_count = 0, folder_count = 0;
    for (const auto& entry : installer_.required_files)
    {
        int before = static_cast<int>(ops.size());
        enqueue_entry(entry, src_base, dst_base, ops, next_doc_order);
        if (static_cast<int>(ops.size()) > before)
        {
            if (entry.is_folder)
                folder_count++;
            else
                file_count++;
        }
    }

    logger.log(std::format("[fomod] Queued {} files and {} folders from required install files",
                           file_count,
                           folder_count));
}

// ---------------------------------------------------------------------------
// validate_json_selections  (Phase 3: IR-based)
// ---------------------------------------------------------------------------

// Check whether the number of selected plugins satisfies the group's cardinality constraint.
static bool validate_cardinality(FomodGroupType type, int selected, int total)
{
    switch (type)
    {
        case FomodGroupType::SelectExactlyOne:
            return selected == 1;
        case FomodGroupType::SelectAtLeastOne:
            return selected >= 1;
        case FomodGroupType::SelectAtMostOne:
            return selected <= 1;
        case FomodGroupType::SelectAll:
            return selected == total;
        case FomodGroupType::SelectAny:
            return true;
    }
    return true;
}

bool FomodService::validate_json_selections(const json& config_json)
{
    auto& logger = Logger::instance();
    if (!config_json.contains("steps") || !config_json["steps"].is_array())
    {
        logger.log("[fomod] No steps in JSON - validation skipped");
        return true;
    }

    logger.log("[fomod] Validating JSON selections against FOMOD schema...");
    bool all_valid = true;

    // Build a map of selected plugin names per step/group from JSON
    std::unordered_map<std::string,
                       std::unordered_map<std::string, std::unordered_set<std::string>>>
        selected_plugins;

    for (const auto& step : config_json["steps"])
    {
        std::string step_name = step.value("name", "");
        if (step.contains("groups") && step["groups"].is_array())
        {
            for (const auto& group : step["groups"])
            {
                std::string group_name = group.value("name", "");
                if (group.contains("plugins") && group["plugins"].is_array())
                {
                    for (const auto& plugin : group["plugins"])
                    {
                        std::string plugin_name = read_plugin_name(plugin);
                        if (plugin_name.empty())
                            continue;
                        selected_plugins[step_name][group_name].insert(plugin_name);
                    }
                }
            }
        }
    }

    // Validate against the IR's step/group structure
    for (const auto& step : installer_.steps)
    {
        auto step_it = selected_plugins.find(step.name);
        if (step_it == selected_plugins.end())
            continue;

        for (const auto& group : step.groups)
        {
            auto group_it = step_it->second.find(group.name);
            if (group_it == step_it->second.end())
                continue;

            const auto& sel = group_it->second;

            // Count how many selected plugins exist in this IR group
            int selected_in_group = 0;
            for (const auto& plugin : group.plugins)
            {
                if (sel.count(plugin.name))
                    selected_in_group++;
            }

            // Warn about selections not found in the IR group
            for (const auto& sel_name : sel)
            {
                bool found = false;
                for (const auto& plugin : group.plugins)
                {
                    if (plugin.name == sel_name)
                    {
                        found = true;
                        break;
                    }
                }
                if (!found)
                {
                    logger.log_warning(std::format(
                        "[fomod] Group \"{}\": selected plugin \"{}\" not found in group",
                        group.name,
                        sel_name));
                }
            }

            auto type_str = enum_to_string(group.type);
            bool valid = validate_cardinality(
                group.type, selected_in_group, static_cast<int>(group.plugins.size()));

            if (!valid)
            {
                all_valid = false;
                logger.log(std::format(
                    "[fomod] WARNING: Group \"{}\" type \"{}\" validation failed: {} selected, {} "
                    "total "
                    "(note: Required plugins are auto-installed and may not appear in JSON "
                    "selections)",
                    group.name,
                    type_str,
                    selected_in_group,
                    group.plugins.size()));
            }
            else
            {
                logger.log(
                    std::format("[fomod] Group \"{}\" type \"{}\": {}/{} plugins selected - VALID",
                                group.name,
                                type_str,
                                selected_in_group,
                                group.plugins.size()));
            }
        }
    }

    logger.log(std::format("[fomod] JSON selections validated: {}",
                           all_valid ? "ALL VALID" : "SOME VIOLATIONS"));
    return all_valid;
}

// ---------------------------------------------------------------------------
// process_optional_files  (Phase 4: IR-based)
// ---------------------------------------------------------------------------

void FomodService::process_optional_files(const json& config_json,
                                          const std::string& src_base,
                                          const std::string& dst_base,
                                          const FomodDependencyContext* context,
                                          std::vector<FileOperation>& ops,
                                          int& next_doc_order)
{
    auto& logger = Logger::instance();
    const auto initial_ops_size = ops.size();
    const int initial_doc_order = next_doc_order;
    bool has_steps = config_json.contains("steps") && config_json["steps"].is_array();
    std::unordered_set<std::string> processed_plugins;

    if (!has_steps)
    {
        logger.log("[fomod] No valid steps in JSON - optional selections skipped");
    }
    else
    {
        try
        {
            // Pass 1: Process explicit JSON selections
            logger.log(std::format("[fomod] Processing optional files from JSON with {} step(s)",
                                   config_json["steps"].size()));

            // Map IR steps by name for occurrence-based matching
            std::unordered_map<std::string, std::vector<size_t>> ir_steps_by_name;
            for (size_t si = 0; si < installer_.steps.size(); ++si)
                ir_steps_by_name[installer_.steps[si].name].push_back(si);

            std::unordered_map<std::string, int> step_occurrence;

            for (const auto& json_step : config_json["steps"])
            {
                std::string step_name = json_step.value("name", "");
                logger.log(std::format("[fomod] Processing step: \"{}\"", step_name));

                if (!json_step.contains("groups") || !json_step["groups"].is_array())
                {
                    logger.log(std::format("[fomod] Step \"{}\" has no groups array", step_name));
                    step_occurrence[step_name]++;
                    continue;
                }

                // Find the correct IR step by name + occurrence
                int occ = step_occurrence[step_name]++;
                const FomodStep* ir_step = nullptr;
                size_t ir_step_idx = 0;
                {
                    auto it = ir_steps_by_name.find(step_name);
                    if (it != ir_steps_by_name.end() && occ < static_cast<int>(it->second.size()))
                    {
                        ir_step_idx = it->second[occ];
                        ir_step = &installer_.steps[ir_step_idx];
                    }
                }

                if (!ir_step)
                {
                    logger.log_warning(std::format(
                        "[fomod] Could not find IR step \"{}\" occurrence {}", step_name, occ));
                    continue;
                }

                // Check step visibility
                if (ir_step->visible &&
                    !evaluate_condition(*ir_step->visible, plugin_flags_, context))
                {
                    logger.log(std::format(
                        "[fomod] Skipping step \"{}\" due to unmet visibility dependencies",
                        step_name));
                    continue;
                }

                // Map IR groups by name for occurrence-based matching
                std::unordered_map<std::string, std::vector<size_t>> ir_groups_by_name;
                for (size_t gi = 0; gi < ir_step->groups.size(); ++gi)
                    ir_groups_by_name[ir_step->groups[gi].name].push_back(gi);

                std::unordered_map<std::string, int> group_occurrence;

                logger.log(std::format("[fomod] Step has {} group(s)", json_step["groups"].size()));

                for (const auto& json_group : json_step["groups"])
                {
                    std::string group_name = json_group.value("name", "");
                    logger.log(std::format("[fomod] Processing group: \"{}\"", group_name));

                    if (!json_group.contains("plugins") || !json_group["plugins"].is_array())
                    {
                        logger.log(
                            std::format("[fomod] Group \"{}\" has no plugins array", group_name));
                        group_occurrence[group_name]++;
                        continue;
                    }

                    // Find the correct IR group by name + occurrence
                    int gocc = group_occurrence[group_name]++;
                    const FomodGroup* ir_group = nullptr;
                    {
                        auto it = ir_groups_by_name.find(group_name);
                        if (it != ir_groups_by_name.end() &&
                            gocc < static_cast<int>(it->second.size()))
                        {
                            ir_group = &ir_step->groups[it->second[gocc]];
                        }
                    }

                    // Build IR plugin list for name matching
                    std::unordered_map<std::string, std::vector<size_t>> ir_plugins_by_name;
                    if (ir_group)
                    {
                        for (size_t pi = 0; pi < ir_group->plugins.size(); ++pi)
                            ir_plugins_by_name[ir_group->plugins[pi].name].push_back(pi);
                    }

                    std::unordered_map<std::string, int> plugin_occurrence;

                    logger.log(std::format("[fomod] Group has {} plugin(s)",
                                           json_group["plugins"].size()));

                    for (const auto& json_plugin : json_group["plugins"])
                    {
                        std::string plugin_name = read_plugin_name(json_plugin);
                        if (plugin_name.empty())
                        {
                            logger.log_warning(
                                "[fomod] Skipping plugin entry with no readable name");
                            continue;
                        }

                        logger.log(std::format(
                            "[fomod] Looking for plugin: \"{}\" in step \"{}\", group \"{}\"",
                            plugin_name,
                            step_name,
                            group_name));

                        // Find plugin by name + occurrence
                        int pocc = plugin_occurrence[plugin_name]++;
                        const FomodPlugin* ir_plugin = nullptr;
                        if (ir_group)
                        {
                            auto it = ir_plugins_by_name.find(plugin_name);
                            if (it != ir_plugins_by_name.end() &&
                                pocc < static_cast<int>(it->second.size()))
                            {
                                ir_plugin = &ir_group->plugins[it->second[pocc]];
                            }
                        }

                        if (ir_plugin)
                        {
                            // Check plugin-level dependencies
                            if (ir_plugin->dependencies &&
                                !evaluate_condition(
                                    *ir_plugin->dependencies, plugin_flags_, context))
                            {
                                logger.log(std::format(
                                    "[fomod] Skipping plugin \"{}\" due to unmet dependencies",
                                    plugin_name));
                                continue;
                            }

                            logger.log(std::format("[fomod] Plugin \"{}\" (explicitly selected)",
                                                   plugin_name));

                            // Accumulate condition flags
                            for (const auto& [fn, fv] : ir_plugin->condition_flags)
                            {
                                if (!fn.empty())
                                    plugin_flags_[fn] = fv;
                            }

                            // Enqueue files
                            enqueue_plugin_files(
                                *ir_plugin, src_base, dst_base, ops, next_doc_order);
                            processed_plugins.insert(
                                make_plugin_key(step_name, group_name, plugin_name));
                        }
                        else
                        {
                            logger.log_error(
                                std::format("[fomod] Could not find plugin \"{}\" in step/group IR",
                                            plugin_name));
                        }
                    }
                }

                // Auto-install Required plugins for this step
                for (const auto& group : ir_step->groups)
                {
                    for (const auto& plugin : group.plugins)
                    {
                        std::string key = make_plugin_key(step_name, group.name, plugin.name);
                        if (processed_plugins.count(key))
                            continue;

                        auto eff_type = evaluate_plugin_type(plugin, plugin_flags_, context);
                        if (eff_type == PluginType::Required)
                        {
                            logger.log(std::format(
                                "[fomod] Auto-installing Required plugin: \"{}\"", plugin.name));

                            for (const auto& [fn, fv] : plugin.condition_flags)
                            {
                                if (!fn.empty())
                                    plugin_flags_[fn] = fv;
                            }

                            enqueue_plugin_files(plugin, src_base, dst_base, ops, next_doc_order);
                            processed_plugins.insert(key);
                        }
                    }
                }
            }

            // Catch-all: auto-install Required plugins for steps not covered by JSON
            std::unordered_set<std::string> covered_steps;
            for (const auto& json_step : config_json["steps"])
                covered_steps.insert(json_step.value("name", ""));

            for (const auto& step : installer_.steps)
            {
                if (covered_steps.count(step.name))
                    continue;

                // Check step visibility
                if (step.visible && !evaluate_condition(*step.visible, plugin_flags_, context))
                    continue;

                for (const auto& group : step.groups)
                {
                    for (const auto& plugin : group.plugins)
                    {
                        std::string key = make_plugin_key(step.name, group.name, plugin.name);
                        if (processed_plugins.count(key))
                            continue;

                        auto eff_type = evaluate_plugin_type(plugin, plugin_flags_, context);
                        if (eff_type == PluginType::Required)
                        {
                            logger.log(std::format(
                                "[fomod] Auto-installing Required plugin: \"{}\"", plugin.name));

                            for (const auto& [fn, fv] : plugin.condition_flags)
                            {
                                if (!fn.empty())
                                    plugin_flags_[fn] = fv;
                            }

                            enqueue_plugin_files(plugin, src_base, dst_base, ops, next_doc_order);
                            processed_plugins.insert(key);
                        }
                    }
                }
            }

            // Pass 3: Auto-install alwaysInstall/installIfUsable files from unselected plugins
            int auto_file_count = 0;
            for (const auto& step : installer_.steps)
            {
                // Check step visibility
                if (step.visible && !evaluate_condition(*step.visible, plugin_flags_, context))
                    continue;

                for (const auto& group : step.groups)
                {
                    for (const auto& plugin : group.plugins)
                    {
                        std::string key = make_plugin_key(step.name, group.name, plugin.name);
                        if (processed_plugins.count(key))
                            continue;

                        auto eff_type = evaluate_plugin_type(plugin, plugin_flags_, context);
                        // Required plugins already handled above
                        if (eff_type == PluginType::Required)
                            continue;

                        for (const auto& entry : plugin.files)
                        {
                            bool should_install =
                                entry.always_install ||
                                (entry.install_if_usable && eff_type != PluginType::NotUsable);
                            if (!should_install)
                                continue;

                            int before = static_cast<int>(ops.size());
                            enqueue_entry(entry, src_base, dst_base, ops, next_doc_order);
                            if (static_cast<int>(ops.size()) > before)
                            {
                                auto_file_count++;
                                logger.log(std::format(
                                    "[fomod] Auto-installing {} ({}): {} -> {}",
                                    entry.is_folder ? "folder" : "file",
                                    entry.always_install ? "alwaysInstall" : "installIfUsable",
                                    entry.source,
                                    entry.destination));
                            }
                        }
                    }
                }
            }

            if (auto_file_count > 0)
            {
                logger.log(std::format(
                    "[fomod] Auto-install pass: {} alwaysInstall/installIfUsable file(s)",
                    auto_file_count));
            }
        }
        catch (const std::exception& ex)
        {
            ops.erase(ops.begin() + static_cast<std::ptrdiff_t>(initial_ops_size), ops.end());
            next_doc_order = initial_doc_order;
            logger.log_error(
                std::format("[fomod] Exception during optional file processing, rolled back queued "
                            "operations: {}",
                            ex.what()));
            throw;
        }
    }

    logger.log(
        std::format("[fomod] Total file operations queued from optional files: {}", ops.size()));
}

// ---------------------------------------------------------------------------
// process_conditional_files  (Phase 5: IR-based)
// ---------------------------------------------------------------------------

void FomodService::process_conditional_files(const std::string& src_base,
                                             const std::string& dst_base,
                                             const FomodDependencyContext* context,
                                             std::vector<FileOperation>& ops,
                                             int& next_doc_order)
{
    auto& logger = Logger::instance();
    if (installer_.conditional_patterns.empty())
    {
        logger.log("[fomod] No conditional file install patterns found");
        return;
    }

    logger.log(std::format("[fomod] Processing {} conditional file install patterns...",
                           installer_.conditional_patterns.size()));
    int processed = 0, skipped = 0;

    for (const auto& pattern : installer_.conditional_patterns)
    {
        if (!evaluate_condition(pattern.condition, plugin_flags_, context))
        {
            skipped++;
            logger.log("[fomod] Skipping pattern due to unmet dependencies");
            continue;
        }

        processed++;
        logger.log(std::format("[fomod] Processing conditional pattern {}/{}",
                               processed,
                               installer_.conditional_patterns.size()));

        for (const auto& entry : pattern.files)
        {
            enqueue_entry(entry, src_base, dst_base, ops, next_doc_order);
        }
    }

    logger.log(std::format(
        "[fomod] Queued {} conditional patterns, skipped {} patterns", processed, skipped));
}

// ---------------------------------------------------------------------------
// execute_file_operations (unchanged)
// ---------------------------------------------------------------------------

int FomodService::execute_file_operations(std::vector<FileOperation>& ops)
{
    auto& logger = Logger::instance();
    // MO2 uses stable sort: priority ascending, then XML document order as tiebreaker
    // Higher priority processed later = overwrites lower priority (last write wins)
    std::stable_sort(ops.begin(),
                     ops.end(),
                     [](const FileOperation& a, const FileOperation& b)
                     {
                         if (a.priority != b.priority)
                             return a.priority < b.priority;
                         return a.document_order < b.document_order;
                     });

    logger.log(
        std::format("[fomod] Executing {} file operations in priority order...", ops.size()));

    int failed = 0;
    for (const auto& op : ops)
    {
        try
        {
            if (op.type == FileOpType::File)
            {
                FileOperations::copy_file(op.source, op.destination);
            }
            else
            {
                FileOperations::copy_folder(op.source, op.destination);
            }
        }
        catch (const std::exception& ex)
        {
            ++failed;
            logger.log_error(std::format("[fomod] Failed to execute file operation: {} -> {}: {}",
                                         op.source,
                                         op.destination,
                                         ex.what()));
        }
    }

    if (failed > 0)
    {
        logger.log_warning(
            std::format("[fomod] {} of {} file operations failed", failed, ops.size()));
    }

    ops.clear();
    return failed;
}

}  // namespace mo2core
