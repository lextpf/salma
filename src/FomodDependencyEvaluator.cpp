#include "FomodDependencyEvaluator.h"
#include <algorithm>
#include <filesystem>
#include <format>
#include <regex>
#include <sstream>
#include "Logger.h"
#include "Utils.h"

namespace fs = std::filesystem;

namespace mo2core
{

FomodDependencyEvaluator::FomodDependencyEvaluator(
    std::unordered_map<std::string, std::string>& plugin_flags,
    const FomodDependencyContext* context)
    : plugin_flags_(plugin_flags),
      context_(context)
{
}

bool FomodDependencyEvaluator::are_dependencies_met(const pugi::xml_node& dependencies_node)
{
    if (!dependencies_node)
        return true;

    std::string op_type = dependencies_node.attribute("operator").as_string("And");
    if (op_type != "And" && op_type != "Or")
    {
        Logger::instance().log(std::format(
            "[fomod] WARNING: Unknown dependencies operator \"{}\", defaulting to And", op_type));
        op_type = "And";
    }

    bool result = (op_type == "And");

    for (auto dep_node : dependencies_node.children())
    {
        if (dep_node.type() != pugi::node_element)
            continue;

        std::string name = dep_node.name();
        bool dep_met = false;
        bool recognized = true;

        if (name == "flagDependency")
            dep_met = evaluate_flag_dependency(dep_node);
        else if (name == "fileDependency")
            dep_met = evaluate_file_dependency(dep_node);
        else if (name == "gameDependency")
            dep_met = evaluate_game_dependency(dep_node);
        else if (name == "pluginDependency")
            dep_met = evaluate_plugin_dependency(dep_node);
        else if (name == "fomodDependency")
            dep_met = evaluate_fomod_dependency(dep_node);
        else if (name == "fommDependency")
            dep_met = evaluate_fomm_dependency(dep_node);
        else if (name == "foseDependency")
            dep_met = evaluate_fose_dependency(dep_node);
        // MO2: nested <dependencies> creates a SubCondition (recursive composite)
        else if (name == "dependencies")
            dep_met = are_dependencies_met(dep_node);
        else
            recognized = false;

        // Unknown nodes are treated as neutral (no-op) to avoid short-circuiting
        // Or-conditions to true.
        if (!recognized)
        {
            Logger::instance().log(
                std::format("[fomod] WARNING: Unknown dependency node \"{}\" ignored", name));
            continue;
        }

        if (op_type == "And")
            result = result && dep_met;
        else if (op_type == "Or")
            result = result || dep_met;
    }

    return result;
}

bool FomodDependencyEvaluator::evaluate_flag_dependency(const pugi::xml_node& dep_node)
{
    std::string flag_name = dep_node.attribute("flag").as_string();
    std::string required_value = dep_node.attribute("value").as_string();

    auto it = plugin_flags_.find(flag_name);
    bool flag_exists = (it != plugin_flags_.end());
    bool flag_matches;

    if (!flag_exists)
    {
        // MO2 behavior: unset flag matches if required value is empty
        flag_matches = required_value.empty();
    }
    else
    {
        flag_matches = (it->second == required_value);
    }

    Logger::instance().log(
        std::format("[fomod] Flag dependency: flag=\"{}\" value=\"{}\" exists={} -> {}",
                    flag_name,
                    required_value,
                    flag_exists,
                    flag_matches ? "MET" : "NOT MET"));

    return flag_matches;
}

bool FomodDependencyEvaluator::evaluate_file_dependency(const pugi::xml_node& dep_node)
{
    std::string file_path = dep_node.attribute("file").as_string();
    std::string state = dep_node.attribute("state").as_string("Active");
    if (file_path.empty())
        return false;

    // Normalize for consistent lookup against the installed_files set
    // (which stores lowercase, forward-slash paths)
    std::string normalized_file = normalize_path(file_path);

    // Check if the file exists in any known location
    bool file_exists = false;
    if (context_)
    {
        if (context_->installed_files.count(normalized_file))
        {
            file_exists = true;
        }
        if (!file_exists && !context_->archive_root.empty())
        {
            auto full = fs::path(context_->archive_root) / file_path;
            if (fs::exists(full))
                file_exists = true;
        }
        if (!file_exists && !context_->game_path.empty())
        {
            auto full = fs::path(context_->game_path) / file_path;
            if (fs::exists(full))
                file_exists = true;
        }
    }

    bool result = false;
    if (state == "Missing")
    {
        // File must NOT exist anywhere
        result = !file_exists;
    }
    else if (state == "Inactive")
    {
        // File physically exists but is NOT active. For plugin files
        // (.esp/.esm/.esl), "active" means present in installed_plugins.
        // For non-plugin files, we cannot distinguish active/inactive
        // without MO2's VFS; default to NOT MET.
        if (file_exists && context_)
        {
            auto ext_str = fs::path(file_path).extension().string();
            std::string ext_lower;
            std::transform(ext_str.begin(),
                           ext_str.end(),
                           std::back_inserter(ext_lower),
                           [](unsigned char c) { return std::tolower(c); });
            bool is_plugin = (ext_lower == ".esp" || ext_lower == ".esm" || ext_lower == ".esl");
            if (is_plugin)
            {
                auto filename = fs::path(file_path).filename().string();
                std::string lower_name;
                std::transform(filename.begin(),
                               filename.end(),
                               std::back_inserter(lower_name),
                               [](unsigned char c) { return std::tolower(c); });
                bool is_active = context_->installed_plugins.count(lower_name) > 0;
                result = !is_active;  // Inactive = exists but not active
            }
            // Non-plugin files: result stays false (can't determine without VFS)
        }
    }
    else
    {  // "Active" (default)
        result = file_exists;
    }

    Logger::instance().log(
        std::format("[fomod] File dependency: file=\"{}\" state=\"{}\" exists={} -> {}",
                    file_path,
                    state,
                    file_exists,
                    result ? "MET" : "NOT MET"));
    return result;
}

bool FomodDependencyEvaluator::evaluate_game_dependency(const pugi::xml_node& dep_node)
{
    std::string version = dep_node.attribute("version").as_string();

    if (!context_ || context_->game_path.empty())
    {
        Logger::instance().log(std::format(
            "[fomod] Game dependency: version=\"{}\" -> MET (standalone mode, no game context)",
            version));
        return true;
    }

    if (!version.empty() && !context_->game_version.empty())
    {
        bool version_matches =
            compare_versions(context_->game_version, version, "GreaterThanOrEqual");
        Logger::instance().log(
            std::format("[fomod] Game dependency: version=\"{}\" (actual=\"{}\") -> {}",
                        version,
                        context_->game_version,
                        version_matches ? "MET" : "NOT MET"));
        return version_matches;
    }

    Logger::instance().log(std::format("[fomod] Game dependency: version=\"{}\" -> MET", version));
    return true;
}

bool FomodDependencyEvaluator::evaluate_plugin_dependency(const pugi::xml_node& dep_node)
{
    std::string plugin_name = dep_node.attribute("name").as_string();
    if (plugin_name.empty())
        return false;

    // Normalize to lowercase for comparison
    std::string lower_name;
    std::transform(plugin_name.begin(),
                   plugin_name.end(),
                   std::back_inserter(lower_name),
                   [](unsigned char c) { return std::tolower(c); });

    bool is_active = context_ && context_->installed_plugins.count(lower_name) > 0;

    // Check if plugin file exists on disk (for Inactive detection)
    bool file_exists = is_active;
    if (!file_exists && context_ && !context_->game_path.empty())
    {
        auto data_path = fs::path(context_->game_path) / "Data" / plugin_name;
        if (fs::exists(data_path))
            file_exists = true;
    }

    // Support Active/Inactive type attribute (default Active).
    std::string type = dep_node.attribute("type").as_string("Active");

    bool result = false;
    if (type == "Inactive")
    {
        result = file_exists && !is_active;
    }
    else
    {  // "Active"
        result = is_active;
    }

    Logger::instance().log(std::format(
        "[fomod] Plugin dependency: plugin=\"{}\" type=\"{}\" active={} exists={} -> {}",
        plugin_name,
        type,
        is_active,
        file_exists,
        result ? "MET" : "NOT MET"));
    return result;
}

bool FomodDependencyEvaluator::evaluate_fomod_dependency(const pugi::xml_node& dep_node)
{
    std::string fomod_name = dep_node.attribute("name").as_string();
    if (fomod_name.empty())
        return false;

    bool exists = context_ && context_->installed_fomods.count(fomod_name);
    Logger::instance().log(std::format(
        "[fomod] FOMOD dependency: fomod=\"{}\" -> {}", fomod_name, exists ? "MET" : "NOT MET"));
    return exists;
}

static int compare_version_parts(const std::vector<int>& x, const std::vector<int>& y)
{
    for (size_t i = 0; i < std::max(x.size(), y.size()); ++i)
    {
        int xv = i < x.size() ? x[i] : 0;
        int yv = i < y.size() ? y[i] : 0;
        if (xv < yv)
            return -1;
        if (xv > yv)
            return 1;
    }
    return 0;
}

static std::vector<int> parse_version_parts(const std::string& version_string)
{
    std::vector<int> parts;
    // Remove non-numeric/non-dot characters
    std::string cleaned;
    for (char c : version_string)
    {
        if (std::isdigit(c) || c == '.')
            cleaned += c;
    }
    std::istringstream ss(cleaned);
    std::string token;
    while (std::getline(ss, token, '.'))
    {
        try
        {
            parts.push_back(std::stoi(token));
        }
        catch (...)
        {
            parts.push_back(0);
        }
    }
    while (parts.size() < 3)
        parts.push_back(0);
    return parts;
}

bool FomodDependencyEvaluator::compare_versions(const std::string& actual,
                                                const std::string& required,
                                                const std::string& op)
{
    auto a = parse_version_parts(actual);
    auto r = parse_version_parts(required);

    int c = compare_version_parts(a, r);
    if (op == "Equal")
        return c == 0;
    if (op == "GreaterThan")
        return c > 0;
    if (op == "LessThan")
        return c < 0;
    if (op == "GreaterThanOrEqual")
        return c >= 0;
    if (op == "LessThanOrEqual")
        return c <= 0;
    return c == 0;
}

bool FomodDependencyEvaluator::evaluate_fomm_dependency(const pugi::xml_node& dep_node)
{
    // MO2 hardcodes FOMM version as "0.13.21" since it is not actually FOMM
    std::string required_version = dep_node.attribute("version").as_string();
    if (required_version.empty())
    {
        Logger::instance().log("[fomod] FOMM dependency: no version required -> MET");
        return true;
    }

    // MO2 uses <= comparison: required version must be <= actual version
    static const std::string fomm_version = "0.13.21";
    auto a = parse_version_parts(fomm_version);
    auto r = parse_version_parts(required_version);

    bool met = compare_version_parts(r, a) <= 0;  // required <= actual
    Logger::instance().log(
        std::format("[fomod] FOMM dependency: version=\"{}\" (actual=\"{}\") -> {}",
                    required_version,
                    fomm_version,
                    met ? "MET" : "NOT MET"));
    return met;
}

bool FomodDependencyEvaluator::evaluate_fose_dependency(const pugi::xml_node& dep_node)
{
    // Script extender version check - we don't have access to the actual SE version
    // in the standalone installer, so default to met (like most mods expect)
    std::string required_version = dep_node.attribute("version").as_string();

    if (!context_ || context_->game_version.empty())
    {
        Logger::instance().log(
            std::format("[fomod] FOSE/SKSE dependency: version=\"{}\" -> MET (no game context, "
                        "defaulting to met)",
                        required_version));
        return true;
    }

    Logger::instance().log(std::format(
        "[fomod] FOSE/SKSE dependency: version=\"{}\" -> MET (standalone mode)", required_version));
    return true;
}

// ---------------------------------------------------------------------------
// evaluate_condition: IR-based condition evaluation
// ---------------------------------------------------------------------------
bool evaluate_condition(const FomodCondition& condition,
                        const std::unordered_map<std::string, std::string>& flags,
                        const FomodDependencyContext* context)
{
    switch (condition.type)
    {
        case FomodConditionType::Composite:
        {
            bool result = (condition.op == FomodConditionOp::And);
            for (const auto& child : condition.children)
            {
                bool child_met = evaluate_condition(child, flags, context);
                if (condition.op == FomodConditionOp::And)
                    result = result && child_met;
                else
                    result = result || child_met;
            }
            return result;
        }
        case FomodConditionType::Flag:
        {
            auto it = flags.find(condition.flag_name);
            if (it == flags.end())
                return condition.flag_value.empty();
            return it->second == condition.flag_value;
        }
        case FomodConditionType::File:
        {
            if (condition.file_path.empty())
                return false;
            std::string normalized = normalize_path(condition.file_path);
            bool file_exists = false;
            if (context)
            {
                if (context->installed_files.count(normalized))
                    file_exists = true;
                if (!file_exists && !context->archive_root.empty())
                {
                    auto full = fs::path(context->archive_root) / condition.file_path;
                    if (fs::exists(full))
                        file_exists = true;
                }
                if (!file_exists && !context->game_path.empty())
                {
                    auto full = fs::path(context->game_path) / condition.file_path;
                    if (fs::exists(full))
                        file_exists = true;
                }
            }
            if (condition.file_state == "Missing")
                return !file_exists;
            if (condition.file_state == "Inactive")
            {
                if (file_exists && context)
                {
                    auto ext_str = fs::path(condition.file_path).extension().string();
                    auto ext_lower = to_lower(ext_str);
                    if (ext_lower == ".esp" || ext_lower == ".esm" || ext_lower == ".esl")
                    {
                        auto filename = fs::path(condition.file_path).filename().string();
                        auto lower_name = to_lower(filename);
                        return context->installed_plugins.count(lower_name) == 0;
                    }
                }
                return false;
            }
            return file_exists;  // Active (default)
        }
        case FomodConditionType::Game:
        {
            if (!context || context->game_path.empty())
                return true;  // standalone mode
            if (!condition.version.empty() && !context->game_version.empty())
            {
                // Use GreaterThanOrEqual comparison (same as XML evaluator)
                auto a = [](const std::string& v)
                {
                    std::vector<int> parts;
                    std::string cleaned;
                    for (char c : v)
                        if (std::isdigit(c) || c == '.')
                            cleaned += c;
                    std::istringstream ss(cleaned);
                    std::string token;
                    while (std::getline(ss, token, '.'))
                    {
                        try
                        {
                            parts.push_back(std::stoi(token));
                        }
                        catch (...)
                        {
                            parts.push_back(0);
                        }
                    }
                    while (parts.size() < 3)
                        parts.push_back(0);
                    return parts;
                };
                auto actual = a(context->game_version);
                auto required = a(condition.version);
                for (size_t i = 0; i < std::max(actual.size(), required.size()); ++i)
                {
                    int av = i < actual.size() ? actual[i] : 0;
                    int rv = i < required.size() ? required[i] : 0;
                    if (av > rv)
                        return true;
                    if (av < rv)
                        return false;
                }
                return true;  // equal
            }
            return true;
        }
        case FomodConditionType::Plugin:
        {
            if (condition.plugin_name.empty())
                return false;
            auto lower_name = to_lower(condition.plugin_name);
            bool is_active = context && context->installed_plugins.count(lower_name) > 0;
            bool file_exists = is_active;
            if (!file_exists && context && !context->game_path.empty())
            {
                auto data_path = fs::path(context->game_path) / "Data" / condition.plugin_name;
                if (fs::exists(data_path))
                    file_exists = true;
            }
            if (condition.plugin_type == "Inactive")
                return file_exists && !is_active;
            return is_active;
        }
        case FomodConditionType::Fomod:
        {
            if (condition.fomod_name.empty())
                return false;
            return context && context->installed_fomods.count(condition.fomod_name) > 0;
        }
        case FomodConditionType::Fomm:
        {
            if (condition.version.empty())
                return true;
            // Same as XML evaluator: required <= "0.13.21"
            return true;  // Almost always met
        }
        case FomodConditionType::Fose:
            return true;  // Standalone mode default
    }
    return true;
}

// ---------------------------------------------------------------------------
// evaluate_condition_inferred: inference-mode condition evaluation
// ---------------------------------------------------------------------------
bool evaluate_condition_inferred(const FomodCondition& condition,
                                 const std::unordered_map<std::string, std::string>& flags,
                                 ExternalConditionOverride external_override,
                                 const FomodDependencyContext* context)
{
    switch (condition.type)
    {
        case FomodConditionType::Composite:
        {
            bool result = (condition.op == FomodConditionOp::And);
            for (const auto& child : condition.children)
            {
                bool child_met =
                    evaluate_condition_inferred(child, flags, external_override, context);
                if (condition.op == FomodConditionOp::And)
                    result = result && child_met;
                else
                    result = result || child_met;
            }
            return result;
        }
        case FomodConditionType::Flag:
        {
            auto it = flags.find(condition.flag_name);
            if (it == flags.end())
                return condition.flag_value.empty();
            return it->second == condition.flag_value;
        }
        case FomodConditionType::File:
        case FomodConditionType::Plugin:
        case FomodConditionType::Fomod:
            if (external_override == ExternalConditionOverride::ForceTrue)
                return true;
            if (external_override == ExternalConditionOverride::ForceFalse)
                return false;
            // Unknown external state: do not over-constrain inference.
            return true;
        case FomodConditionType::Game:
        case FomodConditionType::Fomm:
        case FomodConditionType::Fose:
            return true;
    }
    return true;
}

// ---------------------------------------------------------------------------
// evaluate_plugin_type: resolve effective plugin type from type patterns
// ---------------------------------------------------------------------------
PluginType evaluate_plugin_type(const FomodPlugin& plugin,
                                const std::unordered_map<std::string, std::string>& flags,
                                const FomodDependencyContext* context)
{
    for (const auto& pattern : plugin.type_patterns)
    {
        bool matched = false;
        if (context)
        {
            matched = evaluate_condition(pattern.condition, flags, context);
        }
        else
        {
            matched = evaluate_condition_inferred(
                pattern.condition, flags, ExternalConditionOverride::Unknown, nullptr);
        }

        if (matched)
            return pattern.result_type;
    }
    return plugin.type;
}

}  // namespace mo2core
