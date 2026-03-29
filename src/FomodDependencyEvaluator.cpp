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

// ---------------------------------------------------------------------------
// IR-based condition evaluation
// ---------------------------------------------------------------------------

/// Evaluation mode tag: selects compile-time behaviour in LeafEvaluator.
enum class EvalMode : uint8_t
{
    Normal,   ///< Full evaluation against real dependency context.
    Inferred  ///< Inference mode: external deps follow override, infra deps -> true.
};

// ---------------------------------------------------------------------------
// LeafEvaluator<Mode>: single dispatch point for all leaf condition types.
//
// The switch over FomodConditionType exists exactly once; per-type behaviour
// is selected at compile time via `if constexpr` on the Mode parameter.
// ---------------------------------------------------------------------------
template <EvalMode Mode>
struct LeafEvaluator
{
    const FomodDependencyContext* ctx = nullptr;
    ExternalConditionOverride external_override = ExternalConditionOverride::Unknown;

    bool operator()(const FomodCondition& c) const
    {
        using enum FomodConditionType;
        switch (c.type)
        {
            case File:
                if constexpr (Mode == EvalMode::Normal)
                    return eval_file_dep(c.file_path, c.file_state, ctx);
                else
                    return apply_external_override();

            case Game:
                if constexpr (Mode == EvalMode::Normal)
                    return eval_game_dep(c.version, ctx);
                else
                    return true;

            case Plugin:
                if constexpr (Mode == EvalMode::Normal)
                    return eval_plugin_dep(c.plugin_name, c.plugin_type, ctx).met;
                else
                    return apply_external_override();

            case Fomod:
                if constexpr (Mode == EvalMode::Normal)
                    return eval_fomod_dep(c.fomod_name, ctx);
                else
                    return apply_external_override();

            case Fomm:
                if constexpr (Mode == EvalMode::Normal)
                    return eval_fomm_version(c.version);
                else
                    return true;

            case Fose:
                return true;

            default:
                return true;
        }
    }

private:
    /// Resolve an external condition (File/Plugin/Fomod) via the override mode.
    bool apply_external_override() const
    {
        return external_override == ExternalConditionOverride::ForceTrue;
    }

    /// FOMM version comparison (MO2 hardcodes "0.13.21").
    static bool eval_fomm_version(const std::string& version)
    {
        if (version.empty())
            return true;
        static const std::string fomm_version = "0.13.21";
        auto a = parse_version_parts(fomm_version);
        auto r = parse_version_parts(version);
        return compare_version_parts(r, a) <= 0;  // required <= actual
    }
};

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
            if (depth > MAX_DEPENDENCY_DEPTH)
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
        condition, flags, LeafEvaluator<EvalMode::Normal>{.ctx = context});
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
        LeafEvaluator<EvalMode::Inferred>{.ctx = context, .external_override = external_override});
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
