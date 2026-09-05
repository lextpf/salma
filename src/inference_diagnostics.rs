/*!
 * @brief builds schema-v2 inference reasons, confidence scores, and diagnostics.
 * @author Alex (https://github.com/lextpf)
 *
 * ReasonCode numeric values and names are wire data. append codes; do not renumber or repurpose
 * existing values.
 *
 * ### :material-format-list-numbered: confidence calculation
 *
 * @verbatim
 * plugin = 0.40*evidence + 0.30*propagation + 0.20*repro + 0.10*ambiguity
 * run    = weighted hierarchy aggregate - min(extra, 5)*0.05
 * run   -= 0.10 when phase_reached is "csp.fallback"
 * high >= 0.85; medium >= 0.50; low < 0.50
 * @endverbatim
 *
 * all component and composite values are clamped to [0, 1].
 */

use crate::fomod_csp_types::{ReproMetrics, SolverResult};
use crate::fomod_ir::FomodInstaller;
use crate::fomod_propagator::PropagationResult;
use crate::json::Value;
use crate::logger::Logger;

/**
 * @enum ReasonCode
 * @brief stable reason the inference engine attaches to a plugin, group or step decision.
 * @author Alex (https://github.com/lextpf)
 *
 * codes are integer-backed (`#[repr(i32)]`) and stable across releases: the dashboard maps the
 * integer and the name to UI labels and never reads the human message.
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ReasonCode {
    #[default]
    ImplicitDefault = 0,

    // forced by plugin-type constraints (propagator rule 1).
    ForcedRequired = 100,
    /**
     * @brief plugin type NotUsable eliminated the plugin.
     * @author Alex (https://github.com/lextpf)
     *
     * not recorded for a dynamic `dependencyType` evaluated without an external context: that
     * outcome is not definitive enough to prune on.
     */
    ForcedNotUsable = 101,

    // forced by the cardinality rule (propagator rule 3), and only after the group resolves.
    // SelectAtMostOne and SelectAny resolve at zero usable plugins and so record no Forced* code at
    // all.
    ForcedSelectAll = 102,
    ForcedAtLeastOne = 103,
    ForcedExactlyOne = 104,

    // file evidence (propagator rule 2).
    UniqueFileEvidence = 200,
    /**
     * @brief mark a plugin whose unique destinations are absent from the target.
     * @author Alex (https://github.com/lextpf)
     *
     * a plugin with no group-unique destination is never eliminated by this rule, however many of
     * its files are absent.
     */
    NoFileEvidence = 201,
    /**
     * @brief deselected because nothing in the target maps uniquely here.
     * @author Alex (https://github.com/lextpf)
     */
    NoUniqueEvidence = 202,

    // cardinality. reserved: rule 3 records the three FORCED_* codes above instead.
    /**
     * @brief group narrowed to a single combination by its group type.
     * @author Alex (https://github.com/lextpf)
     */
    CardinalityForced = 300,

    // CSP solver phases.
    CspPhaseGreedy = 400,
    CspPhaseLocalSearch = 401,
    CspPhaseBacktrack = 402,
    CspPhaseRepair = 403,
    CspPhaseFocused = 404,
    CspPhaseFallback = 405,

    // condition and step-visibility overrides. `compute_overrides` decides ForceTrue / ForceFalse /
    // Unknown for conditional patterns, and the forward simulator and the solver consume those
    // decisions, but none of them becomes a reason. only its step-visibility half reaches the
    // builder, through the three STEP_* codes below.
    ConditionForcedTrue = 500,
    ConditionForcedFalse = 501,
    ConditionUnknown = 502,
    StepVisibilityForced = 510,
    StepVisibilityUnknown = 511,
    /**
     * @brief step skipped entirely because not visible.
     * @author Alex (https://github.com/lextpf)
     */
    StepNotVisible = 512,

    // penalties and scoring.
    /**
     * @brief the selection produces a file the target does not have.
     * @author Alex (https://github.com/lextpf)
     */
    ExtraFileProduced = 600,

    // cache / shortcut.
    FomodPlusCache = 700,
}

/**
 * @fn reason_code_to_string(ReasonCode) -> &'static str
 * @brief wire name of a ReasonCode, for example "FORCED_REQUIRED".
 * @author Alex (https://github.com/lextpf)
 *
 */
pub fn reason_code_to_string(code: ReasonCode) -> &'static str {
    match code {
        ReasonCode::ImplicitDefault => "IMPLICIT_DEFAULT",
        ReasonCode::ForcedRequired => "FORCED_REQUIRED",
        ReasonCode::ForcedNotUsable => "FORCED_NOT_USABLE",
        ReasonCode::ForcedSelectAll => "FORCED_SELECT_ALL",
        ReasonCode::ForcedAtLeastOne => "FORCED_AT_LEAST_ONE",
        ReasonCode::ForcedExactlyOne => "FORCED_EXACTLY_ONE",
        ReasonCode::UniqueFileEvidence => "UNIQUE_FILE_EVIDENCE",
        ReasonCode::NoFileEvidence => "NO_FILE_EVIDENCE",
        ReasonCode::NoUniqueEvidence => "NO_UNIQUE_EVIDENCE",
        ReasonCode::CardinalityForced => "CARDINALITY_FORCED",
        ReasonCode::CspPhaseGreedy => "CSP_PHASE_GREEDY",
        ReasonCode::CspPhaseLocalSearch => "CSP_PHASE_LOCAL_SEARCH",
        ReasonCode::CspPhaseBacktrack => "CSP_PHASE_BACKTRACK",
        ReasonCode::CspPhaseRepair => "CSP_PHASE_REPAIR",
        ReasonCode::CspPhaseFocused => "CSP_PHASE_FOCUSED",
        ReasonCode::CspPhaseFallback => "CSP_PHASE_FALLBACK",
        ReasonCode::ConditionForcedTrue => "CONDITION_FORCED_TRUE",
        ReasonCode::ConditionForcedFalse => "CONDITION_FORCED_FALSE",
        ReasonCode::ConditionUnknown => "CONDITION_UNKNOWN",
        ReasonCode::StepVisibilityForced => "STEP_VISIBILITY_FORCED",
        ReasonCode::StepVisibilityUnknown => "STEP_VISIBILITY_UNKNOWN",
        ReasonCode::StepNotVisible => "STEP_NOT_VISIBLE",
        ReasonCode::ExtraFileProduced => "EXTRA_FILE_PRODUCED",
        ReasonCode::FomodPlusCache => "FOMOD_PLUS_CACHE",
    }
}

/**
 * @enum ReasonDetail
 * @brief structured payload carried beside a plugin reason.
 * @author Alex (https://github.com/lextpf)
 *
 * a reason with no payload stores `None` rather than an empty variant, and [`serialize_reason`]
 * then omits the `detail` key entirely.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasonDetail {
    /**
     * @brief positive file evidence: the plugin uniquely produces at least one target file.
     * @author Alex (https://github.com/lextpf)
     *
     * `files` holds up to four examples, sorted byte-ascending so the document is deterministic;
     * `count` is the full number of unique target hits and may exceed `files.len()`.
     */
    UniqueFileEvidence {
        // up to four example destination paths, byte-ascending.
        files: Vec<String>,
        count: i32,
    },
    /**
     * @brief identifies the CSP phase that selected a plugin.
     * @author Alex (https://github.com/lextpf)
     *
     * phase names are stable wire values.
     */
    CspPhase {
        nodes: i32,
        // stable phase identifier, for example "csp.greedy" or "csp.fallback".
        phase: String,
    },
}

const WEIGHT_EVIDENCE: f64 = 0.40;
const WEIGHT_PROPAGATION: f64 = 0.30;
const WEIGHT_REPRO: f64 = 0.20;
const WEIGHT_AMBIGUITY: f64 = 0.10;

const BAND_HIGH_THRESHOLD: f64 = 0.85;
const BAND_MEDIUM_THRESHOLD: f64 = 0.50;

const RUN_EXTRA_PENALTY_PER_FILE: f64 = 0.05;
const RUN_EXTRA_PENALTY_CAP: i32 = 5;
const RUN_FALLBACK_PENALTY: f64 = 0.10;

/**
 * @struct Reason
 * @brief one justification attached to a plugin, group or step decision.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub code: ReasonCode,
    pub message: String,
    pub detail: Option<ReasonDetail>,
}

/**
 * @struct ConfidenceComponents
 * @brief per-axis confidence values from 0.0 to 1.0, each defaulting to 1.0.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfidenceComponents {
    /**
     * @brief score file evidence from the plugin reason chain.
     * @author Alex (https://github.com/lextpf)
     *
     * forcing evidence scores 1.0. otherwise the first matching reason scores 1.0 for
     * UniqueFileEvidence, 0.5 for NoUniqueEvidence or 0.3 for ExtraFileProduced. without a
     * matching reason, selected plugins score 0.5 and deselected plugins score 0.7. this is not a
     * file-count ratio.
     */
    pub evidence: f64,
    /**
     * @brief 1.0 when the plugin's reason chain holds a propagation-forcing code, 0.0 otherwise.
     * @author Alex (https://github.com/lextpf)
     *
     * it is not a positive statement that the CSP made the choice.
     */
    pub propagation: f64,
    /**
     * @brief run-level reproduction quality, not a group-local ratio.
     * @author Alex (https://github.com/lextpf)
     *
     * it is 1.0 for a deselected plugin and for a selected plugin in an exact-match run; otherwise
     * `0.85 * repro_ratio`, where `repro_ratio` is computed once per run as `clamp01(1 - (missing +
     * 0.5 * (size_mismatch + hash_mismatch)) / target_file_count)` and defaults to 1.0 when
     * `target_file_count` is 0.
     */
    pub repro: f64,
    /**
     * @brief score ambiguity from the solver alternative count.
     * @author Alex (https://github.com/lextpf)
     *
     * zero, one, two, and at least three alternatives score 1.0, 0.6, 0.4, and 0.2. the solver
     * initializes all counts to zero, so this component is 1.0.
     */
    pub ambiguity: f64,
}

impl Default for ConfidenceComponents {
    fn default() -> Self {
        ConfidenceComponents {
            evidence: 1.0,
            propagation: 1.0,
            repro: 1.0,
            ambiguity: 1.0,
        }
    }
}

/**
 * @struct ConfidenceScore
 * @brief composite confidence score with a derived band.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, PartialEq)]
pub struct ConfidenceScore {
    /**
     * @brief combine confidence components into one normalized score.
     * @author Alex (https://github.com/lextpf)
     *
     * the score is `clamp01(0.40 * evidence + 0.30 * propagation + 0.20 * repro + 0.10 *
     * ambiguity)`. propagation-forced groups use 1.0 directly.
     */
    pub composite: f64,
    pub band: String,
    pub components: ConfidenceComponents,
}

impl Default for ConfidenceScore {
    fn default() -> Self {
        ConfidenceScore {
            composite: 1.0,
            band: "high".to_string(),
            components: ConfidenceComponents::default(),
        }
    }
}

/**
 * @struct PluginDiagnostics
 * @brief diagnostics for one plugin in one group.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginDiagnostics {
    pub selected: bool,
    pub confidence: ConfidenceScore,
    /**
     * @brief reason chain in evaluation order.
     * @author Alex (https://github.com/lextpf)
     */
    pub reasons: Vec<Reason>,
}

/**
 * @struct GroupDiagnostics
 * @brief diagnostics for one group in one step.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupDiagnostics {
    pub confidence: ConfidenceScore,
    /**
     * @brief identifies the rule or CSP phase that resolved the group.
     * @author Alex (https://github.com/lextpf)
     *
     * propagation.* and cache.fomod_plus count as propagation. csp.* counts as CSP. other values
     * do not increment either run counter. an empty value means no attribution.
     */
    pub resolved_by: String,
    pub reasons: Vec<Reason>,
    pub plugins: Vec<PluginDiagnostics>,
}

/**
 * @struct StepDiagnostics
 * @brief diagnostics for one installation step.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, PartialEq)]
pub struct StepDiagnostics {
    pub confidence: ConfidenceScore,
    pub reasons: Vec<Reason>,
    pub visible: bool,
    pub groups: Vec<GroupDiagnostics>,
}

impl Default for StepDiagnostics {
    fn default() -> Self {
        StepDiagnostics {
            confidence: ConfidenceScore::default(),
            reasons: Vec::new(),
            visible: true,
            groups: Vec::new(),
        }
    }
}

/**
 * @struct DiagnosticTimings
 * @brief pipeline timings in milliseconds.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticTimings {
    pub list_ms: i64,
    pub scan_ms: i64,
    pub solve_ms: i64,
    pub total_ms: i64,
}

/**
 * @struct DiagnosticGroupCounts
 * @brief group-resolution counters, tallied once by InferenceDiagnosticsBuilder::absorb_solver.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticGroupCounts {
    pub total: i32,
    pub resolved_by_propagation: i32,
    /**
     * @brief groups resolved by the CSP solver.
     * @author Alex (https://github.com/lextpf)
     */
    pub resolved_by_csp: i32,
}

/**
 * @struct DiagnosticCacheInfo
 * @brief cache-hit context.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticCacheInfo {
    pub hit: bool,
    pub source: String,
}

/**
 * @struct RunDiagnostics
 * @brief run-level diagnostic summary.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunDiagnostics {
    pub confidence: ConfidenceScore,
    pub exact_match: bool,
    /**
     * @brief highest CSP phase that contributed (or "tier1_cache").
     * @author Alex (https://github.com/lextpf)
     */
    pub phase_reached: String,
    /**
     * @brief total CSP search-tree nodes explored.
     * @author Alex (https://github.com/lextpf)
     */
    pub nodes_explored: i32,
    pub groups: DiagnosticGroupCounts,
    pub repro: ReproMetrics,
    pub timings: DiagnosticTimings,
    pub cache: DiagnosticCacheInfo,
}

/**
 * @struct InferenceDiagnostics
 * @brief top-level diagnostics tree, shaped like the FOMOD installer hierarchy.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, PartialEq)]
pub struct InferenceDiagnostics {
    pub schema_version: i32,
    pub run: RunDiagnostics,
    pub steps: Vec<StepDiagnostics>,
}

impl Default for InferenceDiagnostics {
    fn default() -> Self {
        InferenceDiagnostics {
            schema_version: 2,
            run: RunDiagnostics::default(),
            steps: Vec::new(),
        }
    }
}

fn band_for(composite: f64) -> &'static str {
    if composite >= BAND_HIGH_THRESHOLD {
        "high"
    } else if composite >= BAND_MEDIUM_THRESHOLD {
        "medium"
    } else {
        "low"
    }
}

// clamp to from 0.0 to 1.0.
// the bounds are finite constants, so this cannot panic.
fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

// weighted mean, returning 1.0 when the weight is not positive.
// every aggregation level uses this guard for an empty level.
fn weighted_mean(sum: f64, weight: f64) -> f64 {
    if weight <= 0.0 { 1.0 } else { sum / weight }
}

// multiplication order is part of the wire value; all ones produces 0.9999999999999999.
fn composite_from(c: &ConfidenceComponents) -> f64 {
    clamp01(
        WEIGHT_EVIDENCE * c.evidence
            + WEIGHT_PROPAGATION * c.propagation
            + WEIGHT_REPRO * c.repro
            + WEIGHT_AMBIGUITY * c.ambiguity,
    )
}

fn plugin_was_propagation_forced(reasons: &[Reason]) -> bool {
    reasons.iter().any(|r| {
        matches!(
            r.code,
            ReasonCode::ForcedRequired
                | ReasonCode::ForcedNotUsable
                | ReasonCode::ForcedSelectAll
                | ReasonCode::ForcedAtLeastOne
                | ReasonCode::ForcedExactlyOne
                | ReasonCode::UniqueFileEvidence
                | ReasonCode::NoFileEvidence
                | ReasonCode::CardinalityForced
                | ReasonCode::FomodPlusCache
        )
    })
}

// reason code for a CSP phase id.
fn csp_phase_to_reason(phase_id: &str) -> ReasonCode {
    match phase_id {
        "csp.greedy" => ReasonCode::CspPhaseGreedy,
        "csp.local_search" => ReasonCode::CspPhaseLocalSearch,
        "csp.backtrack" => ReasonCode::CspPhaseBacktrack,
        "csp.repair" => ReasonCode::CspPhaseRepair,
        "csp.focused" => ReasonCode::CspPhaseFocused,
        "csp.fallback" => ReasonCode::CspPhaseFallback,
        _ => ReasonCode::ImplicitDefault,
    }
}

// human message for a CSP phase id; empty for an unrecognised id.
fn csp_phase_message(phase_id: &str) -> &'static str {
    match phase_id {
        "csp.greedy" => "Resolved in greedy phase",
        "csp.local_search" => "Resolved in local-search phase",
        "csp.backtrack" => "Resolved in backtrack phase",
        "csp.repair" => "Resolved in residual-repair phase",
        "csp.focused" => "Resolved in focused-search phase",
        "csp.fallback" => "Resolved in global-fallback phase",
        _ => "",
    }
}

fn plugin_file_count(installer: &FomodInstaller, s: i32, g: i32, p: i32) -> i32 {
    if s < 0 || s as usize >= installer.steps.len() {
        return 0;
    }
    let step = &installer.steps[s as usize];
    if g < 0 || g as usize >= step.groups.len() {
        return 0;
    }
    let group = &step.groups[g as usize];
    if p < 0 || p as usize >= group.plugins.len() {
        return 0;
    }
    group.plugins[p as usize].files.len() as i32
}

// per-plugin evidence axis: a forced plugin scores 1.0; otherwise the first evidence reason in
// chain order wins (UniqueFileEvidence 1.0, NoUniqueEvidence 0.5, ExtraFileProduced 0.3); with no
// evidence reason, a selected plugin scores 0.5 and a deselected one 0.7.
fn evidence_component(plugin: &PluginDiagnostics, propagation_forced: bool) -> f64 {
    if propagation_forced {
        return 1.0;
    }
    for r in &plugin.reasons {
        match r.code {
            ReasonCode::UniqueFileEvidence => return 1.0,
            ReasonCode::NoUniqueEvidence => return 0.5,
            ReasonCode::ExtraFileProduced => return 0.3,
            _ => {}
        }
    }
    if plugin.selected { 0.5 } else { 0.7 }
}

fn propagation_component(forced: bool) -> f64 {
    if forced { 1.0 } else { 0.0 }
}

fn ambiguity_component(alternatives_in_group: i32) -> f64 {
    if alternatives_in_group <= 0 {
        1.0
    } else if alternatives_in_group == 1 {
        0.6
    } else if alternatives_in_group == 2 {
        0.4
    } else {
        0.2
    }
}

/**
 * @struct InferenceDiagnosticsBuilder
 * @brief accumulates per-decision reasons during inference, then computes the confidence formula.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone)]
pub struct InferenceDiagnosticsBuilder {
    diag: InferenceDiagnostics,
    finalized: bool,
    target_file_count: i32,
}

impl InferenceDiagnosticsBuilder {
    pub fn new(installer: &FomodInstaller) -> Self {
        let steps = installer
            .steps
            .iter()
            .map(|step| StepDiagnostics {
                visible: true,
                groups: step
                    .groups
                    .iter()
                    .map(|group| GroupDiagnostics {
                        plugins: vec![PluginDiagnostics::default(); group.plugins.len()],
                        ..GroupDiagnostics::default()
                    })
                    .collect(),
                ..StepDiagnostics::default()
            })
            .collect();
        let total: i32 = installer.steps.iter().map(|s| s.groups.len() as i32).sum();
        let mut diag = InferenceDiagnostics {
            steps,
            ..InferenceDiagnostics::default()
        };
        diag.run.groups.total = total;
        InferenceDiagnosticsBuilder {
            diag,
            finalized: false,
            target_file_count: 0,
        }
    }

    pub fn add_plugin_reason(
        &mut self,
        step: i32,
        group: i32,
        plugin: i32,
        code: ReasonCode,
        message: impl Into<String>,
        detail: Option<ReasonDetail>,
    ) {
        if self.finalized {
            return;
        }
        if step < 0 || step as usize >= self.diag.steps.len() {
            return;
        }
        let s = step as usize;
        if group < 0 || group as usize >= self.diag.steps[s].groups.len() {
            return;
        }
        let g = group as usize;
        if plugin < 0 || plugin as usize >= self.diag.steps[s].groups[g].plugins.len() {
            return;
        }
        let p = plugin as usize;
        self.diag.steps[s].groups[g].plugins[p]
            .reasons
            .push(Reason {
                code,
                message: message.into(),
                detail,
            });
    }

    pub fn add_group_reason(
        &mut self,
        step: i32,
        group: i32,
        code: ReasonCode,
        message: impl Into<String>,
        detail: Option<ReasonDetail>,
    ) {
        if self.finalized {
            return;
        }
        if step < 0 || step as usize >= self.diag.steps.len() {
            return;
        }
        let s = step as usize;
        if group < 0 || group as usize >= self.diag.steps[s].groups.len() {
            return;
        }
        let g = group as usize;
        self.diag.steps[s].groups[g].reasons.push(Reason {
            code,
            message: message.into(),
            detail,
        });
    }

    pub fn set_group_resolved_by(&mut self, step: i32, group: i32, resolved_by: impl Into<String>) {
        if self.finalized {
            return;
        }
        if step < 0 || step as usize >= self.diag.steps.len() {
            return;
        }
        let s = step as usize;
        if group < 0 || group as usize >= self.diag.steps[s].groups.len() {
            return;
        }
        let g = group as usize;
        self.diag.steps[s].groups[g].resolved_by = resolved_by.into();
    }

    /**
     * @fn set_step_visibility(&mut self, i32, bool, ReasonCode)
     * @brief ignore finalized builders and out-of-range step indices.
     * @author Alex (https://github.com/lextpf)
     *
     */
    pub fn set_step_visibility(&mut self, step: i32, visible: bool, code: ReasonCode) {
        if self.finalized {
            return;
        }
        if step < 0 || step as usize >= self.diag.steps.len() {
            return;
        }
        let s = step as usize;
        self.diag.steps[s].visible = visible;
        let msg: &str = match code {
            ReasonCode::StepVisibilityForced => {
                if visible {
                    "Visibility condition evaluated true"
                } else {
                    "Visibility condition evaluated false"
                }
            }
            ReasonCode::StepVisibilityUnknown => "Visibility condition could not be determined",
            ReasonCode::StepNotVisible => "Step skipped (not visible)",
            _ => "Step visibility recorded",
        };
        self.diag.steps[s].reasons.push(Reason {
            code,
            message: msg.to_string(),
            detail: None,
        });
    }

    /**
     * @fn set_run_timings(&mut self, i64, i64, i64, i64)
     * @brief overwrite millisecond timings unless the builder is finalized.
     * @author Alex (https://github.com/lextpf)
     *
     */
    pub fn set_run_timings(&mut self, list_ms: i64, scan_ms: i64, solve_ms: i64, total_ms: i64) {
        if self.finalized {
            return;
        }
        self.diag.run.timings.list_ms = list_ms;
        self.diag.run.timings.scan_ms = scan_ms;
        self.diag.run.timings.solve_ms = solve_ms;
        self.diag.run.timings.total_ms = total_ms;
    }

    /**
     * @fn set_cache_hit(&mut self, impl Into<String>)
     * @brief append an undeduplicated cache reason to every plugin.
     * @author Alex (https://github.com/lextpf)
     *
     * the reason append is not de-duplicated, so a second call leaves two identical reasons on
     * every plugin.
     */
    pub fn set_cache_hit(&mut self, source: impl Into<String>) {
        if self.finalized {
            return;
        }
        self.diag.run.cache.hit = true;
        self.diag.run.cache.source = source.into();
        self.diag.run.phase_reached = "tier1_cache".to_string();
        for step in &mut self.diag.steps {
            for group in &mut step.groups {
                group.resolved_by = "cache.fomod_plus".to_string();
                for plugin in &mut group.plugins {
                    plugin.reasons.push(Reason {
                        code: ReasonCode::FomodPlusCache,
                        message: "Cached selection from meta.ini".to_string(),
                        detail: None,
                    });
                }
            }
        }
    }

    pub fn set_target_file_count(&mut self, count: i32) {
        if self.finalized {
            return;
        }
        self.target_file_count = count;
    }

    /**
     * @fn absorb_propagation(&mut self, &PropagationResult)
     * @brief ignore empty group labels and ImplicitDefault plugin reasons.
     * @author Alex (https://github.com/lextpf)
     *
     * an empty `resolved_by` from the propagator leaves the current value alone, and an
     * `ImplicitDefault` plugin code records no reason at all.
     */
    pub fn absorb_propagation(&mut self, propagation: &PropagationResult) {
        if self.finalized {
            return;
        }
        // resolved_by per group.
        let rb_steps = propagation.resolved_by.len().min(self.diag.steps.len());
        for s in 0..rb_steps {
            let rb_groups = propagation.resolved_by[s]
                .len()
                .min(self.diag.steps[s].groups.len());
            for g in 0..rb_groups {
                let src = &propagation.resolved_by[s][g];
                if !src.is_empty() {
                    self.diag.steps[s].groups[g].resolved_by = src.clone();
                }
            }
        }

        // plugin reasons.
        let pr_steps = propagation.plugin_reasons.len().min(self.diag.steps.len());
        for s in 0..pr_steps {
            let pr_groups = propagation.plugin_reasons[s]
                .len()
                .min(self.diag.steps[s].groups.len());
            for g in 0..pr_groups {
                let pr_plugins = propagation.plugin_reasons[s][g]
                    .len()
                    .min(self.diag.steps[s].groups[g].plugins.len());
                for p in 0..pr_plugins {
                    let code = propagation.plugin_reasons[s][g][p];
                    if code == ReasonCode::ImplicitDefault {
                        continue;
                    }
                    let detail = propagation
                        .plugin_reason_details
                        .get(s)
                        .and_then(|x| x.get(g))
                        .and_then(|x| x.get(p))
                        .cloned()
                        .flatten();
                    let msg: &str = match code {
                        ReasonCode::ForcedRequired => "Plugin type Required",
                        ReasonCode::ForcedNotUsable => "Plugin type NotUsable",
                        ReasonCode::ForcedSelectAll => "Group SelectAll forces this plugin on",
                        ReasonCode::ForcedAtLeastOne => "AtLeastOne with single valid combo",
                        ReasonCode::ForcedExactlyOne => "SelectExactlyOne with single valid combo",
                        ReasonCode::UniqueFileEvidence => "Uniquely produces target file(s)",
                        ReasonCode::NoFileEvidence => "All declared files absent from target",
                        ReasonCode::NoUniqueEvidence => "No unique target evidence",
                        ReasonCode::CardinalityForced => "Single combo remains under group type",
                        other => reason_code_to_string(other),
                    };
                    self.add_plugin_reason(
                        s as i32,
                        g as i32,
                        p as i32,
                        code,
                        msg.to_string(),
                        detail,
                    );
                }
            }
        }
    }

    /**
     * @fn absorb_solver(&mut self, &SolverResult)
     * @brief preserve earlier group attribution while importing solver outcomes.
     * @author Alex (https://github.com/lextpf)
     *
     * a group's `resolved_by` is written only while it is still empty, so `absorb_propagation` and
     * `set_cache_hit` must run before this call or their attribution is replaced by the CSP phase
     * id.
     */
    pub fn absorb_solver(&mut self, result: &SolverResult) {
        if self.finalized {
            return;
        }
        self.diag.run.exact_match = result.exact_match;
        self.diag.run.nodes_explored = result.nodes_explored;
        self.diag.run.repro.missing = result.missing;
        self.diag.run.repro.extra = result.extra;
        self.diag.run.repro.size_mismatch = result.size_mismatch;
        self.diag.run.repro.hash_mismatch = result.hash_mismatch;

        if !result.phase_reached.is_empty() {
            self.diag.run.phase_reached = result.phase_reached.clone();
        }

        // per-group CSP phase -> reason on each selected plugin.
        let pg_steps = result.phase_per_group.len().min(self.diag.steps.len());
        for s in 0..pg_steps {
            let pg_groups = result.phase_per_group[s]
                .len()
                .min(self.diag.steps[s].groups.len());
            for g in 0..pg_groups {
                let phase_id = result.phase_per_group[s][g].clone();
                if phase_id.is_empty() {
                    continue;
                }
                if self.diag.steps[s].groups[g].resolved_by.is_empty() {
                    self.diag.steps[s].groups[g].resolved_by = phase_id.clone();
                }
                let rcode = csp_phase_to_reason(&phase_id);
                if rcode == ReasonCode::ImplicitDefault {
                    continue;
                }
                let msg = csp_phase_message(&phase_id);
                if s < result.selections.len() && g < result.selections[s].len() {
                    let plugin_count = self.diag.steps[s].groups[g].plugins.len();
                    let sel = &result.selections[s][g];
                    for (p, &selected) in sel.iter().enumerate().take(plugin_count) {
                        if !selected {
                            continue;
                        }
                        let detail = Some(ReasonDetail::CspPhase {
                            nodes: result.nodes_explored,
                            phase: phase_id.clone(),
                        });
                        self.add_plugin_reason(
                            s as i32,
                            g as i32,
                            p as i32,
                            rcode,
                            msg.to_string(),
                            detail,
                        );
                    }
                }
            }
        }

        let sel_steps = result.selections.len().min(self.diag.steps.len());
        for s in 0..sel_steps {
            let sel_groups = result.selections[s]
                .len()
                .min(self.diag.steps[s].groups.len());
            for g in 0..sel_groups {
                let sel_plugins = result.selections[s][g]
                    .len()
                    .min(self.diag.steps[s].groups[g].plugins.len());
                for p in 0..sel_plugins {
                    self.diag.steps[s].groups[g].plugins[p].selected = result.selections[s][g][p];
                }
            }
        }

        // group counts.
        let mut prop = 0;
        let mut csp = 0;
        for step in &self.diag.steps {
            for group in &step.groups {
                let rb = &group.resolved_by;
                if rb.starts_with("propagation") || rb == "cache.fomod_plus" {
                    prop += 1;
                } else if rb.starts_with("csp.") {
                    csp += 1;
                }
            }
        }
        self.diag.run.groups.resolved_by_propagation = prop;
        self.diag.run.groups.resolved_by_csp = csp;
    }

    /**
     * @fn finalize(&mut self, &SolverResult, &PropagationResult, &FomodInstaller)
     * @brief compute confidence scores and reproduction totals once.
     * @author Alex (https://github.com/lextpf)
     *
     * must run last. it sets `finalized`; later setters and repeated calls return without changes.
     */
    pub fn finalize(
        &mut self,
        result: &SolverResult,
        _propagation: &PropagationResult,
        installer: &FomodInstaller,
    ) {
        if self.finalized {
            return;
        }

        // fraction of the target tree the chosen selections reproduce, computed once for the whole
        // run. the 0.5 gives a size or hash mismatch half the weight of a miss: the file exists at
        // the right destination and only its content or size is wrong.
        let mut repro_ratio = 1.0_f64;
        if !result.exact_match && self.target_file_count > 0 {
            let effective_miss =
                result.missing as f64 + 0.5 * (result.size_mismatch + result.hash_mismatch) as f64;
            repro_ratio = clamp01(1.0 - effective_miss / self.target_file_count as f64);
        }

        for s in 0..self.diag.steps.len() {
            for g in 0..self.diag.steps[s].groups.len() {
                let alternatives = result
                    .alternatives_per_group
                    .get(s)
                    .and_then(|row| row.get(g))
                    .copied()
                    .unwrap_or(0);

                for p in 0..self.diag.steps[s].groups[g].plugins.len() {
                    let prop_forced = plugin_was_propagation_forced(
                        &self.diag.steps[s].groups[g].plugins[p].reasons,
                    );
                    let selected = self.diag.steps[s].groups[g].plugins[p].selected;
                    let evidence = clamp01(evidence_component(
                        &self.diag.steps[s].groups[g].plugins[p],
                        prop_forced,
                    ));
                    let propagation = clamp01(propagation_component(prop_forced));
                    // the 0.85 is a fixed ceiling: a selected plugin in a
                    // non-exact run never scores above 0.85 on the repro axis, even when nothing is
                    // missing.
                    let repro = if selected {
                        if result.exact_match {
                            1.0
                        } else {
                            0.85 * repro_ratio
                        }
                    } else {
                        1.0
                    };
                    let ambiguity = clamp01(ambiguity_component(alternatives));
                    let plugin = &mut self.diag.steps[s].groups[g].plugins[p];
                    plugin.confidence.components.evidence = evidence;
                    plugin.confidence.components.propagation = propagation;
                    plugin.confidence.components.repro = repro;
                    plugin.confidence.components.ambiguity = ambiguity;
                    plugin.confidence.composite = composite_from(&plugin.confidence.components);
                    plugin.confidence.band = band_for(plugin.confidence.composite).to_string();
                }

                // group composite: all-forced short-circuits to 1.0, else the file-count weighted
                // mean of the four components.
                let all_forced = !self.diag.steps[s].groups[g].plugins.is_empty()
                    && self.diag.steps[s].groups[g]
                        .plugins
                        .iter()
                        .all(|pd| plugin_was_propagation_forced(&pd.reasons));
                if all_forced {
                    let gc = &mut self.diag.steps[s].groups[g].confidence;
                    gc.components.evidence = 1.0;
                    gc.components.propagation = 1.0;
                    gc.components.repro = 1.0;
                    gc.components.ambiguity = 1.0;
                    gc.composite = 1.0;
                } else {
                    let mut total_w = 0.0;
                    let (mut ev, mut pr, mut rp, mut am) = (0.0, 0.0, 0.0, 0.0);
                    for p in 0..self.diag.steps[s].groups[g].plugins.len() {
                        let w =
                            (plugin_file_count(installer, s as i32, g as i32, p as i32) + 1) as f64;
                        total_w += w;
                        let pc = &self.diag.steps[s].groups[g].plugins[p]
                            .confidence
                            .components;
                        ev += w * pc.evidence;
                        pr += w * pc.propagation;
                        rp += w * pc.repro;
                        am += w * pc.ambiguity;
                    }
                    let gc = &mut self.diag.steps[s].groups[g].confidence;
                    gc.components.evidence = clamp01(weighted_mean(ev, total_w));
                    gc.components.propagation = clamp01(weighted_mean(pr, total_w));
                    gc.components.repro = clamp01(weighted_mean(rp, total_w));
                    gc.components.ambiguity = clamp01(weighted_mean(am, total_w));
                    gc.composite = composite_from(&gc.components);
                }
                let composite = self.diag.steps[s].groups[g].confidence.composite;
                self.diag.steps[s].groups[g].confidence.band = band_for(composite).to_string();
            }

            // step composite: group-count weighted mean.
            let mut total_w = 0.0;
            let (mut ev, mut pr, mut rp, mut am) = (0.0, 0.0, 0.0, 0.0);
            for g in 0..self.diag.steps[s].groups.len() {
                let w = (self.diag.steps[s].groups[g].plugins.len() + 1) as f64;
                total_w += w;
                let gc = &self.diag.steps[s].groups[g].confidence.components;
                ev += w * gc.evidence;
                pr += w * gc.propagation;
                rp += w * gc.repro;
                am += w * gc.ambiguity;
            }
            let sc = &mut self.diag.steps[s].confidence;
            sc.components.evidence = clamp01(weighted_mean(ev, total_w));
            sc.components.propagation = clamp01(weighted_mean(pr, total_w));
            sc.components.repro = clamp01(weighted_mean(rp, total_w));
            sc.components.ambiguity = clamp01(weighted_mean(am, total_w));
            sc.composite = composite_from(&sc.components);
            let composite = sc.composite;
            self.diag.steps[s].confidence.band = band_for(composite).to_string();
        }

        let mut total_w = 0.0;
        let (mut ev, mut pr, mut rp, mut am) = (0.0, 0.0, 0.0, 0.0);
        for s in 0..self.diag.steps.len() {
            let w = (self.diag.steps[s].groups.len() + 1) as f64;
            total_w += w;
            let sc = &self.diag.steps[s].confidence.components;
            ev += w * sc.evidence;
            pr += w * sc.propagation;
            rp += w * sc.repro;
            am += w * sc.ambiguity;
        }
        let rc = &mut self.diag.run.confidence.components;
        rc.evidence = clamp01(weighted_mean(ev, total_w));
        rc.propagation = clamp01(weighted_mean(pr, total_w));
        rc.repro = clamp01(weighted_mean(rp, total_w));
        rc.ambiguity = clamp01(weighted_mean(am, total_w));
        let mut composite = composite_from(&self.diag.run.confidence.components);

        // penalties.
        let extra_capped = result.extra.min(RUN_EXTRA_PENALTY_CAP);
        composite -= RUN_EXTRA_PENALTY_PER_FILE * extra_capped as f64;
        if self.diag.run.phase_reached == "csp.fallback" {
            composite -= RUN_FALLBACK_PENALTY;
        }
        self.diag.run.confidence.composite = clamp01(composite);
        let run_composite = self.diag.run.confidence.composite;
        self.diag.run.confidence.band = band_for(run_composite).to_string();

        // backfill `reproduced`. the zero test is defensive: no path writes `run.repro.reproduced`
        // before this point, and `finalize` cannot run twice.
        // the two branches produce very different numbers. with a target count, `reproduced` is the
        // target size minus the misses and the mismatches. without one it is a proxy: the declared
        // file count of the selected plugins, minus the misses. both clamp at 0.
        if self.diag.run.repro.reproduced == 0 {
            if self.target_file_count > 0 {
                self.diag.run.repro.reproduced = (self.target_file_count
                    - result.missing
                    - result.size_mismatch
                    - result.hash_mismatch)
                    .max(0);
            } else {
                let mut contributed = 0;
                for s in 0..result.selections.len() {
                    for g in 0..result.selections[s].len() {
                        for p in 0..result.selections[s][g].len() {
                            if result.selections[s][g][p] {
                                contributed +=
                                    plugin_file_count(installer, s as i32, g as i32, p as i32);
                            }
                        }
                    }
                }
                self.diag.run.repro.reproduced = (contributed - result.missing).max(0);
            }
        }

        self.finalized = true;

        // one-line run summary. this is a log line and not part of the emitted JSON, so its
        // rounding is not a wire contract.
        let run = &self.diag.run;
        Logger::instance().log(&format!(
            "[infer] Diagnostics: confidence={:.2} ({}), phase={}, repro=miss:{}/extra:{}/sm:{}/hm:{}",
            run.confidence.composite,
            run.confidence.band,
            if run.phase_reached.is_empty() {
                "n/a"
            } else {
                &run.phase_reached
            },
            run.repro.missing,
            run.repro.extra,
            run.repro.size_mismatch,
            run.repro.hash_mismatch
        ));
    }

    pub fn diagnostics(&self) -> &InferenceDiagnostics {
        &self.diag
    }
}

// serialize a ReasonDetail to its schema-v2 object.
// keys emit as `count` then `files` for unique-file evidence, `nodes` then `phase` for a CSP phase.
fn serialize_reason_detail(detail: &ReasonDetail) -> Value {
    let mut j = Value::object();
    match detail {
        ReasonDetail::UniqueFileEvidence { files, count } => {
            j.insert("count", Value::Int(*count as i64));
            let mut arr = Value::array();
            for f in files {
                arr.push(Value::string(f));
            }
            j.insert("files", arr);
        }
        ReasonDetail::CspPhase { nodes, phase } => {
            j.insert("nodes", Value::Int(*nodes as i64));
            j.insert("phase", Value::string(phase));
        }
    }
    j
}

pub fn serialize_confidence(score: &ConfidenceScore) -> Value {
    let mut j = Value::object();
    j.insert("composite", Value::Double(score.composite));
    j.insert("band", Value::string(&score.band));
    let mut comps = Value::object();
    comps.insert("evidence", Value::Double(score.components.evidence));
    comps.insert("propagation", Value::Double(score.components.propagation));
    comps.insert("repro", Value::Double(score.components.repro));
    comps.insert("ambiguity", Value::Double(score.components.ambiguity));
    j.insert("components", comps);
    j
}

pub fn serialize_reason(reason: &Reason) -> Value {
    let mut j = Value::object();
    j.insert("code", Value::string(reason_code_to_string(reason.code)));
    j.insert("message", Value::string(&reason.message));
    if let Some(detail) = &reason.detail {
        j.insert("detail", serialize_reason_detail(detail));
    }
    j
}

/**
 * @fn serialize_run_diagnostics(&RunDiagnostics) -> Value
 * @brief emit confidence values as doubles and counters as integers.
 * @author Alex (https://github.com/lextpf)
 *
 * the split decides the emitted text, because the same zero dumps as `0.0` on one side and `0` on
 * the other.
 */
pub fn serialize_run_diagnostics(run: &RunDiagnostics) -> Value {
    let mut j = Value::object();
    j.insert("confidence", serialize_confidence(&run.confidence));
    j.insert("exact_match", Value::Bool(run.exact_match));
    j.insert("phase_reached", Value::string(&run.phase_reached));
    j.insert("nodes_explored", Value::Int(run.nodes_explored as i64));

    let mut groups = Value::object();
    groups.insert("total", Value::Int(run.groups.total as i64));
    groups.insert(
        "resolved_by_propagation",
        Value::Int(run.groups.resolved_by_propagation as i64),
    );
    groups.insert(
        "resolved_by_csp",
        Value::Int(run.groups.resolved_by_csp as i64),
    );
    j.insert("groups", groups);

    let mut repro = Value::object();
    repro.insert("missing", Value::Int(run.repro.missing as i64));
    repro.insert("extra", Value::Int(run.repro.extra as i64));
    repro.insert("size_mismatch", Value::Int(run.repro.size_mismatch as i64));
    repro.insert("hash_mismatch", Value::Int(run.repro.hash_mismatch as i64));
    repro.insert("reproduced", Value::Int(run.repro.reproduced as i64));
    j.insert("repro", repro);

    let mut timings = Value::object();
    timings.insert("list", Value::Int(run.timings.list_ms));
    timings.insert("scan", Value::Int(run.timings.scan_ms));
    timings.insert("solve", Value::Int(run.timings.solve_ms));
    timings.insert("total", Value::Int(run.timings.total_ms));
    j.insert("timings_ms", timings);

    let mut cache = Value::object();
    cache.insert("hit", Value::Bool(run.cache.hit));
    cache.insert("source", Value::string(&run.cache.source));
    j.insert("cache", cache);

    j
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_code_int_values_match_cpp() {
        let cases: &[(ReasonCode, i32)] = &[
            (ReasonCode::ImplicitDefault, 0),
            (ReasonCode::ForcedRequired, 100),
            (ReasonCode::ForcedNotUsable, 101),
            (ReasonCode::ForcedSelectAll, 102),
            (ReasonCode::ForcedAtLeastOne, 103),
            (ReasonCode::ForcedExactlyOne, 104),
            (ReasonCode::UniqueFileEvidence, 200),
            (ReasonCode::NoFileEvidence, 201),
            (ReasonCode::NoUniqueEvidence, 202),
            (ReasonCode::CardinalityForced, 300),
            (ReasonCode::CspPhaseGreedy, 400),
            (ReasonCode::CspPhaseLocalSearch, 401),
            (ReasonCode::CspPhaseBacktrack, 402),
            (ReasonCode::CspPhaseRepair, 403),
            (ReasonCode::CspPhaseFocused, 404),
            (ReasonCode::CspPhaseFallback, 405),
            (ReasonCode::ConditionForcedTrue, 500),
            (ReasonCode::ConditionForcedFalse, 501),
            (ReasonCode::ConditionUnknown, 502),
            (ReasonCode::StepVisibilityForced, 510),
            (ReasonCode::StepVisibilityUnknown, 511),
            (ReasonCode::StepNotVisible, 512),
            (ReasonCode::ExtraFileProduced, 600),
            (ReasonCode::FomodPlusCache, 700),
        ];
        for (code, value) in cases {
            assert_eq!(*code as i32, *value, "{code:?}");
        }
    }

    #[test]
    fn default_is_implicit_default() {
        assert_eq!(ReasonCode::default(), ReasonCode::ImplicitDefault);
        assert_eq!(ReasonCode::default() as i32, 0);
    }

    #[test]
    fn reason_code_to_string_matches_cpp() {
        let cases: &[(ReasonCode, &str)] = &[
            (ReasonCode::ImplicitDefault, "IMPLICIT_DEFAULT"),
            (ReasonCode::ForcedRequired, "FORCED_REQUIRED"),
            (ReasonCode::ForcedNotUsable, "FORCED_NOT_USABLE"),
            (ReasonCode::ForcedSelectAll, "FORCED_SELECT_ALL"),
            (ReasonCode::ForcedAtLeastOne, "FORCED_AT_LEAST_ONE"),
            (ReasonCode::ForcedExactlyOne, "FORCED_EXACTLY_ONE"),
            (ReasonCode::UniqueFileEvidence, "UNIQUE_FILE_EVIDENCE"),
            (ReasonCode::NoFileEvidence, "NO_FILE_EVIDENCE"),
            (ReasonCode::NoUniqueEvidence, "NO_UNIQUE_EVIDENCE"),
            (ReasonCode::CardinalityForced, "CARDINALITY_FORCED"),
            (ReasonCode::CspPhaseGreedy, "CSP_PHASE_GREEDY"),
            (ReasonCode::CspPhaseLocalSearch, "CSP_PHASE_LOCAL_SEARCH"),
            (ReasonCode::CspPhaseBacktrack, "CSP_PHASE_BACKTRACK"),
            (ReasonCode::CspPhaseRepair, "CSP_PHASE_REPAIR"),
            (ReasonCode::CspPhaseFocused, "CSP_PHASE_FOCUSED"),
            (ReasonCode::CspPhaseFallback, "CSP_PHASE_FALLBACK"),
            (ReasonCode::ConditionForcedTrue, "CONDITION_FORCED_TRUE"),
            (ReasonCode::ConditionForcedFalse, "CONDITION_FORCED_FALSE"),
            (ReasonCode::ConditionUnknown, "CONDITION_UNKNOWN"),
            (ReasonCode::StepVisibilityForced, "STEP_VISIBILITY_FORCED"),
            (ReasonCode::StepVisibilityUnknown, "STEP_VISIBILITY_UNKNOWN"),
            (ReasonCode::StepNotVisible, "STEP_NOT_VISIBLE"),
            (ReasonCode::ExtraFileProduced, "EXTRA_FILE_PRODUCED"),
            (ReasonCode::FomodPlusCache, "FOMOD_PLUS_CACHE"),
        ];
        for (code, name) in cases {
            assert_eq!(reason_code_to_string(*code), *name, "{code:?}");
        }
    }

    #[test]
    fn reason_detail_unique_file_evidence_holds_files_and_count() {
        let detail = ReasonDetail::UniqueFileEvidence {
            files: vec!["a.dds".to_string(), "b.dds".to_string()],
            count: 5,
        };
        match detail {
            ReasonDetail::UniqueFileEvidence { files, count } => {
                assert_eq!(files, vec!["a.dds".to_string(), "b.dds".to_string()]);
                assert_eq!(count, 5);
            }
            other => panic!("unexpected variant {other:?}"),
        }
    }

    #[test]
    fn reason_detail_csp_phase_holds_nodes_and_phase() {
        let detail = ReasonDetail::CspPhase {
            nodes: 42,
            phase: "csp.greedy".to_string(),
        };
        match detail {
            ReasonDetail::CspPhase { nodes, phase } => {
                assert_eq!(nodes, 42);
                assert_eq!(phase, "csp.greedy");
            }
            other => panic!("unexpected variant {other:?}"),
        }
    }

    #[test]
    fn band_for_thresholds() {
        assert_eq!(band_for(1.0), "high");
        assert_eq!(band_for(0.85), "high"); // inclusive lower bound of "high"
        assert_eq!(band_for(0.8499999999999999), "medium");
        assert_eq!(band_for(0.5), "medium"); // inclusive lower bound of "medium"
        assert_eq!(band_for(0.4999999999999999), "low");
        assert_eq!(band_for(0.0), "low");
    }

    #[test]
    fn clamp01_bounds() {
        assert_eq!(clamp01(-0.5), 0.0);
        assert_eq!(clamp01(1.5), 1.0);
        assert_eq!(clamp01(0.3), 0.3);
        assert_eq!(clamp01(0.0), 0.0);
        assert_eq!(clamp01(1.0), 1.0);
    }

    #[test]
    fn weighted_mean_zero_weight_guard() {
        assert_eq!(weighted_mean(5.0, 0.0), 1.0);
        assert_eq!(weighted_mean(5.0, -1.0), 1.0);
        assert_eq!(weighted_mean(2.0, 4.0), 0.5);
    }

    #[test]
    fn ambiguity_component_table() {
        assert_eq!(ambiguity_component(0), 1.0);
        assert_eq!(ambiguity_component(-1), 1.0);
        assert_eq!(ambiguity_component(1), 0.6);
        assert_eq!(ambiguity_component(2), 0.4);
        assert_eq!(ambiguity_component(3), 0.2);
        assert_eq!(ambiguity_component(100), 0.2);
    }

    #[test]
    fn composite_from_all_ones_bits() {
        assert_eq!(
            crate::json::format_double(composite_from(&ConfidenceComponents::default())),
            "0.9999999999999999"
        );
    }

    #[test]
    fn evidence_component_first_match_and_fallthrough() {
        let with = |code: ReasonCode, selected: bool| PluginDiagnostics {
            selected,
            reasons: vec![Reason {
                code,
                message: String::new(),
                detail: None,
            }],
            ..PluginDiagnostics::default()
        };
        assert_eq!(
            evidence_component(&with(ReasonCode::NoUniqueEvidence, true), true),
            1.0
        );
        assert_eq!(
            evidence_component(&with(ReasonCode::UniqueFileEvidence, false), false),
            1.0
        );
        assert_eq!(
            evidence_component(&with(ReasonCode::NoUniqueEvidence, false), false),
            0.5
        );
        assert_eq!(
            evidence_component(&with(ReasonCode::ExtraFileProduced, false), false),
            0.3
        );
        let bare = |selected: bool| PluginDiagnostics {
            selected,
            ..PluginDiagnostics::default()
        };
        assert_eq!(evidence_component(&bare(true), false), 0.5);
        assert_eq!(evidence_component(&bare(false), false), 0.7);
    }

    use crate::fomod_ir::{FomodFileEntry, FomodGroup, FomodGroupType, FomodPlugin, FomodStep};
    use crate::json::format_double;

    fn plugin_with_files(name: &str, n: usize) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            files: (0..n)
                .map(|i| FomodFileEntry {
                    source: format!("s{i}"),
                    destination: format!("d{i}"),
                    ..FomodFileEntry::default()
                })
                .collect(),
            ..FomodPlugin::default()
        }
    }

    fn one_group_installer(plugins: Vec<FomodPlugin>) -> FomodInstaller {
        FomodInstaller {
            steps: vec![FomodStep {
                name: "S".to_string(),
                groups: vec![FomodGroup {
                    name: "G".to_string(),
                    r#type: FomodGroupType::SelectExactlyOne,
                    plugins,
                }],
                ..FomodStep::default()
            }],
            ..FomodInstaller::default()
        }
    }

    #[test]
    fn mu_joint_fix_worked_example_group_composite() {
        let installer = one_group_installer(vec![
            plugin_with_files("SE_AE", 1),
            plugin_with_files("VR", 1),
        ]);
        let result = SolverResult {
            selections: vec![vec![vec![true, false]]],
            exact_match: true,
            ..SolverResult::default()
        };
        let mut b = InferenceDiagnosticsBuilder::new(&installer);
        b.absorb_solver(&result);
        b.finalize(&result, &PropagationResult::default(), &installer);

        let g = &b.diagnostics().steps[0].groups[0];
        assert_eq!(g.plugins[0].confidence.components.evidence, 0.5);
        assert_eq!(g.plugins[1].confidence.components.evidence, 0.7);
        assert_eq!(format_double(g.plugins[0].confidence.composite), "0.5");
        assert_eq!(format_double(g.plugins[1].confidence.composite), "0.58");
        assert_eq!(format_double(g.confidence.components.evidence), "0.6");
        assert_eq!(format_double(g.confidence.composite), "0.54");
        assert_eq!(g.confidence.band, "medium");
    }

    #[test]
    fn all_forced_group_short_circuits_to_one() {
        // a single propagation-forced plugin: the group composite is exactly 1.0 (all-forced
        // short-circuit) even though the plugin composite is the 0.9999999999999999 the weighted
        // formula would produce.
        let installer = one_group_installer(vec![plugin_with_files("A", 1)]);
        let result = SolverResult {
            selections: vec![vec![vec![true]]],
            exact_match: true,
            ..SolverResult::default()
        };
        let mut b = InferenceDiagnosticsBuilder::new(&installer);
        b.add_plugin_reason(0, 0, 0, ReasonCode::ForcedSelectAll, "x", None);
        b.absorb_solver(&result);
        b.finalize(&result, &PropagationResult::default(), &installer);

        let g = &b.diagnostics().steps[0].groups[0];
        assert_eq!(g.confidence.composite, 1.0);
        assert_eq!(g.confidence.band, "high");
        assert_eq!(
            format_double(g.plugins[0].confidence.composite),
            "0.9999999999999999"
        );
    }

    fn run_composite(extra: i32, phase: &str) -> f64 {
        let installer = one_group_installer(vec![plugin_with_files("A", 1)]);
        let result = SolverResult {
            selections: vec![vec![vec![true]]],
            exact_match: true,
            extra,
            phase_reached: phase.to_string(),
            ..SolverResult::default()
        };
        let mut b = InferenceDiagnosticsBuilder::new(&installer);
        b.add_plugin_reason(0, 0, 0, ReasonCode::ForcedSelectAll, "x", None);
        b.absorb_solver(&result);
        b.finalize(&result, &PropagationResult::default(), &installer);
        b.diagnostics().run.confidence.composite
    }

    #[test]
    fn run_level_extra_and_fallback_penalties() {
        let base = run_composite(0, "");
        assert!((base - run_composite(3, "") - 0.15).abs() < 1e-12);
        assert!((base - run_composite(10, "") - 0.25).abs() < 1e-12);
        assert!((base - run_composite(0, "csp.fallback") - 0.10).abs() < 1e-12);
        assert_eq!(base, run_composite(0, "csp.greedy"));
    }

    #[test]
    fn reproduced_backfill_both_branches() {
        {
            let installer = one_group_installer(vec![plugin_with_files("A", 1)]);
            let result = SolverResult {
                selections: vec![vec![vec![true]]],
                exact_match: false,
                missing: 35,
                ..SolverResult::default()
            };
            let mut b = InferenceDiagnosticsBuilder::new(&installer);
            b.absorb_solver(&result);
            b.set_target_file_count(36);
            b.finalize(&result, &PropagationResult::default(), &installer);
            assert_eq!(b.diagnostics().run.repro.reproduced, 1);
        }
        {
            let installer = one_group_installer(vec![plugin_with_files("A", 3)]);
            let result = SolverResult {
                selections: vec![vec![vec![true]]],
                exact_match: false,
                missing: 1,
                ..SolverResult::default()
            };
            let mut b = InferenceDiagnosticsBuilder::new(&installer);
            b.absorb_solver(&result);
            b.finalize(&result, &PropagationResult::default(), &installer);
            assert_eq!(b.diagnostics().run.repro.reproduced, 2);
        }
    }

    #[test]
    fn serialize_reason_key_and_detail_ordering() {
        let r = Reason {
            code: ReasonCode::UniqueFileEvidence,
            message: "m".to_string(),
            detail: Some(ReasonDetail::UniqueFileEvidence {
                files: vec!["a".to_string(), "b".to_string()],
                count: 2,
            }),
        };
        let dumped = serialize_reason(&r).dump(2);
        let expected = "{\n  \"code\": \"UNIQUE_FILE_EVIDENCE\",\n  \"detail\": {\n    \"count\": 2,\n    \"files\": [\n      \"a\",\n      \"b\"\n    ]\n  },\n  \"message\": \"m\"\n}";
        assert_eq!(dumped, expected);
    }

    #[test]
    fn serialize_reason_without_detail_skips_key() {
        let r = Reason {
            code: ReasonCode::NoFileEvidence,
            message: "m".to_string(),
            detail: None,
        };
        assert_eq!(
            serialize_reason(&r).dump(2),
            "{\n  \"code\": \"NO_FILE_EVIDENCE\",\n  \"message\": \"m\"\n}"
        );
    }

    #[test]
    fn serialize_csp_phase_detail_keys_sort_nodes_then_phase() {
        let r = Reason {
            code: ReasonCode::CspPhaseGreedy,
            message: "Resolved in greedy phase".to_string(),
            detail: Some(ReasonDetail::CspPhase {
                nodes: 1,
                phase: "csp.greedy".to_string(),
            }),
        };
        let expected = "{\n  \"code\": \"CSP_PHASE_GREEDY\",\n  \"detail\": {\n    \"nodes\": 1,\n    \"phase\": \"csp.greedy\"\n  },\n  \"message\": \"Resolved in greedy phase\"\n}";
        assert_eq!(serialize_reason(&r).dump(2), expected);
    }

    #[test]
    fn serialize_confidence_key_order_and_float_format() {
        let score = ConfidenceScore {
            composite: 0.54,
            band: "medium".to_string(),
            components: ConfidenceComponents {
                evidence: 0.6,
                propagation: 0.0,
                repro: 1.0,
                ambiguity: 1.0,
            },
        };
        let expected = "{\n  \"band\": \"medium\",\n  \"components\": {\n    \"ambiguity\": 1.0,\n    \"evidence\": 0.6,\n    \"propagation\": 0.0,\n    \"repro\": 1.0\n  },\n  \"composite\": 0.54\n}";
        assert_eq!(serialize_confidence(&score).dump(2), expected);
    }
}
