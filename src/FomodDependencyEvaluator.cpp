#include "FomodDependencyEvaluator.h"
#include <algorithm>
#include <concepts>
#include <filesystem>
#include <format>
#include <sstream>
#include "Logger.h"
#include "Utils.h"

using namespace std::string_view_literals;

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

        std::string_view name = dep_node.name();
        bool dep_met = false;
        bool recognized = true;

        switch (fnv1a_hash(name.data(), name.size()))
        {
            case "flagDependency"_h:
                dep_met = evaluate_flag_dependency(dep_node);
                break;
            case "fileDependency"_h:
                dep_met = evaluate_file_dependency(dep_node);
                break;
            case "gameDependency"_h:
                dep_met = evaluate_game_dependency(dep_node);
                break;
            case "pluginDependency"_h:
                dep_met = evaluate_plugin_dependency(dep_node);
                break;
            case "fomodDependency"_h:
                dep_met = evaluate_fomod_dependency(dep_node);
                break;
            case "fommDependency"_h:
                dep_met = evaluate_fomm_dependency(dep_node);
                break;
            case "foseDependency"_h:
                dep_met = evaluate_fose_dependency(dep_node);
                break;
            // MO2: nested <dependencies> creates a SubCondition (recursive composite)
            case "dependencies"_h:
                dep_met = are_dependencies_met(dep_node);
                break;
            default:
                recognized = false;
                break;
        }

        // Unknown nodes are treated as neutral (no-op) to avoid short-circuiting
        // Or-conditions to true.
        if (!recognized)
        {
            Logger::instance().log(
                std::format("[fomod] WARNING: Unknown dependency node \"{}\" ignored", name));
            continue;
        }

        if (op_type == "And")
        {
            result = result && dep_met;
            if (!result)
                return false;  // Short-circuit: And already failed
        }
        else if (op_type == "Or")
        {
            result = result || dep_met;
            if (result)
                return true;  // Short-circuit: Or already satisfied
        }
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
            auto ext_lower = to_lower(fs::path(file_path).extension().string());
            bool is_plugin = (ext_lower == ".esp" || ext_lower == ".esm" || ext_lower == ".esl");
            if (is_plugin)
            {
                auto lower_name = to_lower(fs::path(file_path).filename().string());
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
            compare_versions(context_->game_version, version, "GreaterThanOrEqual").value_or(false);
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
    auto lower_name = to_lower(plugin_name);

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

Result<bool> FomodDependencyEvaluator::compare_versions(const std::string& actual,
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
    return std::unexpected(std::format("Unknown version operator: \"{}\"", op));
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
// evaluate_condition_core: shared Composite/Flag handling, delegates leaf
// types to a caller-supplied strategy.
// ---------------------------------------------------------------------------
template <typename LeafEval>
    requires std::invocable<LeafEval, const FomodCondition&>
static constexpr bool evaluate_condition_core(
    const FomodCondition& condition,
    const std::unordered_map<std::string, std::string>& flags,
    const LeafEval& eval_leaf)
{
    using enum FomodConditionType;
    switch (condition.type)
    {
        case Composite:
        {
            bool is_and = (condition.op == FomodConditionOp::And);
            bool result = is_and;
            for (const auto& child : condition.children)
            {
                bool child_met = evaluate_condition_core(child, flags, eval_leaf);
                if (is_and)
                {
                    result = result && child_met;
                    if (!result)
                        return false;  // Short-circuit
                }
                else
                {
                    result = result || child_met;
                    if (result)
                        return true;  // Short-circuit
                }
            }
            return result;
        }
        case Flag:
        {
            auto it = flags.find(condition.flag_name);
            if (it == flags.end())
                return condition.flag_value.empty();
            return it->second == condition.flag_value;
        }
        default:
            return eval_leaf(condition);
    }
}

// ---------------------------------------------------------------------------
// Named leaf evaluators for IR-based condition evaluation
// ---------------------------------------------------------------------------
static bool eval_file_condition(const FomodCondition& c, const FomodDependencyContext* context)
{
    if (c.file_path.empty())
        return false;
    std::string normalized = normalize_path(c.file_path);
    bool file_exists = false;
    if (context)
    {
        if (context->installed_files.count(normalized))
            file_exists = true;
        if (!file_exists && !context->archive_root.empty())
        {
            auto full = fs::path(context->archive_root) / c.file_path;
            if (fs::exists(full))
                file_exists = true;
        }
        if (!file_exists && !context->game_path.empty())
        {
            auto full = fs::path(context->game_path) / c.file_path;
            if (fs::exists(full))
                file_exists = true;
        }
    }
    if (c.file_state == "Missing")
        return !file_exists;
    if (c.file_state == "Inactive")
    {
        if (file_exists && context)
        {
            auto ext_lower = to_lower(fs::path(c.file_path).extension().string());
            if (ext_lower == ".esp" || ext_lower == ".esm" || ext_lower == ".esl")
            {
                auto lower_name = to_lower(fs::path(c.file_path).filename().string());
                return context->installed_plugins.count(lower_name) == 0;
            }
        }
        return false;
    }
    return file_exists;  // Active (default)
}

static bool eval_game_condition(const FomodCondition& c, const FomodDependencyContext* context)
{
    if (!context || context->game_path.empty())
        return true;  // standalone mode
    if (!c.version.empty() && !context->game_version.empty())
    {
        return compare_version_parts(parse_version_parts(context->game_version),
                                     parse_version_parts(c.version)) >= 0;
    }
    return true;
}

static bool eval_plugin_condition(const FomodCondition& c, const FomodDependencyContext* context)
{
    if (c.plugin_name.empty())
        return false;
    auto lower_name = to_lower(c.plugin_name);
    bool is_active = context && context->installed_plugins.count(lower_name) > 0;
    bool file_exists = is_active;
    if (!file_exists && context && !context->game_path.empty())
    {
        auto data_path = fs::path(context->game_path) / "Data" / c.plugin_name;
        if (fs::exists(data_path))
            file_exists = true;
    }
    if (c.plugin_type == "Inactive")
        return file_exists && !is_active;
    return is_active;
}

static bool eval_fomod_condition(const FomodCondition& c, const FomodDependencyContext* context)
{
    if (c.fomod_name.empty())
        return false;
    return context && context->installed_fomods.count(c.fomod_name) > 0;
}

// ---------------------------------------------------------------------------
// evaluate_condition: IR-based condition evaluation
// ---------------------------------------------------------------------------
bool evaluate_condition(const FomodCondition& condition,
                        const std::unordered_map<std::string, std::string>& flags,
                        const FomodDependencyContext* context)
{
    return evaluate_condition_core(condition,
                                   flags,
                                   [&](const FomodCondition& c) -> bool
                                   {
                                       using enum FomodConditionType;
                                       switch (c.type)
                                       {
                                           case File:
                                               return eval_file_condition(c, context);
                                           case Game:
                                               return eval_game_condition(c, context);
                                           case Plugin:
                                               return eval_plugin_condition(c, context);
                                           case Fomod:
                                               return eval_fomod_condition(c, context);
                                           case Fomm:
                                               [[fallthrough]];
                                           case Fose:
                                               return true;
                                           default:
                                               return true;
                                       }
                                   });
}

// ---------------------------------------------------------------------------
// evaluate_condition_inferred: inference-mode condition evaluation
// ---------------------------------------------------------------------------
bool evaluate_condition_inferred(const FomodCondition& condition,
                                 const std::unordered_map<std::string, std::string>& flags,
                                 ExternalConditionOverride external_override,
                                 const FomodDependencyContext* context)
{
    return evaluate_condition_core(
        condition,
        flags,
        [&](const FomodCondition& c) -> bool
        {
            using enum FomodConditionType;
            switch (c.type)
            {
                case File:
                case Plugin:
                case Fomod:
                    if (external_override == ExternalConditionOverride::ForceTrue)
                        return true;
                    if (external_override == ExternalConditionOverride::ForceFalse)
                        return false;
                    // Unknown external state: do not over-constrain inference.
                    return true;
                case Game:
                case Fomm:
                case Fose:
                    return true;
                default:
                    return true;
            }
        });
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
