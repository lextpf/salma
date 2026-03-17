#include "FomodService.h"
#include <algorithm>
#include <format>
#include <sstream>
#include "FileOperations.h"
#include "FomodDependencyEvaluator.h"
#include "Logger.h"
#include "Utils.h"

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2core
{

// parse_plugin_type_string and get_ordered_nodes are now in Utils.h

static const char* plugin_type_to_string(PluginType type)
{
    switch (type)
    {
        case PluginType::Required:
            return "Required";
        case PluginType::Recommended:
            return "Recommended";
        case PluginType::Optional:
            return "Optional";
        case PluginType::NotUsable:
            return "NotUsable";
        case PluginType::CouldBeUsable:
            return "CouldBeUsable";
    }
    return "Unknown";
}

static std::string normalize_destination_for_join(std::string destination)
{
    // FOMOD destinations are mod-root-relative. Values like "\" or "/"
    // mean "root", not an absolute filesystem path.
    while (!destination.empty() && (destination.front() == '\\' || destination.front() == '/'))
    {
        destination.erase(destination.begin());
    }
    while (destination.starts_with("./") || destination.starts_with(".\\"))
    {
        destination = destination.substr(2);
    }
    return destination;
}

bool FomodService::check_module_dependencies(const pugi::xml_document& doc,
                                             const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    auto node = doc.child("config").child("moduleDependencies");
    if (!node)
    {
        logger.log("[fomod] No module-level dependencies found");
        return true;
    }

    logger.log("[fomod] Checking module-level dependencies...");
    FomodDependencyEvaluator evaluator(plugin_flags_, context);
    bool met = evaluator.are_dependencies_met(node);

    if (!met)
    {
        logger.log(
            "[fomod] ERROR: Module-level dependencies not met - installation cannot proceed");
        return false;
    }

    logger.log("[fomod] Module-level dependencies satisfied");
    return true;
}

void FomodService::process_required_files(const pugi::xml_document& doc,
                                          const std::string& src_base,
                                          const std::string& dst_base,
                                          const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    auto req_parent = doc.child("config").child("requiredInstallFiles");
    if (!req_parent)
    {
        logger.log("[fomod] No required install files found in XML");
        return;
    }

    std::vector<pugi::xml_node> nodes;
    for (auto child : req_parent.children())
    {
        if (child.type() == pugi::node_element)
            nodes.push_back(child);
    }
    if (nodes.empty())
    {
        logger.log("[fomod] No required install files found in XML");
        return;
    }

    logger.log(
        std::format("[fomod] Processing {} required install files from XML...", nodes.size()));

    int file_count = 0, folder_count = 0;
    for (const auto& node : nodes)
    {
        std::string source = node.attribute("source").as_string();
        auto dest_attr = node.attribute("destination");
        // Folder destination default: when the attribute is absent, use the
        // source path (preserves archive structure). This matches MO2 behavior.
        // The spec arguably defaults to root, but all major managers use source.
        std::string destination = dest_attr ? dest_attr.as_string() : source;

        // MO2 behavior: skip empty source (no-op hack)
        if (source.empty())
        {
            logger.log(std::format("[fomod] Skipping {} node with empty source (no-op entry)",
                                   node.name()));
            continue;
        }

        // For <file> with empty destination, use the source filename.
        // Destination ending with / or \ means "into this directory".
        std::string name = node.name();
        if (name == "file" && destination.empty())
        {
            auto slash = source.find_last_of("/\\");
            destination = (slash != std::string::npos) ? source.substr(slash + 1) : source;
        }
        else if (name == "file" && !destination.empty() &&
                 (destination.back() == '/' || destination.back() == '\\'))
        {
            auto slash = source.find_last_of("/\\");
            auto filename = (slash != std::string::npos) ? source.substr(slash + 1) : source;
            destination += filename;
        }

        destination = normalize_destination_for_join(destination);
        auto src_path = (fs::path(src_base) / source).string();
        auto dst_path = (fs::path(dst_base) / destination).string();
        logger.log(std::format("[fomod] Mapped: {} -> {}", src_path, dst_path));

        int priority = get_priority(node);
        if (name == "file")
        {
            file_operations_.push_back(
                {FileOpType::File, src_path, dst_path, priority, next_document_order_++});
            file_count++;
        }
        else if (name == "folder")
        {
            file_operations_.push_back(
                {FileOpType::Folder, src_path, dst_path, priority, next_document_order_++});
            folder_count++;
        }
        else
        {
            logger.log(std::format("[fomod] Unknown node in requiredInstallFiles: {}", name));
        }
    }

    logger.log(std::format("[fomod] Queued {} files and {} folders from required install files",
                           file_count,
                           folder_count));
}

bool FomodService::validate_json_selections(const pugi::xml_document& doc, const json& config_json)
{
    auto& logger = Logger::instance();
    if (!config_json.contains("steps") || !config_json["steps"].is_array())
    {
        logger.log("[fomod] No steps in JSON - validation skipped");
        return true;
    }

    logger.log("[fomod] Validating JSON selections against FOMOD schema...");
    bool all_valid = true;

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
                        std::string plugin_name = plugin.get<std::string>();
                        if (!plugin_name.empty())
                        {
                            selected_plugins[step_name][group_name].insert(plugin_name);
                        }
                    }
                }
            }
        }
    }

    auto steps_parent = doc.child("config").child("installSteps");
    auto ordered_steps = get_ordered_nodes(steps_parent, "installStep");
    for (const auto& step_node : ordered_steps)
    {
        std::string step_name = step_node.attribute("name").as_string();
        auto step_it = selected_plugins.find(step_name);
        if (step_it == selected_plugins.end())
            continue;

        auto fg_parent = step_node.child("optionalFileGroups");
        auto ordered_groups = get_ordered_nodes(fg_parent, "group");
        for (const auto& group_node : ordered_groups)
        {
            std::string group_name = group_node.attribute("name").as_string();
            auto group_it = step_it->second.find(group_name);
            if (group_it == step_it->second.end())
                continue;

            if (!validate_group_selection(group_node, group_it->second))
            {
                all_valid = false;
            }
        }
    }

    logger.log(std::format("[fomod] JSON selections validated: {}",
                           all_valid ? "ALL VALID" : "SOME VIOLATIONS"));
    return all_valid;
}

bool FomodService::validate_group_selection(const pugi::xml_node& group_node,
                                            const std::unordered_set<std::string>& selected_plugins)
{
    auto& logger = Logger::instance();
    std::string group_type = group_node.attribute("type").as_string("SelectAny");
    std::string group_name = group_node.attribute("name").as_string();

    auto plugins_parent = group_node.child("plugins");
    auto ordered_plugins = get_ordered_nodes(plugins_parent, "plugin");
    std::vector<std::string> plugins_in_group;
    for (const auto& pnode : ordered_plugins)
    {
        std::string pname = pnode.attribute("name").as_string();
        if (!pname.empty())
            plugins_in_group.push_back(pname);
    }

    int selected_in_group = 0;
    for (const auto& p : plugins_in_group)
    {
        if (selected_plugins.count(p))
            selected_in_group++;
    }

    bool valid = true;
    if (group_type == "SelectExactlyOne")
        valid = (selected_in_group == 1);
    else if (group_type == "SelectAtLeastOne")
        valid = (selected_in_group >= 1);
    else if (group_type == "SelectAtMostOne")
        valid = (selected_in_group <= 1);
    else if (group_type == "SelectAll")
        valid = (selected_in_group == static_cast<int>(plugins_in_group.size()));

    if (!valid)
    {
        logger.log(std::format(
            "[fomod] WARNING: Group \"{}\" type \"{}\" validation failed: {} selected, {} total",
            group_name,
            group_type,
            selected_in_group,
            plugins_in_group.size()));
    }
    else
    {
        logger.log(std::format("[fomod] Group \"{}\" type \"{}\": {}/{} plugins selected - VALID",
                               group_name,
                               group_type,
                               selected_in_group,
                               plugins_in_group.size()));
    }
    return valid;
}

void FomodService::process_optional_files(const pugi::xml_document& doc,
                                          const json& config_json,
                                          const std::string& src_base,
                                          const std::string& dst_base,
                                          const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    bool has_steps = config_json.contains("steps") && config_json["steps"].is_array();
    std::unordered_set<std::string> processed_plugins;

    if (!has_steps)
    {
        logger.log("[fomod] No valid steps in JSON - optional selections skipped");
    }
    else
    {
        auto& steps = config_json["steps"];
        logger.log(std::format("[fomod] Processing optional files from JSON with {} step(s)",
                               steps.size()));

        // Build ordered XML step list for positional matching.
        // This avoids XPath name-collision issues when multiple steps/groups
        // share the same name.
        auto steps_parent = doc.child("config").child("installSteps");
        auto ordered_xml_steps = get_ordered_nodes(steps_parent, "installStep");

        // Map step names to their ordered XML nodes for occurrence tracking.
        std::unordered_map<std::string, std::vector<pugi::xml_node>> xml_steps_by_name;
        for (const auto& sn : ordered_xml_steps)
            xml_steps_by_name[sn.attribute("name").as_string()].push_back(sn);

        std::unordered_map<std::string, int> step_occurrence;

        for (const auto& step : steps)
        {
            std::string step_name = step.value("name", "");
            logger.log(std::format("[fomod] Processing step: \"{}\"", step_name));

            if (!step.contains("groups") || !step["groups"].is_array())
            {
                logger.log(std::format("[fomod] Step \"{}\" has no groups array", step_name));
                step_occurrence[step_name]++;
                continue;
            }

            // Find the correct XML step node by name + occurrence
            int occ = step_occurrence[step_name]++;
            pugi::xml_node xml_step_node;
            {
                auto it = xml_steps_by_name.find(step_name);
                if (it != xml_steps_by_name.end() && occ < static_cast<int>(it->second.size()))
                    xml_step_node = it->second[occ];
            }

            if (!xml_step_node)
            {
                logger.log_warning(std::format(
                    "[fomod] Could not find XML step \"{}\" occurrence {}", step_name, occ));
                continue;
            }

            // Build ordered group list within this specific step
            auto fg_parent = xml_step_node.child("optionalFileGroups");
            auto ordered_xml_groups = get_ordered_nodes(fg_parent, "group");
            std::unordered_map<std::string, std::vector<pugi::xml_node>> xml_groups_by_name;
            for (const auto& gn : ordered_xml_groups)
                xml_groups_by_name[gn.attribute("name").as_string()].push_back(gn);

            std::unordered_map<std::string, int> group_occurrence;

            logger.log(std::format("[fomod] Step has {} group(s)", step["groups"].size()));

            for (const auto& group : step["groups"])
            {
                std::string group_name = group.value("name", "");
                logger.log(std::format("[fomod] Processing group: \"{}\"", group_name));

                // Check for deselected plugins
                std::unordered_set<std::string> deselected;
                if (group.contains("deselected") && group["deselected"].is_array())
                {
                    for (const auto& d : group["deselected"])
                    {
                        std::string dname = d.get<std::string>();
                        if (!dname.empty())
                        {
                            deselected.insert(dname);
                            logger.log(std::format(
                                "[fomod] Plugin \"{}\" is marked as deselected, will skip", dname));
                        }
                    }
                }

                if (!group.contains("plugins") || !group["plugins"].is_array())
                {
                    logger.log(
                        std::format("[fomod] Group \"{}\" has no plugins array", group_name));
                    group_occurrence[group_name]++;
                    continue;
                }

                // Find the correct XML group node by name + occurrence
                int gocc = group_occurrence[group_name]++;
                pugi::xml_node xml_group_node;
                {
                    auto it = xml_groups_by_name.find(group_name);
                    if (it != xml_groups_by_name.end() &&
                        gocc < static_cast<int>(it->second.size()))
                        xml_group_node = it->second[gocc];
                }

                logger.log(std::format("[fomod] Group has {} plugin(s)", group["plugins"].size()));

                // Build plugin list within this specific group for name matching
                std::vector<pugi::xml_node> xml_plugins;
                if (xml_group_node)
                {
                    auto plugins_parent = xml_group_node.child("plugins");
                    for (auto pn = plugins_parent.child("plugin"); pn;
                         pn = pn.next_sibling("plugin"))
                        xml_plugins.push_back(pn);
                }

                std::unordered_map<std::string, int> plugin_occurrence;

                for (const auto& plugin : group["plugins"])
                {
                    std::string plugin_name = plugin.get<std::string>();
                    if (plugin_name.empty())
                    {
                        logger.log("[fomod] Skipping empty plugin name");
                        continue;
                    }

                    if (deselected.count(plugin_name))
                    {
                        logger.log(
                            std::format("[fomod] Skipping deselected plugin: \"{}\"", plugin_name));
                        continue;
                    }

                    logger.log(std::format(
                        "[fomod] Looking for plugin: \"{}\" in step \"{}\", group \"{}\"",
                        plugin_name,
                        step_name,
                        group_name));

                    // Find plugin by name + occurrence within this specific group
                    int pocc = plugin_occurrence[plugin_name]++;
                    pugi::xml_node matched_plugin;
                    int seen = 0;
                    for (const auto& pn : xml_plugins)
                    {
                        if (std::string(pn.attribute("name").as_string()) == plugin_name)
                        {
                            if (seen == pocc)
                            {
                                matched_plugin = pn;
                                break;
                            }
                            seen++;
                        }
                    }

                    if (matched_plugin)
                    {
                        logger.log(std::format("[fomod] Plugin \"{}\" (explicitly selected)",
                                               plugin_name));
                        process_plugin_node(matched_plugin, src_base, dst_base, context);
                        processed_plugins.insert(
                            make_plugin_key(step_name, group_name, plugin_name));
                    }
                    else
                    {
                        logger.log(std::format(
                            "[fomod] ERROR: Could not find plugin \"{}\" in step/group XML node",
                            plugin_name));
                    }
                }
            }

            // Auto-install Required plugins for this step (preserves document order)
            auto_install_required_for_step(
                doc, step_name, processed_plugins, src_base, dst_base, context);
        }
    }

    // Ensure Required plugins are installed even when JSON is partial or missing steps.
    auto_install_required_for_step(doc, "", processed_plugins, src_base, dst_base, context);

    // Second pass: auto-install alwaysInstall/installIfUsable files from unselected plugins
    process_auto_install_plugins(doc, processed_plugins, src_base, dst_base, context);

    logger.log(std::format("[fomod] Total file operations queued from optional files: {}",
                           file_operations_.size()));
}

void FomodService::process_conditional_files(const pugi::xml_document& doc,
                                             const std::string& src_base,
                                             const std::string& dst_base,
                                             const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    auto patterns_parent = doc.child("config").child("conditionalFileInstalls").child("patterns");
    if (!patterns_parent)
    {
        logger.log("[fomod] No conditional file install patterns found");
        return;
    }

    std::vector<pugi::xml_node> patterns;
    for (auto p : patterns_parent.children("pattern"))
        patterns.push_back(p);
    if (patterns.empty())
    {
        logger.log("[fomod] No conditional file install patterns found");
        return;
    }

    logger.log(
        std::format("[fomod] Processing {} conditional file install patterns...", patterns.size()));
    int processed = 0, skipped = 0;

    FomodDependencyEvaluator evaluator(plugin_flags_, context);

    for (const auto& pattern : patterns)
    {
        auto deps_node = pattern.child("dependencies");
        if (deps_node && !evaluator.are_dependencies_met(deps_node))
        {
            skipped++;
            logger.log("[fomod] Skipping pattern due to unmet dependencies");
            continue;
        }

        processed++;
        logger.log(std::format(
            "[fomod] Processing conditional pattern {}/{}", processed, patterns.size()));

        auto files_node = pattern.child("files");
        if (files_node)
        {
            process_files_node(files_node, src_base, dst_base);
        }
    }

    logger.log(std::format(
        "[fomod] Queued {} conditional patterns, skipped {} patterns", processed, skipped));
}

void FomodService::extract_flags(const pugi::xml_node& plugin_node)
{
    auto cf_node = plugin_node.child("conditionFlags");
    if (!cf_node)
        return;

    for (auto flag_node : cf_node.children("flag"))
    {
        std::string flag_name = flag_node.attribute("name").as_string();
        std::string flag_value = flag_node.text().as_string();
        if (!flag_name.empty())
        {
            plugin_flags_[flag_name] = flag_value;
        }
    }
}

void FomodService::process_files_node(const pugi::xml_node& files_node,
                                      const std::string& src_base,
                                      const std::string& dst_base)
{
    auto& logger = Logger::instance();
    int file_count = 0, folder_count = 0;

    for (auto file_node : files_node.children())
    {
        if (file_node.type() != pugi::node_element)
            continue;

        std::string source = file_node.attribute("source").as_string();
        auto dest_attr = file_node.attribute("destination");
        // When destination attribute is absent, default to source path
        // (preserves archive structure). Matches MO2/NMM/Vortex behavior.
        // The spec arguably defaults to root, but all major managers use source.
        std::string destination = dest_attr ? dest_attr.as_string() : source;

        // MO2 behavior: skip empty source
        if (source.empty())
        {
            logger.log(std::format("[fomod] Skipping {} node with empty source (no-op entry)",
                                   file_node.name()));
            continue;
        }

        // For <file> with empty destination, use the source filename.
        // Destination ending with / or \ means "into this directory".
        std::string name = file_node.name();
        if (name == "file" && destination.empty())
        {
            auto slash = source.find_last_of("/\\");
            destination = (slash != std::string::npos) ? source.substr(slash + 1) : source;
        }
        else if (name == "file" && !destination.empty() &&
                 (destination.back() == '/' || destination.back() == '\\'))
        {
            auto slash = source.find_last_of("/\\");
            auto filename = (slash != std::string::npos) ? source.substr(slash + 1) : source;
            destination += filename;
        }

        destination = normalize_destination_for_join(destination);
        auto src_path = (fs::path(src_base) / source).string();
        auto dst_path = (fs::path(dst_base) / destination).string();
        int priority = get_priority(file_node);

        if (name == "file")
        {
            logger.log(std::format("[fomod] Adding file operation: {} -> {} (priority: {})",
                                   src_path,
                                   dst_path,
                                   priority));
            file_operations_.push_back(
                {FileOpType::File, src_path, dst_path, priority, next_document_order_++});
            file_count++;
        }
        else if (name == "folder")
        {
            logger.log(std::format("[fomod] Adding folder operation: {} -> {} (priority: {})",
                                   src_path,
                                   dst_path,
                                   priority));
            file_operations_.push_back(
                {FileOpType::Folder, src_path, dst_path, priority, next_document_order_++});
            folder_count++;
        }
        else
        {
            logger.log(std::format("[fomod] Unknown file node type: {}", name));
        }
    }

    logger.log(std::format(
        "[fomod] Processed {} file(s) and {} folder(s) from files node", file_count, folder_count));
}

void FomodService::process_plugin_node(const pugi::xml_node& plugin_node,
                                       const std::string& src_base,
                                       const std::string& dst_base,
                                       const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    std::string plugin_name = plugin_node.attribute("name").as_string("unknown");
    logger.log(std::format("[fomod] Processing plugin node: \"{}\"", plugin_name));

    FomodDependencyEvaluator evaluator(plugin_flags_, context);

    // Check plugin-level dependencies
    auto plugin_deps = plugin_node.child("dependencies");
    if (plugin_deps && !evaluator.are_dependencies_met(plugin_deps))
    {
        logger.log(
            std::format("[fomod] Skipping plugin \"{}\" due to unmet dependencies", plugin_name));
        return;
    }

    // Check install step visibility
    auto current = plugin_node.parent();
    while (current && std::string(current.name()) != "installStep")
    {
        current = current.parent();
    }
    if (current)
    {
        auto visible_node = current.child("visible");
        if (visible_node && !evaluator.are_dependencies_met(visible_node))
        {
            logger.log(std::format(
                "[fomod] Skipping plugin \"{}\" due to unmet step visibility dependencies",
                plugin_name));
            return;
        }
    }

    // Apply flag side-effects only after plugin/step eligibility checks pass.
    extract_flags(plugin_node);

    auto files_node = plugin_node.child("files");
    if (files_node)
    {
        logger.log(std::format("[fomod] Plugin \"{}\" has files node, processing...", plugin_name));
        int before = static_cast<int>(file_operations_.size());
        process_files_node(files_node, src_base, dst_base);
        int after = static_cast<int>(file_operations_.size());
        logger.log(std::format(
            "[fomod] Plugin \"{}\": Added {} file operation(s)", plugin_name, after - before));
    }
    else
    {
        logger.log(std::format("[fomod] WARNING: Plugin \"{}\" has no files node", plugin_name));
    }
}

int FomodService::get_priority(const pugi::xml_node& node)
{
    auto attr = node.attribute("priority");
    if (attr)
        return attr.as_int(0);
    return 0;  // MO2 default priority is 0
}

void FomodService::execute_file_operations()
{
    auto& logger = Logger::instance();
    // MO2 uses stable sort: priority ascending, then XML document order as tiebreaker
    // Higher priority processed later = overwrites lower priority (last write wins)
    std::stable_sort(file_operations_.begin(),
                     file_operations_.end(),
                     [](const FileOperation& a, const FileOperation& b)
                     {
                         if (a.priority != b.priority)
                             return a.priority < b.priority;
                         return a.document_order < b.document_order;
                     });

    logger.log(std::format("[fomod] Executing {} file operations in priority order...",
                           file_operations_.size()));

    for (const auto& op : file_operations_)
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
            logger.log(std::format("[fomod] ERROR: Failed to execute file operation: {} -> {}: {}",
                                   op.source,
                                   op.destination,
                                   ex.what()));
        }
    }

    file_operations_.clear();
}

std::string FomodService::escape_xpath_string(const std::string& value)
{
    if (value.empty())
        return "''";

    bool has_single = value.find('\'') != std::string::npos;
    bool has_double = value.find('"') != std::string::npos;

    if (has_single && has_double)
    {
        // Use concat() to handle both quote types
        std::string result = "concat(";
        std::string remaining = value;
        bool first = true;

        while (!remaining.empty())
        {
            auto sq = remaining.find('\'');
            auto dq = remaining.find('"');

            if (sq == std::string::npos && dq == std::string::npos)
            {
                if (!first)
                    result += ",";
                result += "'" + remaining + "'";
                break;
            }
            else if (sq != std::string::npos && (dq == std::string::npos || sq < dq))
            {
                if (sq > 0)
                {
                    if (!first)
                        result += ",";
                    result += "'" + remaining.substr(0, sq) + "'";
                    first = false;
                }
                if (!first)
                    result += ",";
                result += "\"'\"";
                first = false;
                remaining = remaining.substr(sq + 1);
            }
            else
            {
                if (dq > 0)
                {
                    if (!first)
                        result += ",";
                    result += "'" + remaining.substr(0, dq) + "'";
                    first = false;
                }
                if (!first)
                    result += ",";
                result += "'\"'";
                first = false;
                remaining = remaining.substr(dq + 1);
            }
        }
        result += ")";
        return result;
    }
    else if (has_single)
    {
        return "\"" + value + "\"";
    }
    else
    {
        return "'" + value + "'";
    }
}

std::string FomodService::normalize_string(const std::string& value)
{
    if (value.empty())
        return "";

    std::string result = value;

    // Trim whitespace
    auto start = result.find_first_not_of(" \t\n\r");
    auto end = result.find_last_not_of(" \t\n\r");
    if (start == std::string::npos)
        return "";
    result = result.substr(start, end - start + 1);

    // Decode XML entities
    auto replace_all = [&](const std::string& from, const std::string& to)
    {
        size_t pos = 0;
        while ((pos = result.find(from, pos)) != std::string::npos)
        {
            result.replace(pos, from.length(), to);
            pos += to.length();
        }
    };

    replace_all("&quot;", "\"");
    replace_all("&amp;", "&");
    replace_all("&lt;", "<");
    replace_all("&gt;", ">");
    replace_all("&apos;", "'");

    return result;
}

std::vector<pugi::xml_node> FomodService::find_plugin_nodes_by_value(const pugi::xml_document& doc,
                                                                     const std::string& step_name,
                                                                     const std::string& group_name,
                                                                     const std::string& plugin_name)
{
    auto& logger = Logger::instance();
    std::vector<pugi::xml_node> results;
    auto normalized_plugin = normalize_string(plugin_name);

    auto steps_parent = doc.child("config").child("installSteps");
    auto ordered_steps = get_ordered_nodes(steps_parent, "installStep");
    for (const auto& step_node : ordered_steps)
    {
        auto step_attr = normalize_string(step_node.attribute("name").as_string());

        if (!step_name.empty())
        {
            auto normalized_step = normalize_string(step_name);
            auto lower = [](std::string s)
            {
                std::transform(
                    s.begin(), s.end(), s.begin(), [](unsigned char c) { return std::tolower(c); });
                return s;
            };
            if (lower(step_attr) != lower(normalized_step))
                continue;
        }

        auto fg_parent = step_node.child("optionalFileGroups");
        auto ordered_groups = get_ordered_nodes(fg_parent, "group");
        for (const auto& group_node : ordered_groups)
        {
            auto group_attr = normalize_string(group_node.attribute("name").as_string());

            if (!group_name.empty())
            {
                auto lower = [](std::string s)
                {
                    std::transform(s.begin(),
                                   s.end(),
                                   s.begin(),
                                   [](unsigned char c) { return std::tolower(c); });
                    return s;
                };
                if (lower(group_attr) != lower(normalize_string(group_name)))
                    continue;
            }

            auto plugins_parent = group_node.child("plugins");
            auto ordered_plugins = get_ordered_nodes(plugins_parent, "plugin");
            for (const auto& pnode : ordered_plugins)
            {
                auto pattr = normalize_string(pnode.attribute("name").as_string());

                auto lower = [](std::string s)
                {
                    std::transform(s.begin(),
                                   s.end(),
                                   s.begin(),
                                   [](unsigned char c) { return std::tolower(c); });
                    return s;
                };
                if (lower(pattr) == lower(normalized_plugin))
                {
                    logger.log(std::format(
                        "[fomod] Found plugin by value comparison: \"{}\" matches \"{}\"",
                        pattr,
                        plugin_name));
                    results.push_back(pnode);
                }
            }
        }
    }

    return results;
}

std::string FomodService::make_plugin_key(const std::string& step,
                                          const std::string& group,
                                          const std::string& plugin)
{
    return to_lower(step) + "|" + to_lower(group) + "|" + to_lower(plugin);
}

PluginType FomodService::evaluate_plugin_type(const pugi::xml_node& plugin_node,
                                              const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    auto type_desc = plugin_node.child("typeDescriptor");
    if (!type_desc)
        return PluginType::Optional;

    // Static type: <type name="Required"/>
    auto type_node = type_desc.child("type");
    if (type_node)
    {
        return parse_plugin_type_string(type_node.attribute("name").as_string("Optional"));
    }

    // Dynamic type: <dependencyType><defaultType/><patterns>...</patterns></dependencyType>
    auto dep_type = type_desc.child("dependencyType");
    if (dep_type)
    {
        FomodDependencyEvaluator evaluator(plugin_flags_, context);

        auto patterns = dep_type.child("patterns");
        if (patterns)
        {
            for (auto pattern : patterns.children("pattern"))
            {
                auto deps = pattern.child("dependencies");
                if (!deps || evaluator.are_dependencies_met(deps))
                {
                    auto ptype = pattern.child("type");
                    if (ptype)
                    {
                        auto result =
                            parse_plugin_type_string(ptype.attribute("name").as_string("Optional"));
                        logger.log(
                            std::format("[fomod] Plugin type resolved via dependency pattern: {}",
                                        plugin_type_to_string(result)));
                        return result;
                    }
                }
            }
        }

        auto default_type = dep_type.child("defaultType");
        if (default_type)
        {
            return parse_plugin_type_string(default_type.attribute("name").as_string("Optional"));
        }
    }

    return PluginType::Optional;
}

void FomodService::auto_install_required_for_step(const pugi::xml_document& doc,
                                                  const std::string& step_name,
                                                  std::unordered_set<std::string>& processed_keys,
                                                  const std::string& src_base,
                                                  const std::string& dst_base,
                                                  const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    FomodDependencyEvaluator evaluator(plugin_flags_, context);

    auto steps_parent = doc.child("config").child("installSteps");
    auto ordered_steps = get_ordered_nodes(steps_parent, "installStep");
    for (const auto& step_node : ordered_steps)
    {
        std::string xml_step_name = step_node.attribute("name").as_string();

        // Match by step name (case-insensitive, normalized)
        auto lower = [](std::string s)
        {
            std::transform(
                s.begin(), s.end(), s.begin(), [](unsigned char c) { return std::tolower(c); });
            return s;
        };
        if (!step_name.empty() &&
            lower(normalize_string(xml_step_name)) != lower(normalize_string(step_name)))
            continue;

        // Check step visibility
        auto visible = step_node.child("visible");
        if (visible && !evaluator.are_dependencies_met(visible))
            continue;

        auto fg_parent = step_node.child("optionalFileGroups");
        auto ordered_groups = get_ordered_nodes(fg_parent, "group");
        for (const auto& group_node : ordered_groups)
        {
            std::string group_name = group_node.attribute("name").as_string();

            auto plugins_parent = group_node.child("plugins");
            auto ordered_plugins = get_ordered_nodes(plugins_parent, "plugin");
            for (const auto& plugin_node : ordered_plugins)
            {
                std::string plugin_name = plugin_node.attribute("name").as_string();

                std::string key = make_plugin_key(xml_step_name, group_name, plugin_name);
                if (processed_keys.count(key))
                    continue;

                PluginType type = evaluate_plugin_type(plugin_node, context);
                if (type == PluginType::Required)
                {
                    logger.log(std::format("[fomod] Auto-installing Required plugin: \"{}\"",
                                           plugin_name));
                    process_plugin_node(plugin_node, src_base, dst_base, context);
                    processed_keys.insert(key);
                }
            }
        }
    }
}

void FomodService::process_auto_install_plugins(
    const pugi::xml_document& doc,
    const std::unordered_set<std::string>& processed_keys,
    const std::string& src_base,
    const std::string& dst_base,
    const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    FomodDependencyEvaluator evaluator(plugin_flags_, context);
    int auto_file_count = 0;

    auto steps_parent = doc.child("config").child("installSteps");
    auto ordered_steps = get_ordered_nodes(steps_parent, "installStep");
    for (const auto& step_node : ordered_steps)
    {
        std::string step_name = step_node.attribute("name").as_string();

        // Check step visibility
        auto visible = step_node.child("visible");
        if (visible && !evaluator.are_dependencies_met(visible))
            continue;

        auto fg_parent = step_node.child("optionalFileGroups");
        auto ordered_groups = get_ordered_nodes(fg_parent, "group");
        for (const auto& group_node : ordered_groups)
        {
            std::string group_name = group_node.attribute("name").as_string();

            auto plugins_parent = group_node.child("plugins");
            auto ordered_plugins = get_ordered_nodes(plugins_parent, "plugin");
            for (const auto& plugin_node : ordered_plugins)
            {
                std::string plugin_name = plugin_node.attribute("name").as_string();

                std::string key = make_plugin_key(step_name, group_name, plugin_name);
                if (processed_keys.count(key))
                    continue;

                PluginType type = evaluate_plugin_type(plugin_node, context);

                // Required plugins already handled inline per-step
                if (type == PluginType::Required)
                    continue;

                // Check for alwaysInstall/installIfUsable files from unselected plugins
                auto files_node = plugin_node.child("files");
                if (files_node)
                {
                    int before = static_cast<int>(file_operations_.size());
                    process_files_node_filtered(files_node, src_base, dst_base, type);
                    auto_file_count += static_cast<int>(file_operations_.size()) - before;
                }
            }
        }
    }

    if (auto_file_count > 0)
    {
        logger.log(
            std::format("[fomod] Auto-install pass: {} alwaysInstall/installIfUsable file(s)",
                        auto_file_count));
    }
}

void FomodService::process_files_node_filtered(const pugi::xml_node& files_node,
                                               const std::string& src_base,
                                               const std::string& dst_base,
                                               PluginType plugin_type)
{
    auto& logger = Logger::instance();

    for (auto file_node : files_node.children())
    {
        if (file_node.type() != pugi::node_element)
            continue;

        bool always_install = xml_bool_attribute_true(file_node.attribute("alwaysInstall"));
        bool install_if_usable = xml_bool_attribute_true(file_node.attribute("installIfUsable"));

        bool should_install =
            always_install || (install_if_usable && plugin_type != PluginType::NotUsable);
        if (!should_install)
            continue;

        std::string source = file_node.attribute("source").as_string();
        auto dest_attr = file_node.attribute("destination");
        std::string destination = dest_attr ? dest_attr.as_string() : source;
        if (source.empty())
            continue;

        // For <file> with empty destination, use the source filename.
        // Destination ending with / or \ means "into this directory".
        std::string name = file_node.name();
        if (name == "file" && destination.empty())
        {
            auto slash = source.find_last_of("/\\");
            destination = (slash != std::string::npos) ? source.substr(slash + 1) : source;
        }
        else if (name == "file" && !destination.empty() &&
                 (destination.back() == '/' || destination.back() == '\\'))
        {
            auto slash = source.find_last_of("/\\");
            auto filename = (slash != std::string::npos) ? source.substr(slash + 1) : source;
            destination += filename;
        }

        destination = normalize_destination_for_join(destination);
        auto src_path = (fs::path(src_base) / source).string();
        auto dst_path = (fs::path(dst_base) / destination).string();
        int priority = get_priority(file_node);

        if (name == "file" || name == "folder")
        {
            logger.log(std::format("[fomod] Auto-installing {} ({}): {} -> {} (priority: {})",
                                   name,
                                   always_install ? "alwaysInstall" : "installIfUsable",
                                   src_path,
                                   dst_path,
                                   priority));
            file_operations_.push_back({name == "file" ? FileOpType::File : FileOpType::Folder,
                                        src_path,
                                        dst_path,
                                        priority,
                                        next_document_order_++});
        }
    }
}

}  // namespace mo2core
