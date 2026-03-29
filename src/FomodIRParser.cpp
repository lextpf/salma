#include "FomodIRParser.h"
#include "FomodDependencyEvaluator.h"
#include "Logger.h"
#include "Utils.h"

#include <concepts>
#include <format>
#include <string_view>

using namespace std::string_view_literals;

static_assert(mo2core::no_hash_collisions(std::array{
    "flagDependency"sv,
    "fileDependency"sv,
    "gameDependency"sv,
    "pluginDependency"sv,
    "fomodDependency"sv,
    "fommDependency"sv,
    "foseDependency"sv,
    "dependencies"sv,
}));

namespace mo2core
{

// Iterate <file>/<folder> child elements with non-empty source attributes.
template <typename Func>
    requires std::invocable<Func, const pugi::xml_node&>
static void for_each_file_node(const pugi::xml_node& parent, Func&& fn)
{
    for (auto node : parent.children())
    {
        if (node.type() != pugi::node_element)
            continue;
        std::string_view name = node.name();
        if (name != "file" && name != "folder")
            continue;
        if (std::string_view(node.attribute("source").as_string()).empty())
            continue;
        fn(node);
    }
}

// ---------------------------------------------------------------------------
// compile_condition: recursively convert a <dependencies> XML node into IR
// ---------------------------------------------------------------------------

// Guard against malicious/malformed XML: depth prevents stack overflow,
// breadth prevents CPU-bound DoS from millions of sibling nodes.
static constexpr int MAX_CONDITION_CHILDREN = 10000;

static FomodCondition compile_condition_impl(const pugi::xml_node& deps_node, int depth)
{
    FomodCondition cond;
    cond.type = FomodConditionType::Composite;

    if (depth > MAX_DEPENDENCY_DEPTH)
    {
        // Bail out with an always-false empty Or to safely reject overly deep conditions
        mo2core::Logger::instance().log_warning(
            "[fomod] Condition nesting exceeds maximum depth, treating as always-false");
        cond.op = FomodConditionOp::Or;
        return cond;
    }

    cond.op = parse_enum<FomodConditionOp>(deps_node.attribute("operator").as_string("And"));

    int child_count = 0;
    for (auto child : deps_node.children())
    {
        if (child.type() != pugi::node_element)
            continue;
        if (++child_count > MAX_CONDITION_CHILDREN)
        {
            mo2core::Logger::instance().log_warning(
                std::format("[fomod] Condition node has more than {} children, truncating",
                            MAX_CONDITION_CHILDREN));
            break;
        }

        std::string_view name = child.name();
        FomodCondition leaf;

        switch (fnv1a_hash(name.data(), name.size()))
        {
            case "flagDependency"_h:
                leaf.type = FomodConditionType::Flag;
                leaf.flag_name = child.attribute("flag").as_string();
                leaf.flag_value = child.attribute("value").as_string();
                cond.children.push_back(std::move(leaf));
                break;
            case "fileDependency"_h:
                leaf.type = FomodConditionType::File;
                leaf.file_path = child.attribute("file").as_string();
                leaf.file_state = child.attribute("state").as_string("Active");
                cond.children.push_back(std::move(leaf));
                break;
            case "gameDependency"_h:
                leaf.type = FomodConditionType::Game;
                leaf.version = child.attribute("version").as_string();
                cond.children.push_back(std::move(leaf));
                break;
            case "pluginDependency"_h:
                leaf.type = FomodConditionType::Plugin;
                leaf.plugin_name = child.attribute("name").as_string();
                leaf.plugin_type = child.attribute("type").as_string("Active");
                cond.children.push_back(std::move(leaf));
                break;
            case "fomodDependency"_h:
                leaf.type = FomodConditionType::Fomod;
                leaf.fomod_name = child.attribute("name").as_string();
                cond.children.push_back(std::move(leaf));
                break;
            case "fommDependency"_h:
                leaf.type = FomodConditionType::Fomm;
                leaf.version = child.attribute("version").as_string();
                cond.children.push_back(std::move(leaf));
                break;
            case "foseDependency"_h:
                leaf.type = FomodConditionType::Fose;
                leaf.version = child.attribute("version").as_string();
                cond.children.push_back(std::move(leaf));
                break;
            case "dependencies"_h:
                cond.children.push_back(compile_condition_impl(child, depth + 1));
                break;
            default:
                mo2core::Logger::instance().log_warning(
                    std::format("[fomod] Unknown condition element \"{}\" skipped", child.name()));
                break;
        }
    }

    return cond;
}

FomodCondition FomodIRParser::compile_condition(const pugi::xml_node& deps_node)
{
    return compile_condition_impl(deps_node, 0);
}

// ---------------------------------------------------------------------------
// parse_file_entry: convert a <file> or <folder> XML node into IR
// ---------------------------------------------------------------------------
FomodFileEntry FomodIRParser::parse_file_entry(const pugi::xml_node& node,
                                               const std::string& archive_prefix)
{
    FomodFileEntry entry;
    std::string source_attr = node.attribute("source").as_string();
    auto dest_attr = node.attribute("destination");
    std::string dest_raw = dest_attr ? dest_attr.as_string() : source_attr;

    bool is_folder = (std::string(node.name()) == "folder");
    entry.is_folder = is_folder;

    // Build archive-relative source path
    std::string full_source =
        archive_prefix.empty() ? source_attr : (archive_prefix + "/" + source_attr);
    entry.source = normalize_path(full_source);

    // Resolve and normalize destination
    if (is_folder)
    {
        entry.destination = normalize_path(dest_raw);
    }
    else
    {
        entry.destination = normalize_path(resolve_file_destination(source_attr, dest_raw, true));
    }

    entry.priority = node.attribute("priority").as_int(0);
    entry.always_install = xml_bool_attribute_true(node.attribute("alwaysInstall"));
    entry.install_if_usable = xml_bool_attribute_true(node.attribute("installIfUsable"));

    return entry;
}

// ---------------------------------------------------------------------------
// parse_group_type
// ---------------------------------------------------------------------------
FomodGroupType FomodIRParser::parse_group_type(const std::string& type_str)
{
    return parse_enum<FomodGroupType>(type_str);
}

// ---------------------------------------------------------------------------
// parse: main entry point - walk the FOMOD XML and build the IR
// ---------------------------------------------------------------------------
FomodInstaller FomodIRParser::parse(const pugi::xml_document& doc,
                                    const std::string& archive_prefix)
{
    FomodInstaller installer;
    auto config = doc.child("config");
    if (!config)
        return installer;

    // Module dependencies
    auto mod_deps = config.child("moduleDependencies");
    if (mod_deps)
    {
        auto deps_child = mod_deps.child("dependencies");
        if (deps_child)
            installer.module_dependencies = compile_condition(deps_child);
        else if (mod_deps.first_child())
            installer.module_dependencies = compile_condition(mod_deps);
    }

    // Required install files
    auto req_parent = config.child("requiredInstallFiles");
    for_each_file_node(
        req_parent,
        [&](const pugi::xml_node& node)
        { installer.required_files.push_back(parse_file_entry(node, archive_prefix)); });

    // Install steps (ordered)
    auto steps_parent = config.child("installSteps");
    auto ordered_steps = get_ordered_nodes(steps_parent, "installStep");
    int step_ordinal = 0;
    for (const auto& step_node : ordered_steps)
    {
        FomodStep step;
        step.name = step_node.attribute("name").as_string();
        step.ordinal = step_ordinal++;

        // Step visibility
        auto visible_node = step_node.child("visible");
        if (visible_node)
        {
            auto deps = visible_node.child("dependencies");
            if (deps)
                step.visible = compile_condition(deps);
            else if (visible_node.first_child())
                step.visible = compile_condition(visible_node);
        }

        // Groups (ordered)
        auto fg_parent = step_node.child("optionalFileGroups");
        auto ordered_groups = get_ordered_nodes(fg_parent, "group");
        for (const auto& group_node : ordered_groups)
        {
            FomodGroup group;
            group.name = group_node.attribute("name").as_string();
            group.type = parse_group_type(group_node.attribute("type").as_string("SelectAny"));

            // Plugins (ordered)
            auto plugins_parent = group_node.child("plugins");
            auto ordered_plugins = get_ordered_nodes(plugins_parent, "plugin");
            for (const auto& pnode : ordered_plugins)
            {
                FomodPlugin plugin;
                plugin.name = pnode.attribute("name").as_string();

                // Type descriptor
                auto type_desc = pnode.child("typeDescriptor");
                if (type_desc)
                {
                    auto type_node = type_desc.child("type");
                    if (type_node)
                    {
                        plugin.type = parse_plugin_type_string(
                            type_node.attribute("name").as_string("Optional"));
                    }
                    else
                    {
                        auto dep_type = type_desc.child("dependencyType");
                        if (dep_type)
                        {
                            auto default_type = dep_type.child("defaultType");
                            if (default_type)
                            {
                                plugin.type = parse_plugin_type_string(
                                    default_type.attribute("name").as_string("Optional"));
                            }

                            auto patterns = dep_type.child("patterns");
                            if (patterns)
                            {
                                for (auto pat : patterns.children("pattern"))
                                {
                                    FomodTypePattern tp;
                                    auto pat_deps = pat.child("dependencies");
                                    if (pat_deps)
                                        tp.condition = compile_condition(pat_deps);
                                    // else: empty And = always true

                                    auto ptype = pat.child("type");
                                    if (ptype)
                                    {
                                        tp.result_type = parse_plugin_type_string(
                                            ptype.attribute("name").as_string("Optional"));
                                    }
                                    plugin.type_patterns.push_back(std::move(tp));
                                }
                            }
                        }
                    }
                }

                // Files
                auto files_node = pnode.child("files");
                if (files_node)
                {
                    for_each_file_node(
                        files_node,
                        [&](const pugi::xml_node& fnode)
                        { plugin.files.push_back(parse_file_entry(fnode, archive_prefix)); });
                }

                // Condition flags
                auto cf_node = pnode.child("conditionFlags");
                if (cf_node)
                {
                    for (auto flag_node : cf_node.children("flag"))
                    {
                        std::string flag_name = flag_node.attribute("name").as_string();
                        std::string flag_value = flag_node.text().as_string();
                        if (!flag_name.empty())
                            plugin.condition_flags.emplace_back(flag_name, flag_value);
                    }
                }

                // Plugin dependencies
                auto plugin_deps = pnode.child("dependencies");
                if (plugin_deps)
                    plugin.dependencies = compile_condition(plugin_deps);

                group.plugins.push_back(std::move(plugin));
            }
            step.groups.push_back(std::move(group));
        }
        installer.steps.push_back(std::move(step));
    }

    // Conditional file installs
    auto cfi = config.child("conditionalFileInstalls");
    if (cfi)
    {
        auto patterns = cfi.child("patterns");
        if (patterns)
        {
            for (auto pat : patterns.children("pattern"))
            {
                FomodConditionalPattern cp;
                auto deps = pat.child("dependencies");
                if (deps)
                    cp.condition = compile_condition(deps);
                // else: empty And = always true

                auto files = pat.child("files");
                if (files)
                {
                    for_each_file_node(
                        files,
                        [&](const pugi::xml_node& fnode)
                        { cp.files.push_back(parse_file_entry(fnode, archive_prefix)); });
                }
                installer.conditional_patterns.push_back(std::move(cp));
            }
        }
    }

    return installer;
}

}  // namespace mo2core
