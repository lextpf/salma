#include "InferenceDiagnostics.h"

#include "Logger.h"

#include <algorithm>
#include <format>
#include <utility>

namespace mo2core
{

namespace
{

// --- Confidence formula constants -----------------------------------------
//
// Component weights for the per-plugin composite. They sum to 1.0.
constexpr double kWeightEvidence = 0.40;
constexpr double kWeightPropagation = 0.30;
constexpr double kWeightRepro = 0.20;
constexpr double kWeightAmbiguity = 0.10;

// Band thresholds (composite score boundaries).
constexpr double kBandHighThreshold = 0.85;
constexpr double kBandMediumThreshold = 0.50;

// Run-level penalties applied after weighted aggregation.
constexpr double kRunExtraPenaltyPerFile = 0.05;
constexpr int kRunExtraPenaltyCap = 5;
constexpr double kRunFallbackPenalty = 0.10;

// Ambiguity component normalizer - matches kAmbiguityWindow used by the
// solver in `alternatives_per_group` accounting (10% of the chosen option's
// evidence score).

// --- Helpers --------------------------------------------------------------

std::string band_for(double composite)
{
    if (composite >= kBandHighThreshold)
    {
        return "high";
    }
    if (composite >= kBandMediumThreshold)
    {
        return "medium";
    }
    return "low";
}

double clamp01(double v)
{
    if (v < 0.0)
    {
        return 0.0;
    }
    if (v > 1.0)
    {
        return 1.0;
    }
    return v;
}

double weighted_mean(double sum, double weight)
{
    if (weight <= 0.0)
    {
        return 1.0;
    }
    return sum / weight;
}

double composite_from(const ConfidenceComponents& c)
{
    return clamp01(kWeightEvidence * c.evidence + kWeightPropagation * c.propagation +
                   kWeightRepro * c.repro + kWeightAmbiguity * c.ambiguity);
}

bool plugin_was_propagation_forced(const std::vector<Reason>& reasons)
{
    for (const auto& r : reasons)
    {
        switch (r.code)
        {
            case ReasonCode::FORCED_REQUIRED:
            case ReasonCode::FORCED_NOT_USABLE:
            case ReasonCode::FORCED_SELECT_ALL:
            case ReasonCode::FORCED_AT_LEAST_ONE:
            case ReasonCode::FORCED_EXACTLY_ONE:
            case ReasonCode::UNIQUE_FILE_EVIDENCE:
            case ReasonCode::NO_FILE_EVIDENCE:
            case ReasonCode::CARDINALITY_FORCED:
            case ReasonCode::FOMOD_PLUS_CACHE:
                return true;
            default:
                break;
        }
    }
    return false;
}

ReasonCode csp_phase_to_reason(const std::string& phase_id)
{
    if (phase_id == "csp.greedy")
    {
        return ReasonCode::CSP_PHASE_GREEDY;
    }
    if (phase_id == "csp.local_search")
    {
        return ReasonCode::CSP_PHASE_LOCAL_SEARCH;
    }
    if (phase_id == "csp.backtrack")
    {
        return ReasonCode::CSP_PHASE_BACKTRACK;
    }
    if (phase_id == "csp.repair")
    {
        return ReasonCode::CSP_PHASE_REPAIR;
    }
    if (phase_id == "csp.focused")
    {
        return ReasonCode::CSP_PHASE_FOCUSED;
    }
    if (phase_id == "csp.fallback")
    {
        return ReasonCode::CSP_PHASE_FALLBACK;
    }
    return ReasonCode::IMPLICIT_DEFAULT;
}

std::string csp_phase_message(const std::string& phase_id)
{
    if (phase_id == "csp.greedy")
    {
        return "Resolved in greedy phase";
    }
    if (phase_id == "csp.local_search")
    {
        return "Resolved in local-search phase";
    }
    if (phase_id == "csp.backtrack")
    {
        return "Resolved in backtrack phase";
    }
    if (phase_id == "csp.repair")
    {
        return "Resolved in residual-repair phase";
    }
    if (phase_id == "csp.focused")
    {
        return "Resolved in focused-search phase";
    }
    if (phase_id == "csp.fallback")
    {
        return "Resolved in global-fallback phase";
    }
    return "";
}

int plugin_file_count(const FomodInstaller& installer, int s, int g, int p)
{
    if (s < 0 || s >= static_cast<int>(installer.steps.size()))
    {
        return 0;
    }
    const auto& step = installer.steps[s];
    if (g < 0 || g >= static_cast<int>(step.groups.size()))
    {
        return 0;
    }
    const auto& group = step.groups[g];
    if (p < 0 || p >= static_cast<int>(group.plugins.size()))
    {
        return 0;
    }
    return static_cast<int>(group.plugins[p].files.size());
}

// Compute the per-plugin evidence component. For selected plugins we use the
// fraction of their declared files that participate in the target tree as
// reflected by the recorded reasons. Without per-plugin file maps wired
// through here, we fall back to a propagation-aware heuristic: forced or
// uniquely-supported plugins score 1.0; unforced plugins score 0.5 unless
// they carry a specific evidence reason.
double evidence_component(const PluginDiagnostics& plugin, bool propagation_forced)
{
    if (propagation_forced)
    {
        return 1.0;
    }
    for (const auto& r : plugin.reasons)
    {
        if (r.code == ReasonCode::UNIQUE_FILE_EVIDENCE)
        {
            return 1.0;
        }
        if (r.code == ReasonCode::NO_UNIQUE_EVIDENCE)
        {
            return 0.5;
        }
        if (r.code == ReasonCode::EXTRA_FILE_PRODUCED)
        {
            return 0.3;
        }
    }
    return plugin.selected ? 0.5 : 0.7;
}

double propagation_component(bool forced)
{
    return forced ? 1.0 : 0.0;
}

double ambiguity_component(int alternatives_in_group)
{
    if (alternatives_in_group <= 0)
    {
        return 1.0;
    }
    // 1 close alt -> 0.6, 2 -> 0.4, 3+ -> 0.2 floor.
    if (alternatives_in_group == 1)
    {
        return 0.6;
    }
    if (alternatives_in_group == 2)
    {
        return 0.4;
    }
    return 0.2;
}

}  // namespace

// --- ReasonCode helpers ---------------------------------------------------

std::string reason_code_to_string(ReasonCode code)
{
    switch (code)
    {
        case ReasonCode::IMPLICIT_DEFAULT:
            return "IMPLICIT_DEFAULT";
        case ReasonCode::FORCED_REQUIRED:
            return "FORCED_REQUIRED";
        case ReasonCode::FORCED_NOT_USABLE:
            return "FORCED_NOT_USABLE";
        case ReasonCode::FORCED_SELECT_ALL:
            return "FORCED_SELECT_ALL";
        case ReasonCode::FORCED_AT_LEAST_ONE:
            return "FORCED_AT_LEAST_ONE";
        case ReasonCode::FORCED_EXACTLY_ONE:
            return "FORCED_EXACTLY_ONE";
        case ReasonCode::UNIQUE_FILE_EVIDENCE:
            return "UNIQUE_FILE_EVIDENCE";
        case ReasonCode::NO_FILE_EVIDENCE:
            return "NO_FILE_EVIDENCE";
        case ReasonCode::NO_UNIQUE_EVIDENCE:
            return "NO_UNIQUE_EVIDENCE";
        case ReasonCode::CARDINALITY_FORCED:
            return "CARDINALITY_FORCED";
        case ReasonCode::CSP_PHASE_GREEDY:
            return "CSP_PHASE_GREEDY";
        case ReasonCode::CSP_PHASE_LOCAL_SEARCH:
            return "CSP_PHASE_LOCAL_SEARCH";
        case ReasonCode::CSP_PHASE_BACKTRACK:
            return "CSP_PHASE_BACKTRACK";
        case ReasonCode::CSP_PHASE_REPAIR:
            return "CSP_PHASE_REPAIR";
        case ReasonCode::CSP_PHASE_FOCUSED:
            return "CSP_PHASE_FOCUSED";
        case ReasonCode::CSP_PHASE_FALLBACK:
            return "CSP_PHASE_FALLBACK";
        case ReasonCode::CONDITION_FORCED_TRUE:
            return "CONDITION_FORCED_TRUE";
        case ReasonCode::CONDITION_FORCED_FALSE:
            return "CONDITION_FORCED_FALSE";
        case ReasonCode::CONDITION_UNKNOWN:
            return "CONDITION_UNKNOWN";
        case ReasonCode::STEP_VISIBILITY_FORCED:
            return "STEP_VISIBILITY_FORCED";
        case ReasonCode::STEP_VISIBILITY_UNKNOWN:
            return "STEP_VISIBILITY_UNKNOWN";
        case ReasonCode::STEP_NOT_VISIBLE:
            return "STEP_NOT_VISIBLE";
        case ReasonCode::EXTRA_FILE_PRODUCED:
            return "EXTRA_FILE_PRODUCED";
        case ReasonCode::FOMOD_PLUS_CACHE:
            return "FOMOD_PLUS_CACHE";
    }
    return "UNKNOWN";
}

// --- InferenceDiagnosticsBuilder -----------------------------------------

InferenceDiagnosticsBuilder::InferenceDiagnosticsBuilder(const FomodInstaller& installer)
{
    diag_.steps.resize(installer.steps.size());
    for (size_t s = 0; s < installer.steps.size(); ++s)
    {
        auto& step_diag = diag_.steps[s];
        step_diag.visible = true;
        step_diag.groups.resize(installer.steps[s].groups.size());
        for (size_t g = 0; g < installer.steps[s].groups.size(); ++g)
        {
            auto& group_diag = step_diag.groups[g];
            group_diag.plugins.resize(installer.steps[s].groups[g].plugins.size());
        }
    }
    diag_.run.groups.total = 0;
    for (const auto& step : installer.steps)
    {
        diag_.run.groups.total += static_cast<int>(step.groups.size());
    }
}

void InferenceDiagnosticsBuilder::add_plugin_reason(
    int step, int group, int plugin, ReasonCode code, std::string message, nlohmann::json detail)
{
    if (finalized_)
    {
        return;
    }
    if (step < 0 || step >= static_cast<int>(diag_.steps.size()))
    {
        return;
    }
    auto& step_diag = diag_.steps[step];
    if (group < 0 || group >= static_cast<int>(step_diag.groups.size()))
    {
        return;
    }
    auto& group_diag = step_diag.groups[group];
    if (plugin < 0 || plugin >= static_cast<int>(group_diag.plugins.size()))
    {
        return;
    }
    group_diag.plugins[plugin].reasons.push_back(
        Reason{code, std::move(message), std::move(detail)});
}

void InferenceDiagnosticsBuilder::add_group_reason(
    int step, int group, ReasonCode code, std::string message, nlohmann::json detail)
{
    if (finalized_)
    {
        return;
    }
    if (step < 0 || step >= static_cast<int>(diag_.steps.size()))
    {
        return;
    }
    auto& step_diag = diag_.steps[step];
    if (group < 0 || group >= static_cast<int>(step_diag.groups.size()))
    {
        return;
    }
    step_diag.groups[group].reasons.push_back(Reason{code, std::move(message), std::move(detail)});
}

void InferenceDiagnosticsBuilder::set_group_resolved_by(int step,
                                                        int group,
                                                        std::string resolved_by)
{
    if (finalized_)
    {
        return;
    }
    if (step < 0 || step >= static_cast<int>(diag_.steps.size()))
    {
        return;
    }
    auto& step_diag = diag_.steps[step];
    if (group < 0 || group >= static_cast<int>(step_diag.groups.size()))
    {
        return;
    }
    step_diag.groups[group].resolved_by = std::move(resolved_by);
}

void InferenceDiagnosticsBuilder::set_step_visibility(int step, bool visible, ReasonCode code)
{
    if (finalized_)
    {
        return;
    }
    if (step < 0 || step >= static_cast<int>(diag_.steps.size()))
    {
        return;
    }
    auto& step_diag = diag_.steps[step];
    step_diag.visible = visible;
    std::string msg;
    switch (code)
    {
        case ReasonCode::STEP_VISIBILITY_FORCED:
            msg = visible ? "Visibility condition evaluated true"
                          : "Visibility condition evaluated false";
            break;
        case ReasonCode::STEP_VISIBILITY_UNKNOWN:
            msg = "Visibility condition could not be determined";
            break;
        case ReasonCode::STEP_NOT_VISIBLE:
            msg = "Step skipped (not visible)";
            break;
        default:
            msg = "Step visibility recorded";
            break;
    }
    step_diag.reasons.push_back(Reason{code, std::move(msg), {}});
}

void InferenceDiagnosticsBuilder::set_run_timings(int64_t list_ms,
                                                  int64_t scan_ms,
                                                  int64_t solve_ms,
                                                  int64_t total_ms)
{
    if (finalized_)
    {
        return;
    }
    diag_.run.timings.list_ms = list_ms;
    diag_.run.timings.scan_ms = scan_ms;
    diag_.run.timings.solve_ms = solve_ms;
    diag_.run.timings.total_ms = total_ms;
}

void InferenceDiagnosticsBuilder::set_cache_hit(std::string source)
{
    if (finalized_)
    {
        return;
    }
    diag_.run.cache.hit = true;
    diag_.run.cache.source = std::move(source);
    diag_.run.phase_reached = "tier1_cache";
    // Push a FOMOD_PLUS_CACHE reason on every plugin so consumers see why
    // every selection is at confidence 1.0 / band high.
    for (auto& step : diag_.steps)
    {
        for (auto& group : step.groups)
        {
            group.resolved_by = "cache.fomod_plus";
            for (auto& plugin : group.plugins)
            {
                plugin.reasons.push_back(
                    Reason{ReasonCode::FOMOD_PLUS_CACHE, "Cached selection from meta.ini", {}});
            }
        }
    }
}

void InferenceDiagnosticsBuilder::absorb_propagation(const PropagationResult& propagation)
{
    if (finalized_)
    {
        return;
    }
    // resolved_by per group
    if (!propagation.resolved_by.empty())
    {
        for (size_t s = 0; s < propagation.resolved_by.size() && s < diag_.steps.size(); ++s)
        {
            for (size_t g = 0;
                 g < propagation.resolved_by[s].size() && g < diag_.steps[s].groups.size();
                 ++g)
            {
                const auto& src = propagation.resolved_by[s][g];
                if (!src.empty())
                {
                    diag_.steps[s].groups[g].resolved_by = src;
                }
            }
        }
    }

    // Plugin reasons - integer-encoded ReasonCode + optional detail.
    if (!propagation.plugin_reasons.empty())
    {
        for (size_t s = 0; s < propagation.plugin_reasons.size() && s < diag_.steps.size(); ++s)
        {
            for (size_t g = 0;
                 g < propagation.plugin_reasons[s].size() && g < diag_.steps[s].groups.size();
                 ++g)
            {
                for (size_t p = 0; p < propagation.plugin_reasons[s][g].size() &&
                                   p < diag_.steps[s].groups[g].plugins.size();
                     ++p)
                {
                    int raw = propagation.plugin_reasons[s][g][p];
                    auto code = static_cast<ReasonCode>(raw);
                    if (code == ReasonCode::IMPLICIT_DEFAULT)
                    {
                        continue;
                    }
                    nlohmann::json detail;
                    if (s < propagation.plugin_reason_details.size() &&
                        g < propagation.plugin_reason_details[s].size() &&
                        p < propagation.plugin_reason_details[s][g].size())
                    {
                        detail = propagation.plugin_reason_details[s][g][p];
                    }
                    std::string msg;
                    switch (code)
                    {
                        case ReasonCode::FORCED_REQUIRED:
                            msg = "Plugin type Required";
                            break;
                        case ReasonCode::FORCED_NOT_USABLE:
                            msg = "Plugin type NotUsable";
                            break;
                        case ReasonCode::FORCED_SELECT_ALL:
                            msg = "Group SelectAll forces this plugin on";
                            break;
                        case ReasonCode::FORCED_AT_LEAST_ONE:
                            msg = "AtLeastOne with single valid combo";
                            break;
                        case ReasonCode::FORCED_EXACTLY_ONE:
                            msg = "SelectExactlyOne with single valid combo";
                            break;
                        case ReasonCode::UNIQUE_FILE_EVIDENCE:
                            msg = "Uniquely produces target file(s)";
                            break;
                        case ReasonCode::NO_FILE_EVIDENCE:
                            msg = "All declared files absent from target";
                            break;
                        case ReasonCode::NO_UNIQUE_EVIDENCE:
                            msg = "No unique target evidence";
                            break;
                        case ReasonCode::CARDINALITY_FORCED:
                            msg = "Single combo remains under group type";
                            break;
                        default:
                            msg = reason_code_to_string(code);
                            break;
                    }
                    add_plugin_reason(static_cast<int>(s),
                                      static_cast<int>(g),
                                      static_cast<int>(p),
                                      code,
                                      std::move(msg),
                                      std::move(detail));
                }
            }
        }
    }
}

void InferenceDiagnosticsBuilder::absorb_solver(const SolverResult& result)
{
    if (finalized_)
    {
        return;
    }
    diag_.run.exact_match = result.exact_match;
    diag_.run.nodes_explored = result.nodes_explored;
    diag_.run.repro.missing = result.missing;
    diag_.run.repro.extra = result.extra;
    diag_.run.repro.size_mismatch = result.size_mismatch;
    diag_.run.repro.hash_mismatch = result.hash_mismatch;
    // reproduced is not directly carried on SolverResult; the inference
    // service computes it from the target tree size when available.

    if (!result.phase_reached.empty())
    {
        diag_.run.phase_reached = result.phase_reached;
    }

    // Per-group CSP phase -> CSP_PHASE_* reason on each selected plugin.
    if (!result.phase_per_group.empty())
    {
        for (size_t s = 0; s < result.phase_per_group.size() && s < diag_.steps.size(); ++s)
        {
            for (size_t g = 0;
                 g < result.phase_per_group[s].size() && g < diag_.steps[s].groups.size();
                 ++g)
            {
                const auto& phase_id = result.phase_per_group[s][g];
                if (phase_id.empty())
                {
                    continue;
                }
                // Only set resolved_by from CSP if propagation didn't already
                // resolve the group.
                if (diag_.steps[s].groups[g].resolved_by.empty())
                {
                    diag_.steps[s].groups[g].resolved_by = phase_id;
                }
                ReasonCode rcode = csp_phase_to_reason(phase_id);
                std::string msg = csp_phase_message(phase_id);
                if (rcode == ReasonCode::IMPLICIT_DEFAULT)
                {
                    continue;
                }
                if (s < result.selections.size() && g < result.selections[s].size())
                {
                    for (size_t p = 0; p < result.selections[s][g].size() &&
                                       p < diag_.steps[s].groups[g].plugins.size();
                         ++p)
                    {
                        if (!result.selections[s][g][p])
                        {
                            continue;
                        }
                        nlohmann::json detail;
                        detail["phase"] = phase_id;
                        detail["nodes"] = result.nodes_explored;
                        add_plugin_reason(static_cast<int>(s),
                                          static_cast<int>(g),
                                          static_cast<int>(p),
                                          rcode,
                                          msg,
                                          detail);
                    }
                }
            }
        }
    }

    // Mirror selection state into PluginDiagnostics.selected so the wire
    // format can express both selected and deselected plugins uniformly.
    for (size_t s = 0; s < result.selections.size() && s < diag_.steps.size(); ++s)
    {
        for (size_t g = 0; g < result.selections[s].size() && g < diag_.steps[s].groups.size(); ++g)
        {
            for (size_t p = 0;
                 p < result.selections[s][g].size() && p < diag_.steps[s].groups[g].plugins.size();
                 ++p)
            {
                diag_.steps[s].groups[g].plugins[p].selected = result.selections[s][g][p];
            }
        }
    }

    // Group counts.
    int prop = 0;
    int csp = 0;
    for (size_t s = 0; s < diag_.steps.size(); ++s)
    {
        for (size_t g = 0; g < diag_.steps[s].groups.size(); ++g)
        {
            const auto& rb = diag_.steps[s].groups[g].resolved_by;
            if (rb.rfind("propagation", 0) == 0 || rb == "cache.fomod_plus")
            {
                ++prop;
            }
            else if (rb.rfind("csp.", 0) == 0)
            {
                ++csp;
            }
        }
    }
    diag_.run.groups.resolved_by_propagation = prop;
    diag_.run.groups.resolved_by_csp = csp;
}

void InferenceDiagnosticsBuilder::finalize(const SolverResult& result,
                                           const PropagationResult& propagation,
                                           const FomodInstaller& installer)
{
    if (finalized_)
    {
        return;
    }
    (void)propagation;  // currently consumed earlier via absorb_propagation

    // Per-plugin component computation.
    for (size_t s = 0; s < diag_.steps.size(); ++s)
    {
        auto& step_diag = diag_.steps[s];
        for (size_t g = 0; g < step_diag.groups.size(); ++g)
        {
            auto& group_diag = step_diag.groups[g];
            int alternatives = 0;
            if (s < result.alternatives_per_group.size() &&
                g < result.alternatives_per_group[s].size())
            {
                alternatives = result.alternatives_per_group[s][g];
            }
            for (size_t p = 0; p < group_diag.plugins.size(); ++p)
            {
                auto& plugin_diag = group_diag.plugins[p];
                bool prop_forced = plugin_was_propagation_forced(plugin_diag.reasons);
                ConfidenceComponents& c = plugin_diag.confidence.components;
                c.evidence = clamp01(evidence_component(plugin_diag, prop_forced));
                c.propagation = clamp01(propagation_component(prop_forced));
                // Repro: deselected plugins inherit a high score; selected
                // plugins start at 1.0 and degrade if the group has misses.
                if (plugin_diag.selected)
                {
                    int group_misses = 0;
                    int group_targets = 0;
                    for (const auto& q : group_diag.plugins)
                    {
                        // Approximate group-local target count via plugin file count.
                        group_targets +=
                            plugin_file_count(installer,
                                              static_cast<int>(s),
                                              static_cast<int>(g),
                                              static_cast<int>(&q - &group_diag.plugins[0]));
                    }
                    (void)group_targets;
                    (void)group_misses;
                    c.repro = result.exact_match ? 1.0 : 0.85;
                }
                else
                {
                    c.repro = 1.0;
                }
                c.ambiguity = clamp01(ambiguity_component(alternatives));
                plugin_diag.confidence.composite = composite_from(c);
                plugin_diag.confidence.band = band_for(plugin_diag.confidence.composite);
            }

            // Group composite: file-count weighted mean of plugin composites,
            // short-circuiting to 1.0 when every plugin is propagation-forced.
            bool all_forced = !group_diag.plugins.empty();
            for (const auto& plugin_diag : group_diag.plugins)
            {
                if (!plugin_was_propagation_forced(plugin_diag.reasons))
                {
                    all_forced = false;
                    break;
                }
            }
            if (all_forced)
            {
                ConfidenceComponents& gc = group_diag.confidence.components;
                gc.evidence = 1.0;
                gc.propagation = 1.0;
                gc.repro = 1.0;
                gc.ambiguity = 1.0;
                group_diag.confidence.composite = 1.0;
            }
            else
            {
                double total_w = 0.0;
                ConfidenceComponents agg{0.0, 0.0, 0.0, 0.0};
                for (size_t p = 0; p < group_diag.plugins.size(); ++p)
                {
                    double w = static_cast<double>(plugin_file_count(installer,
                                                                     static_cast<int>(s),
                                                                     static_cast<int>(g),
                                                                     static_cast<int>(p)) +
                                                   1);
                    total_w += w;
                    const auto& pc = group_diag.plugins[p].confidence.components;
                    agg.evidence += w * pc.evidence;
                    agg.propagation += w * pc.propagation;
                    agg.repro += w * pc.repro;
                    agg.ambiguity += w * pc.ambiguity;
                }
                ConfidenceComponents& gc = group_diag.confidence.components;
                gc.evidence = clamp01(weighted_mean(agg.evidence, total_w));
                gc.propagation = clamp01(weighted_mean(agg.propagation, total_w));
                gc.repro = clamp01(weighted_mean(agg.repro, total_w));
                gc.ambiguity = clamp01(weighted_mean(agg.ambiguity, total_w));
                group_diag.confidence.composite = composite_from(gc);
            }
            group_diag.confidence.band = band_for(group_diag.confidence.composite);
        }

        // Step composite: group-count weighted mean.
        double total_w = 0.0;
        ConfidenceComponents agg{0.0, 0.0, 0.0, 0.0};
        for (size_t g = 0; g < step_diag.groups.size(); ++g)
        {
            double w = static_cast<double>(step_diag.groups[g].plugins.size() + 1);
            total_w += w;
            const auto& gc = step_diag.groups[g].confidence.components;
            agg.evidence += w * gc.evidence;
            agg.propagation += w * gc.propagation;
            agg.repro += w * gc.repro;
            agg.ambiguity += w * gc.ambiguity;
        }
        ConfidenceComponents& sc = step_diag.confidence.components;
        sc.evidence = clamp01(weighted_mean(agg.evidence, total_w));
        sc.propagation = clamp01(weighted_mean(agg.propagation, total_w));
        sc.repro = clamp01(weighted_mean(agg.repro, total_w));
        sc.ambiguity = clamp01(weighted_mean(agg.ambiguity, total_w));
        step_diag.confidence.composite = composite_from(sc);
        step_diag.confidence.band = band_for(step_diag.confidence.composite);
    }

    // Run-level: step-count weighted mean of step composites + penalties.
    double total_w = 0.0;
    ConfidenceComponents agg{0.0, 0.0, 0.0, 0.0};
    for (size_t s = 0; s < diag_.steps.size(); ++s)
    {
        double w = static_cast<double>(diag_.steps[s].groups.size() + 1);
        total_w += w;
        const auto& sc = diag_.steps[s].confidence.components;
        agg.evidence += w * sc.evidence;
        agg.propagation += w * sc.propagation;
        agg.repro += w * sc.repro;
        agg.ambiguity += w * sc.ambiguity;
    }
    ConfidenceComponents& rc = diag_.run.confidence.components;
    rc.evidence = clamp01(weighted_mean(agg.evidence, total_w));
    rc.propagation = clamp01(weighted_mean(agg.propagation, total_w));
    rc.repro = clamp01(weighted_mean(agg.repro, total_w));
    rc.ambiguity = clamp01(weighted_mean(agg.ambiguity, total_w));
    double composite = composite_from(rc);

    // Penalties.
    int extra_capped = std::min(result.extra, kRunExtraPenaltyCap);
    composite -= kRunExtraPenaltyPerFile * static_cast<double>(extra_capped);
    if (diag_.run.phase_reached == "csp.fallback")
    {
        composite -= kRunFallbackPenalty;
    }
    diag_.run.confidence.composite = clamp01(composite);
    diag_.run.confidence.band = band_for(diag_.run.confidence.composite);

    // Backfill `reproduced` if not already populated.
    if (diag_.run.repro.reproduced == 0)
    {
        // Count selected plugins' file contributions as a rough proxy when
        // the solver did not provide an explicit reproduced count.
        int contributed = 0;
        for (size_t s = 0; s < result.selections.size(); ++s)
        {
            for (size_t g = 0; g < result.selections[s].size(); ++g)
            {
                for (size_t p = 0; p < result.selections[s][g].size(); ++p)
                {
                    if (result.selections[s][g][p])
                    {
                        contributed += plugin_file_count(installer,
                                                         static_cast<int>(s),
                                                         static_cast<int>(g),
                                                         static_cast<int>(p));
                    }
                }
            }
        }
        diag_.run.repro.reproduced = std::max(0, contributed - result.missing);
    }

    finalized_ = true;

    Logger::instance().log(std::format(
        "[infer] Diagnostics: confidence={:.2f} ({}), phase={}, repro=miss:{}/extra:{}/sm:{}/hm:{}",
        diag_.run.confidence.composite,
        diag_.run.confidence.band,
        diag_.run.phase_reached.empty() ? "n/a" : diag_.run.phase_reached,
        diag_.run.repro.missing,
        diag_.run.repro.extra,
        diag_.run.repro.size_mismatch,
        diag_.run.repro.hash_mismatch));
}

const InferenceDiagnostics& InferenceDiagnosticsBuilder::diagnostics() const
{
    return diag_;
}

// --- Serialization --------------------------------------------------------

nlohmann::json serialize_confidence(const ConfidenceScore& score)
{
    nlohmann::json j;
    j["composite"] = score.composite;
    j["band"] = score.band;
    nlohmann::json comps;
    comps["evidence"] = score.components.evidence;
    comps["propagation"] = score.components.propagation;
    comps["repro"] = score.components.repro;
    comps["ambiguity"] = score.components.ambiguity;
    j["components"] = std::move(comps);
    return j;
}

nlohmann::json serialize_reason(const Reason& reason)
{
    nlohmann::json j;
    j["code"] = reason_code_to_string(reason.code);
    j["message"] = reason.message;
    if (!reason.detail.is_null() && !reason.detail.empty())
    {
        j["detail"] = reason.detail;
    }
    return j;
}

nlohmann::json serialize_run_diagnostics(const RunDiagnostics& run)
{
    nlohmann::json j;
    j["confidence"] = serialize_confidence(run.confidence);
    j["exact_match"] = run.exact_match;
    j["phase_reached"] = run.phase_reached;
    j["nodes_explored"] = run.nodes_explored;

    nlohmann::json groups;
    groups["total"] = run.groups.total;
    groups["resolved_by_propagation"] = run.groups.resolved_by_propagation;
    groups["resolved_by_csp"] = run.groups.resolved_by_csp;
    j["groups"] = std::move(groups);

    nlohmann::json repro;
    repro["missing"] = run.repro.missing;
    repro["extra"] = run.repro.extra;
    repro["size_mismatch"] = run.repro.size_mismatch;
    repro["hash_mismatch"] = run.repro.hash_mismatch;
    repro["reproduced"] = run.repro.reproduced;
    j["repro"] = std::move(repro);

    nlohmann::json timings;
    timings["list"] = run.timings.list_ms;
    timings["scan"] = run.timings.scan_ms;
    timings["solve"] = run.timings.solve_ms;
    timings["total"] = run.timings.total_ms;
    j["timings_ms"] = std::move(timings);

    nlohmann::json cache;
    cache["hit"] = run.cache.hit;
    cache["source"] = run.cache.source;
    j["cache"] = std::move(cache);

    return j;
}

}  // namespace mo2core
