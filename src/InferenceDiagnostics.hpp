#pragma once

#include "Export.h"
#include "FomodCSPSolver.h"
#include "FomodCSPTypes.h"
#include "FomodIR.h"
#include "FomodPropagator.h"

#include <nlohmann/json.hpp>

#include <cstdint>
#include <string>
#include <vector>

namespace mo2core
{

/**
 * @brief Stable enumeration of reasons the inference engine can attach to a
 *        plugin, group, or step decision.
 * @author Alex (https://github.com/lextpf)
 * @ingroup FomodService
 *
 * Codes are integer-backed and stable across releases - the dashboard maps
 * them to UI labels and never compares against the human `message`. New codes
 * are appended; existing codes never change their integer value or semantics.
 *
 * Codes are grouped by the kind of decision that produced them:
 *
 * - `FORCED_*` come from FOMOD spec constraints (Required, NotUsable, etc.)
 *   evaluated by `FomodPropagator` rule P.
 * - `*_FILE_EVIDENCE` / `NO_FILE_EVIDENCE` come from rule F (target-tree
 *   evidence comparison).
 * - `CARDINALITY_FORCED` comes from rule C (group-type cardinality check).
 * - `CSP_PHASE_*` indicate which phase of `solve_fomod_csp` made the pick.
 * - `CONDITION_*` and `STEP_*` describe override and visibility decisions.
 * - `FOMOD_PLUS_CACHE` is set when inference was lifted from a cached
 *   `meta.ini` Tier-1 entry.
 * - `IMPLICIT_DEFAULT` is the catch-all when no other reason applies.
 *
 * @see InferenceDiagnostics, mo2core::FomodPropagator
 */
enum class ReasonCode : int
{
    /// No explicit reason recorded yet (default for unfilled entries).
    IMPLICIT_DEFAULT = 0,

    // Forced by plugin-type constraints (Rule P)
    FORCED_REQUIRED = 100,      ///< Plugin type Required pinned the selection.
    FORCED_NOT_USABLE = 101,    ///< Plugin type NotUsable eliminated the selection.
    FORCED_SELECT_ALL = 102,    ///< SelectAll group forced this plugin on.
    FORCED_AT_LEAST_ONE = 103,  ///< AtLeastOne group with one valid combo.
    FORCED_EXACTLY_ONE = 104,   ///< SelectExactlyOne with one valid combo.

    // File-evidence rules (Rule F)
    UNIQUE_FILE_EVIDENCE = 200,  ///< Plugin uniquely produces a target file; detail lists files.
    NO_FILE_EVIDENCE = 201,      ///< All plugin files absent from target; eliminated.
    NO_UNIQUE_EVIDENCE = 202,    ///< Deselected: nothing in target uniquely maps here.

    // Cardinality rules (Rule C)
    CARDINALITY_FORCED = 300,  ///< Group narrowed to a single combo by group type.

    // CSP solver phases
    CSP_PHASE_GREEDY = 400,        ///< Picked by the greedy phase.
    CSP_PHASE_LOCAL_SEARCH = 401,  ///< Improved/picked by local search.
    CSP_PHASE_BACKTRACK = 402,     ///< Picked by systematic backtracking.
    CSP_PHASE_REPAIR = 403,        ///< Picked by the residual-repair phase.
    CSP_PHASE_FOCUSED = 404,       ///< Picked by the focused-search phase.
    CSP_PHASE_FALLBACK = 405,      ///< Picked by the global-fallback phase.

    // Condition / step visibility overrides
    CONDITION_FORCED_TRUE = 500,    ///< compute_overrides forced a conditional pattern true.
    CONDITION_FORCED_FALSE = 501,   ///< compute_overrides forced it false.
    CONDITION_UNKNOWN = 502,        ///< Could not determine; the simulator guessed.
    STEP_VISIBILITY_FORCED = 510,   ///< Step visibility condition forced true.
    STEP_VISIBILITY_UNKNOWN = 511,  ///< Could not determine step visibility.
    STEP_NOT_VISIBLE = 512,         ///< Step skipped entirely because not visible.

    // Penalties / scoring
    EXTRA_FILE_PRODUCED = 600,  ///< Selection produces a file not in target (penalty).

    // Cache / shortcut
    FOMOD_PLUS_CACHE = 700,  ///< Selection lifted from meta.ini Tier-1 cache.
};

/**
 * @brief Convert a `ReasonCode` to its stable string name (e.g. "FORCED_REQUIRED").
 *
 * The returned string is the enum-identifier text, suitable for the JSON wire
 * format and for grepping logs. Returns `"UNKNOWN"` when @p code is not a
 * recognized enumerator.
 */
MO2_API std::string reason_code_to_string(ReasonCode code);

/**
 * @brief A single justification attached to a plugin / group / step decision.
 *
 * Reasons accumulate in evaluation order to form a chain (e.g. "Required" then
 * "Unique file evidence"). Each carries a stable `code` for machine consumers
 * and a human `message` for direct display. `detail` is an optional structured
 * payload (e.g. file path + size + hash for `UNIQUE_FILE_EVIDENCE`); empty
 * when the code is self-explanatory.
 */
struct Reason
{
    ReasonCode code = ReasonCode::IMPLICIT_DEFAULT;
    std::string message;
    nlohmann::json detail;
};

/**
 * @brief Per-axis confidence breakdown.
 *
 * Each component is in [0.0, 1.0]. They are aggregated up the hierarchy with
 * file-count weighting and combined into a single `composite` score using the
 * weights defined in `InferenceDiagnostics.cpp`.
 *
 * - `evidence`: fraction of plugin files that uniquely match target.
 * - `propagation`: 1.0 if forced by propagation, 0.0 if CSP-decided.
 * - `repro`: 1 - (group-local mismatches / group-local targets).
 * - `ambiguity`: 1 - (close-evidence alternatives / total alternatives).
 */
struct ConfidenceComponents
{
    double evidence = 1.0;
    double propagation = 1.0;
    double repro = 1.0;
    double ambiguity = 1.0;
};

/**
 * @brief Composite confidence score with a derived band.
 *
 * `composite` is a linear combination of the four `components` (see
 * `InferenceDiagnostics.cpp` for the exact weights). `band` is a categorical
 * bucket derived from `composite`:
 *
 * - "high"   when composite >= 0.85
 * - "medium" when 0.50 <= composite < 0.85
 * - "low"    when composite < 0.50
 */
struct ConfidenceScore
{
    double composite = 1.0;
    std::string band = "high";
    ConfidenceComponents components;
};

/**
 * @brief Diagnostics for one plugin in one group.
 *
 * The `selected` flag mirrors the boolean stored in `SolverResult.selections`
 * for this position; carrying it on the diagnostic object lets the wire format
 * express selected and deselected plugins as a single homogeneous shape.
 */
struct PluginDiagnostics
{
    bool selected = false;
    ConfidenceScore confidence;
    std::vector<Reason> reasons;
};

/**
 * @brief Diagnostics for one group in one step.
 *
 * `resolved_by` names the rule or solver phase that fixed the last unresolved
 * plugin in this group (e.g. `"propagation.required"`, `"csp.greedy"`). It is
 * the empty string before any decision was recorded; consumers should treat
 * empty-string as "unknown / implicit default".
 */
struct GroupDiagnostics
{
    ConfidenceScore confidence;
    std::string resolved_by;
    std::vector<Reason> reasons;
    std::vector<PluginDiagnostics> plugins;
};

/**
 * @brief Diagnostics for one installation step.
 */
struct StepDiagnostics
{
    ConfidenceScore confidence;
    std::vector<Reason> reasons;
    bool visible = true;
    std::vector<GroupDiagnostics> groups;
};

/**
 * @brief Per-pipeline timings reported in milliseconds.
 *
 * Values mirror `InferenceContext::t_list`, `t_scan`, `t_solve` plus a
 * `total` measured from infer_selections entry to JSON assembly.
 */
struct DiagnosticTimings
{
    int64_t list_ms = 0;
    int64_t scan_ms = 0;
    int64_t solve_ms = 0;
    int64_t total_ms = 0;
};

/**
 * @brief Group-resolution counters for the run-level summary.
 */
struct DiagnosticGroupCounts
{
    int total = 0;
    int resolved_by_propagation = 0;
    int resolved_by_csp = 0;
};

/**
 * @brief Cache-hit context for the run-level summary.
 *
 * `hit` is true when inference short-circuited via the Tier-1 meta.ini cache.
 * `source` names the cache origin (currently always `"fomod-plus"` when set).
 */
struct DiagnosticCacheInfo
{
    bool hit = false;
    std::string source;
};

/**
 * @brief Run-level diagnostic summary.
 *
 * Aggregates whole-pipeline metrics: composite confidence, exact-match flag,
 * highest CSP phase that contributed to the result, total nodes explored,
 * group-resolution counts, repro metrics, timings, and cache-hit context.
 */
struct RunDiagnostics
{
    ConfidenceScore confidence;
    bool exact_match = false;
    std::string phase_reached;
    int nodes_explored = 0;
    DiagnosticGroupCounts groups;
    ReproMetrics repro;
    DiagnosticTimings timings;
    DiagnosticCacheInfo cache;
};

/**
 * @brief Top-level diagnostics tree mirroring the FOMOD installer hierarchy.
 *
 * `schema_version` is incremented when the wire format changes in a
 * non-additive way; the first version that introduces this struct emits 2
 * (the legacy string-array format was implicit version 1).
 */
struct InferenceDiagnostics
{
    int schema_version = 2;
    RunDiagnostics run;
    std::vector<StepDiagnostics> steps;
};

/**
 * @brief Builder that accumulates per-decision reasons during inference and
 *        finalizes the confidence formula at the end.
 * @author Alex (https://github.com/lextpf)
 * @ingroup FomodService
 *
 * Construction sizes the nested `steps`/`groups`/`plugins` vectors to match
 * the installer's hierarchy so individual `add_plugin_reason` calls can index
 * directly without bounds-grow logic. Reason emission and metadata setters
 * are write-once-or-append: the builder does not enforce ordering, but
 * `finalize` must run last.
 *
 * Typical lifecycle inside `FomodInferenceService::infer_selections`:
 *
 * @code
 * InferenceDiagnosticsBuilder builder(installer);
 * builder.absorb_propagation(propagation_result);
 * builder.absorb_solver(solver_result);
 * builder.set_run_timings(t_list, t_scan, t_solve, t_total);
 * builder.finalize(solver_result, propagation_result, installer);
 * auto json = assemble_json(installer, solver_result, builder.diagnostics());
 * @endcode
 */
class MO2_API InferenceDiagnosticsBuilder
{
public:
    /**
     * @brief Construct with hierarchy sized to match @p installer.
     *
     * After construction, every step/group/plugin position is reachable via
     * its `(step, group, plugin)` triple and carries default-initialized
     * `PluginDiagnostics` (selected=false, IMPLICIT_DEFAULT reason set).
     */
    explicit InferenceDiagnosticsBuilder(const FomodInstaller& installer);

    /**
     * @brief Append a reason to a plugin's reason chain.
     *
     * @param step   Step index in `FomodInstaller::steps`.
     * @param group  Group index within the step.
     * @param plugin Plugin index within the group.
     * @param code   Stable reason code.
     * @param message Human-readable explanation.
     * @param detail Optional structured payload (default: empty json).
     */
    void add_plugin_reason(int step,
                           int group,
                           int plugin,
                           ReasonCode code,
                           std::string message,
                           nlohmann::json detail = {});

    /**
     * @brief Append a reason to a group's reason chain.
     *
     * Group-level reasons describe rules that resolved the group as a whole
     * (e.g. cardinality forcing, propagation completion).
     */
    void add_group_reason(
        int step, int group, ReasonCode code, std::string message, nlohmann::json detail = {});

    /**
     * @brief Set the rule or solver phase that resolved a group.
     *
     * @param resolved_by Stable string identifier such as
     *                    `"propagation.unique_evidence"` or `"csp.greedy"`.
     */
    void set_group_resolved_by(int step, int group, std::string resolved_by);

    /**
     * @brief Record a step's visibility decision and its origin.
     *
     * @param step    Step index.
     * @param visible Whether the step is visible at install time.
     * @param code    `STEP_VISIBILITY_FORCED` / `STEP_VISIBILITY_UNKNOWN` /
     *                `STEP_NOT_VISIBLE`.
     */
    void set_step_visibility(int step, bool visible, ReasonCode code);

    /**
     * @brief Record pipeline timings (in milliseconds).
     */
    void set_run_timings(int64_t list_ms, int64_t scan_ms, int64_t solve_ms, int64_t total_ms);

    /**
     * @brief Mark this run as a Tier-1 cache hit.
     *
     * Implicitly sets `phase_reached = "tier1_cache"` and pushes a
     * `FOMOD_PLUS_CACHE` reason on every plugin in the hierarchy.
     */
    void set_cache_hit(std::string source);

    /**
     * @brief Absorb propagation results into per-plugin and per-group reasons.
     *
     * Reads `PropagationResult.plugin_reasons`, `plugin_reason_details`, and
     * `resolved_by` (added in Phase 2) and copies them into the diagnostic
     * tree. Idempotent: calling twice with the same input is a no-op.
     */
    void absorb_propagation(const PropagationResult& propagation);

    /**
     * @brief Absorb solver results into per-group phase + ambiguity counts.
     *
     * Reads `SolverResult.phase_per_group`, `alternatives_per_group`, and
     * `repro` (added in Phase 2). Records the corresponding `CSP_PHASE_*`
     * reason on each plugin selected by that phase and feeds ambiguity
     * counts into the confidence formula.
     */
    void absorb_solver(const SolverResult& result);

    /**
     * @brief Compute confidence components and bands top-down.
     *
     * Walks the diagnostic tree, computes per-plugin components (`evidence`,
     * `propagation`, `repro`, `ambiguity`), aggregates group-, step-, and
     * run-level scores with file-count weighting, and assigns a categorical
     * `band`. Must be called last; subsequent setter calls are silently
     * dropped.
     */
    void finalize(const SolverResult& result,
                  const PropagationResult& propagation,
                  const FomodInstaller& installer);

    /**
     * @brief Access the accumulated diagnostics.
     *
     * The returned reference is valid until the builder is destroyed.
     */
    const InferenceDiagnostics& diagnostics() const;

private:
    InferenceDiagnostics diag_;
    bool finalized_ = false;
};

/**
 * @brief Serialize an `InferenceDiagnostics` tree to JSON.
 *
 * Produces an object whose layout matches the schema documented in the plan:
 * `{ confidence, exact_match, phase_reached, nodes_explored, groups, repro,
 *    timings_ms, cache }` for the run; nested step/group/plugin objects
 * carry their own confidence and reason chains.
 *
 * Used by `assemble_json` (`FomodInferenceAtoms.cpp`) when emitting schema
 * version 2 wire format.
 */
MO2_API nlohmann::json serialize_run_diagnostics(const RunDiagnostics& run);

/**
 * @brief Serialize a single `ConfidenceScore` to JSON.
 *
 * Centralized so step/group/plugin nodes share the exact same shape.
 */
MO2_API nlohmann::json serialize_confidence(const ConfidenceScore& score);

/**
 * @brief Serialize a single `Reason` to JSON.
 *
 * Always emits `code` (string name) and `message`. Emits `detail` only when
 * it is non-empty - empty `nlohmann::json` payloads are skipped to keep the
 * wire format compact.
 */
MO2_API nlohmann::json serialize_reason(const Reason& reason);

}  // namespace mo2core
