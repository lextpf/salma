#include "FomodIRParser.h"
#include "Utils.h"

namespace mo2core
{

// ---------------------------------------------------------------------------
// compile_condition: recursively convert a <dependencies> XML node into IR
// ---------------------------------------------------------------------------
FomodCondition FomodIRParser::compile_condition(const pugi::xml_node& deps_node)
{
    FomodCondition cond;
    cond.type = FomodConditionType::Composite;

    std::string op = deps_node.attribute("operator").as_string("And");
    cond.op = (op == "Or") ? FomodConditionOp::Or : FomodConditionOp::And;

    for (auto child : deps_node.children())
    {
        if (child.type() != pugi::node_element)
            continue;

        std::string name = child.name();

        if (name == "flagDependency")
        {
            FomodCondition leaf;
            leaf.type = FomodConditionType::Flag;
            leaf.flag_name = child.attribute("flag").as_string();
            leaf.flag_value = child.attribute("value").as_string();
            cond.children.push_back(std::move(leaf));
        }
        else if (name == "fileDependency")
        {
            FomodCondition leaf;
            leaf.type = FomodConditionType::File;
            leaf.file_path = child.attribute("file").as_string();
            leaf.file_state = child.attribute("state").as_string("Active");
            cond.children.push_back(std::move(leaf));
        }
        else if (name == "gameDependency")
        {
            FomodCondition leaf;
            leaf.type = FomodConditionType::Game;
            leaf.version = child.attribute("version").as_string();
            cond.children.push_back(std::move(leaf));
        }
        else if (name == "pluginDependency")
        {
            FomodCondition leaf;
            leaf.type = FomodConditionType::Plugin;
            leaf.plugin_name = child.attribute("name").as_string();
            leaf.plugin_type = child.attribute("type").as_string("Active");
            cond.children.push_back(std::move(leaf));
        }
        else if (name == "fomodDependency")
        {
            FomodCondition leaf;
            leaf.type = FomodConditionType::Fomod;
            leaf.fomod_name = child.attribute("name").as_string();
            cond.children.push_back(std::move(leaf));
        }
        else if (name == "fommDependency")
        {
            FomodCondition leaf;
            leaf.type = FomodConditionType::Fomm;
            leaf.version = child.attribute("version").as_string();
            cond.children.push_back(std::move(leaf));
        }
        else if (name == "foseDependency")
        {
            FomodCondition leaf;
            leaf.type = FomodConditionType::Fose;
            leaf.version = child.attribute("version").as_string();
            cond.children.push_back(std::move(leaf));
        }
        else if (name == "dependencies")
        {
            cond.children.push_back(compile_condition(child));
        }
        // Unknown elements are silently skipped (matches existing behavior)
    }

    return cond;
}

// ---------------------------------------------------------------------------
// resolve_file_destination: replicate FomodService file destination semantics
// ---------------------------------------------------------------------------
std::string FomodIRParser::resolve_file_destination(const std::string& source,
                                                    const std::string& dest_raw)
{
    if (dest_raw.empty())
    {
        auto slash = source.find_last_of("/\\");
        return (slash != std::string::npos) ? source.substr(slash + 1) : source;
    }
    if (dest_raw.back() == '/' || dest_raw.back() == '\\')
    {
        auto slash = source.find_last_of("/\\");
        auto filename = (slash != std::string::npos) ? source.substr(slash + 1) : source;
        return dest_raw + filename;
    }
    return dest_raw;
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
        entry.destination = normalize_path(resolve_file_destination(source_attr, dest_raw));
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
    if (type_str == "SelectExactlyOne")
        return FomodGroupType::SelectExactlyOne;
    if (type_str == "SelectAtMostOne")
        return FomodGroupType::SelectAtMostOne;
    if (type_str == "SelectAtLeastOne")
        return FomodGroupType::SelectAtLeastOne;
    if (type_str == "SelectAll")
        return FomodGroupType::SelectAll;
    return FomodGroupType::SelectAny;
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
    for (auto node : req_parent.children())
    {
        if (node.type() != pugi::node_element)
            continue;
        std::string name = node.name();
        if (name != "file" && name != "folder")
            continue;
        std::string source = node.attribute("source").as_string();
        if (source.empty())
            continue;
        installer.required_files.push_back(parse_file_entry(node, archive_prefix));
    }

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
                    for (auto fnode : files_node.children())
                    {
                        if (fnode.type() != pugi::node_element)
                            continue;
                        std::string fname = fnode.name();
                        if (fname != "file" && fname != "folder")
                            continue;
                        std::string source = fnode.attribute("source").as_string();
                        if (source.empty())
                            continue;
                        plugin.files.push_back(parse_file_entry(fnode, archive_prefix));
                    }
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
                    for (auto fnode : files.children())
                    {
                        if (fnode.type() != pugi::node_element)
                            continue;
                        std::string fname = fnode.name();
                        if (fname != "file" && fname != "folder")
                            continue;
                        std::string source = fnode.attribute("source").as_string();
                        if (source.empty())
                            continue;
                        cp.files.push_back(parse_file_entry(fnode, archive_prefix));
                    }
                }
                installer.conditional_patterns.push_back(std::move(cp));
            }
        }
    }

    return installer;
}

}  // namespace mo2core
