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

// ---------------------------------------------------------------------------
// Version helpers (file-scoped, used by both the anonymous namespace helpers
// and the FomodDependencyEvaluator class methods).
// ---------------------------------------------------------------------------

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
        catch (const std::exception& ex)
        {
            mo2core::Logger::instance().log_warning(
                std::format("[fomod] Malformed version component \"{}\": {}", token, ex.what()));
            parts.push_back(0);
        }
    }
    while (parts.size() < 3)
        parts.push_back(0);
    return parts;
}

// ---------------------------------------------------------------------------
// Shared helpers: core evaluation logic for dependency types.
// Used by both the XML-based class methods and the IR-based static functions.
// Callers are responsible for logging.
// ---------------------------------------------------------------------------

namespace
{

// Non-throwing wrapper around fs::exists.  The std::error_code overload
// never throws filesystem_error, so callers can use this in paths where
// an I/O failure (permissions, etc.) should simply yield "does not exist".
static bool safe_exists(const fs::path& p) noexcept
{
    std::error_code ec;
    return fs::exists(p, ec);
}

bool eval_file_dep(const std::string& file_path,
                   const std::string& state,
                   const mo2core::FomodDependencyContext* ctx)
{
    if (file_path.empty())
        return false;

    std::string normalized = mo2core::normalize_path(file_path);

    bool file_exists = false;
    if (ctx)
    {
        if (ctx->installed_files.count(normalized))
            file_exists = true;
        auto ext_lower = mo2core::to_lower(fs::path(file_path).extension().string());
        bool is_plugin_file = (ext_lower == ".esp" || ext_lower == ".esm" || ext_lower == ".esl");
        if (!file_exists && is_plugin_file)
        {
            auto lower_name = mo2core::to_lower(fs::path(file_path).filename().string());
            if (ctx->installed_plugins.count(lower_name))
                file_exists = true;
        }
        if (!file_exists && !ctx->archive_root.empty())
        {
            auto full = fs::path(ctx->archive_root) / normalized;
            if (safe_exists(full))
                file_exists = true;
        }
        if (!file_exists && !ctx->game_path.empty())
        {
            auto full = fs::path(ctx->game_path) / normalized;
            if (safe_exists(full))
                file_exists = true;
        }
    }

    if (state == "Missing")
        return !file_exists;

    if (state == "Inactive")
    {
        // "Inactive" semantics: for plugin files (.esp/.esm/.esl), check
        // whether the file exists but is NOT in the active plugin list.
        // For non-plugin files, FOMOD has no standard "Inactive" meaning,
        // so we conservatively return !file_exists (treat as "Missing").
        if (file_exists && ctx)
        {
            auto ext_lower = mo2core::to_lower(fs::path(file_path).extension().string());
            if (ext_lower == ".esp" || ext_lower == ".esm" || ext_lower == ".esl")
            {
                auto lower_name = mo2core::to_lower(fs::path(file_path).filename().string());
                return ctx->installed_plugins.count(lower_name) == 0;
            }
        }
        return !file_exists;
    }

    // "Active" (default)
    if (state != "Active")
    {
        mo2core::Logger::instance().log_warning("Unknown file dependency state: " + state +
                                                " for file: " + file_path + ", treating as Active");
    }
    return file_exists;
}

bool eval_game_dep(const std::string& version, const mo2core::FomodDependencyContext* ctx)
{
    if (!ctx || ctx->game_path.empty())
        return true;  // standalone mode

    if (!version.empty() && !ctx->game_version.empty())
    {
        return compare_version_parts(parse_version_parts(ctx->game_version),
                                     parse_version_parts(version)) >= 0;
    }

    return true;
}

struct PluginDepResult
{
    bool met;
    bool is_active;
    bool file_exists;
};

PluginDepResult eval_plugin_dep(const std::string& plugin_name,
                                const std::string& type,
                                const mo2core::FomodDependencyContext* ctx)
{
    if (plugin_name.empty())
        return {false, false, false};

    auto lower_name = mo2core::to_lower(plugin_name);
    bool is_active = ctx && ctx->installed_plugins.count(lower_name) > 0;

    bool file_exists = is_active;
    if (!file_exists && ctx && !ctx->game_path.empty())
    {
        auto data_path = fs::path(ctx->game_path) / "Data" / plugin_name;
        if (safe_exists(data_path))
            file_exists = true;
    }

    bool result;
    if (type == "Inactive")
        result = file_exists && !is_active;
    else
        result = is_active;  // "Active" (default)

    return {result, is_active, file_exists};
}

bool eval_fomod_dep(const std::string& fomod_name, const mo2core::FomodDependencyContext* ctx)
{
    if (fomod_name.empty())
        return false;
    return ctx && ctx->installed_fomods.count(fomod_name) > 0;
}

}  // anonymous namespace

namespace mo2core
{

FomodDependencyEvaluator::FomodDependencyEvaluator(
    std::unordered_map<std::string, std::string>& plugin_flags,
    const FomodDependencyContext* context)
    : plugin_flags_(plugin_flags),
      context_(context)
{
}

bool FomodDependencyEvaluator::are_dependencies_met(const pugi::xml_node& dependencies_node,
                                                    int depth)
{
    if (!dependencies_node)
        return true;

    if (depth > MAX_DEPENDENCY_DEPTH)
    {
        Logger::instance().log_warning(
            "[fomod] Maximum dependency recursion depth exceeded; treating as unmet (not a "
            "definitive evaluation -- XML may be malformed or maliciously crafted)");
        return false;
    }

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
                dep_met = are_dependencies_met(dep_node, depth + 1);
                break;
            default:
                recognized = false;
                break;
        }

        // Unknown nodes are treated as neutral (no-op) to avoid short-circuiting
        // Or-conditions to true. Deduplicate warnings to avoid log flooding on
        // large XMLs with many repeated unknown node types.
        if (!recognized)
        {
            Logger::instance().log_warning(
                std::format("[fomod] Unknown dependency node \"{}\" ignored", name));
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

    return flag_matches;
}

bool FomodDependencyEvaluator::evaluate_file_dependency(const pugi::xml_node& dep_node)
{
    std::string file_path = dep_node.attribute("file").as_string();
    std::string state = dep_node.attribute("state").as_string("Active");

    bool result = eval_file_dep(file_path, state, context_);
    return result;
}

bool FomodDependencyEvaluator::evaluate_game_dependency(const pugi::xml_node& dep_node)
{
    std::string version = dep_node.attribute("version").as_string();

    bool result = eval_game_dep(version, context_);
    return result;
}

bool FomodDependencyEvaluator::evaluate_plugin_dependency(const pugi::xml_node& dep_node)
{
    std::string plugin_name = dep_node.attribute("name").as_string();
    std::string type = dep_node.attribute("type").as_string("Active");

    auto pdr = eval_plugin_dep(plugin_name, type, context_);

    return pdr.met;
}

bool FomodDependencyEvaluator::evaluate_fomod_dependency(const pugi::xml_node& dep_node)
{
    std::string fomod_name = dep_node.attribute("name").as_string();

    bool result = eval_fomod_dep(fomod_name, context_);

    return result;
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
    // Intentional no-op: Salma does not have access to the Script Extender
    // runtime version, so this dependency always evaluates to met. This matches
    // MO2 behavior (which also does not evaluate foseDependency version checks).
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
// IR-based condition evaluation
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// evaluate_condition_core: shared Composite/Flag handling, delegates leaf
// types to a caller-supplied strategy.
// ---------------------------------------------------------------------------
template <typename LeafEval>
    requires std::invocable<LeafEval, const FomodCondition&>
static constexpr bool evaluate_condition_core(
    const FomodCondition& condition,
    const std::unordered_map<std::string, std::string>& flags,
    const LeafEval& eval_leaf,
    int depth = 0)
{
    using enum FomodConditionType;
    switch (condition.type)
    {
        case Composite:
        {
            if (depth > FomodDependencyEvaluator::MAX_DEPENDENCY_DEPTH)
            {
                Logger::instance().log_warning(
                    "[fomod-ir] Condition tree exceeds maximum depth, treating as unmet");
                return false;
            }
            bool is_and = (condition.op == FomodConditionOp::And);
            bool result = is_and;
            for (const auto& child : condition.children)
            {
                bool child_met = evaluate_condition_core(child, flags, eval_leaf, depth + 1);
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
// evaluate_condition: IR-based condition evaluation
// ---------------------------------------------------------------------------
bool evaluate_condition(const FomodCondition& condition,
                        const std::unordered_map<std::string, std::string>& flags,
                        const FomodDependencyContext* context)
{
    return evaluate_condition_core(
        condition,
        flags,
        [&](const FomodCondition& c) -> bool
        {
            using enum FomodConditionType;
            bool result;
            switch (c.type)
            {
                case File:
                    result = eval_file_dep(c.file_path, c.file_state, context);
                    break;
                case Game:
                    result = eval_game_dep(c.version, context);
                    break;
                case Plugin:
                    result = eval_plugin_dep(c.plugin_name, c.plugin_type, context).met;
                    break;
                case Fomod:
                    result = eval_fomod_dep(c.fomod_name, context);
                    break;
                case Fomm:
                {
                    // Replicate XML-based FOMM version comparison (MO2 hardcodes "0.13.21")
                    if (c.version.empty())
                    {
                        result = true;
                    }
                    else
                    {
                        static const std::string fomm_version = "0.13.21";
                        auto a = parse_version_parts(fomm_version);
                        auto r = parse_version_parts(c.version);
                        result = compare_version_parts(r, a) <= 0;  // required <= actual
                    }
                    break;
                }
                case Fose:
                    result = true;
                    break;
                default:
                    result = true;
                    break;
            }
            return result;
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
                    // Conservative default -- do not assume unknown external conditions are met
                    return false;
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
