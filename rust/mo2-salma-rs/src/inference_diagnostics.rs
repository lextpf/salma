//! Inference diagnostics - partial Rust port of `src/InferenceDiagnostics.hpp`
//! / `.cpp`.
//!
//! Task 7 ports only the pieces the constraint propagator (the first consumer
//! of these codes) needs:
//!
//! - [`ReasonCode`] - the stable, integer-backed enumeration of reasons the
//!   inference engine attaches to a plugin/group/step decision. Mirror of
//!   `mo2core::ReasonCode` in `src/InferenceDiagnostics.hpp`. The WHOLE enum is
//!   ported now (not just the `FORCED_*` / `*_FILE_EVIDENCE` codes the
//!   propagator emits) because the CSP solver (Tasks 8-9) and the diagnostics
//!   assembler (Task 10) reference the `CSP_PHASE_*` / `CONDITION_*` / `STEP_*`
//!   / penalty / cache codes and the integer values must stay stable.
//! - [`reason_code_to_string`] - stable string name for each code, mirror of
//!   `mo2core::reason_code_to_string`.
//! - [`ReasonDetail`] - typed replacement for the C++ `nlohmann::json` detail
//!   payload carried alongside a plugin reason. The propagator emits exactly
//!   one variant ([`ReasonDetail::UniqueFileEvidence`]).
//!
//! The remainder of `InferenceDiagnostics.hpp` (the `Reason` struct, the
//! `ConfidenceComponents` / `ConfidenceScore` scoring types, the
//! `InferenceDiagnosticsBuilder` accumulator, and the `serialize_*` / schema-v2
//! JSON helpers) lands with the diagnostics port in Task 10, which will EXTEND
//! this module rather than replace it. Task 10 owns the ReasonDetail ->
//! schema-v2 JSON mapping; no JSON model is introduced here.

use crate::fomod_csp_types::{ReproMetrics, SolverResult};
use crate::fomod_ir::FomodInstaller;
use crate::fomod_propagator::PropagationResult;
use crate::json::Value;
use crate::logger::Logger;

/// Stable enumeration of reasons the inference engine can attach to a plugin,
/// group, or step decision. Mirror of `mo2core::ReasonCode` in
/// `src/InferenceDiagnostics.hpp`.
///
/// Codes are integer-backed (`#[repr(i32)]`) with the EXACT C++ integer values
/// and are stable across releases: the dashboard maps them to UI labels and
/// never compares against the human message. New codes are appended; existing
/// codes never change their integer value or semantics. Read the numeric value
/// with `code as i32` (the C++ `PropagationResult` stores `int` only to dodge an
/// include cycle - the Rust port has no such cycle and stores the enum directly).
///
/// Codes are grouped by the kind of decision that produced them:
///
/// - `Forced*` come from FOMOD spec constraints (Required, NotUsable, etc.)
///   evaluated by the propagator's plugin-type rule.
/// - `*FileEvidence` / `NoFileEvidence` come from the target-tree evidence rule.
/// - `CardinalityForced` comes from the group-type cardinality rule.
/// - `CspPhase*` indicate which phase of the CSP solver made the pick.
/// - `Condition*` and `Step*` describe override and visibility decisions.
/// - `FomodPlusCache` is set when inference was lifted from a cached `meta.ini`
///   Tier-1 entry.
/// - `ImplicitDefault` is the catch-all when no other reason applies.
///
/// The variant identifiers are Rust UpperCamelCase; their stable wire names
/// (the C++ SCREAMING_SNAKE enumerator text) are produced by
/// [`reason_code_to_string`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ReasonCode {
    /// No explicit reason recorded yet (default for unfilled entries).
    #[default]
    ImplicitDefault = 0,

    // Forced by plugin-type constraints (Rule P).
    /// Plugin type Required pinned the selection.
    ForcedRequired = 100,
    /// Plugin type NotUsable eliminated the selection.
    ForcedNotUsable = 101,
    /// SelectAll group forced this plugin on.
    ForcedSelectAll = 102,
    /// AtLeastOne group with one valid combo.
    ForcedAtLeastOne = 103,
    /// SelectExactlyOne with one valid combo.
    ForcedExactlyOne = 104,

    // File-evidence rules (Rule F).
    /// Plugin uniquely produces a target file; detail lists files.
    UniqueFileEvidence = 200,
    /// All plugin files absent from target; eliminated.
    NoFileEvidence = 201,
    /// Deselected: nothing in target uniquely maps here.
    NoUniqueEvidence = 202,

    // Cardinality rules (Rule C).
    /// Group narrowed to a single combo by group type.
    CardinalityForced = 300,

    // CSP solver phases.
    /// Picked by the greedy phase.
    CspPhaseGreedy = 400,
    /// Improved/picked by local search.
    CspPhaseLocalSearch = 401,
    /// Picked by systematic backtracking.
    CspPhaseBacktrack = 402,
    /// Picked by the residual-repair phase.
    CspPhaseRepair = 403,
    /// Picked by the focused-search phase.
    CspPhaseFocused = 404,
    /// Picked by the global-fallback phase.
    CspPhaseFallback = 405,

    // Condition / step visibility overrides.
    /// compute_overrides forced a conditional pattern true.
    ConditionForcedTrue = 500,
    /// compute_overrides forced it false.
    ConditionForcedFalse = 501,
    /// Could not determine; the simulator guessed.
    ConditionUnknown = 502,
    /// Step visibility condition forced true.
    StepVisibilityForced = 510,
    /// Could not determine step visibility.
    StepVisibilityUnknown = 511,
    /// Step skipped entirely because not visible.
    StepNotVisible = 512,

    // Penalties / scoring.
    /// Selection produces a file not in target (penalty).
    ExtraFileProduced = 600,

    // Cache / shortcut.
    /// Selection lifted from meta.ini Tier-1 cache.
    FomodPlusCache = 700,
}

/// Convert a [`ReasonCode`] to its stable string name (e.g. `"FORCED_REQUIRED"`).
/// Mirror of `mo2core::reason_code_to_string`.
///
/// The returned string is the C++ enum-identifier text (SCREAMING_SNAKE),
/// suitable for the JSON wire format and for grepping logs. The C++ function
/// returns `"UNKNOWN"` for an unrecognized enumerator; under Rust's exhaustive
/// match over a closed enum that miss-branch is unreachable, so it is encoded
/// as the absence of any wildcard arm rather than a live `"UNKNOWN"` return.
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

/// Typed structured payload accompanying a plugin reason. Rust replacement for
/// the C++ `nlohmann::json` `detail` field carried on each plugin reason.
///
/// The constraint propagator emits [`ReasonDetail::UniqueFileEvidence`] (the
/// target files a plugin uniquely produces); the diagnostics assembler's
/// [`InferenceDiagnosticsBuilder::absorb_solver`] emits
/// [`ReasonDetail::CspPhase`] (which solver phase fixed a selection and the node
/// count at that point). Task 10 maps each variant to its schema-v2 JSON shape.
/// A missing detail is represented as `None` at the storage site rather than an
/// empty variant here, mirroring the C++ null-json sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasonDetail {
    /// Positive file evidence: the plugin uniquely produces at least one target
    /// file. `files` lists up to the first four example destinations (sorted
    /// byte-ascending in the port for determinism); `count` is the full number
    /// of unique target hits, which may exceed `files.len()`.
    UniqueFileEvidence {
        /// Up to four example destination paths (byte-ascending).
        files: Vec<String>,
        /// Total count of unique target hits (may exceed `files.len()`).
        count: i32,
    },
    /// CSP-phase attribution: the plugin was selected by a solver phase. Emitted
    /// by [`InferenceDiagnosticsBuilder::absorb_solver`] onto every selected
    /// plugin of a group whose `phase_per_group` entry is non-empty. Mirror of
    /// the C++ `detail["phase"] = phase_id; detail["nodes"] = nodes_explored`
    /// object (`src/InferenceDiagnostics.cpp:612-614`); its schema-v2 keys sort
    /// to `nodes` then `phase`.
    CspPhase {
        /// The CSP search-tree node count at the point of the pick
        /// (`SolverResult::nodes_explored`, a run-level total).
        nodes: i32,
        /// Stable phase identifier (e.g. `"csp.greedy"`, `"csp.fallback"`).
        phase: String,
    },
}

// ---------------------------------------------------------------------------
// Confidence formula constants (mirror of the anonymous-namespace `constexpr`
// block in `src/InferenceDiagnostics.cpp:18-30`).
// ---------------------------------------------------------------------------

/// Weight of the evidence component in the per-plugin composite (0.40).
const WEIGHT_EVIDENCE: f64 = 0.40;
/// Weight of the propagation component (0.30).
const WEIGHT_PROPAGATION: f64 = 0.30;
/// Weight of the repro component (0.20).
const WEIGHT_REPRO: f64 = 0.20;
/// Weight of the ambiguity component (0.10).
const WEIGHT_AMBIGUITY: f64 = 0.10;

/// Composite score at/above which the band is `"high"` (0.85).
const BAND_HIGH_THRESHOLD: f64 = 0.85;
/// Composite score at/above which the band is `"medium"` (0.50).
const BAND_MEDIUM_THRESHOLD: f64 = 0.50;

/// Run-level composite penalty per extra (unexpected) file (0.05).
const RUN_EXTRA_PENALTY_PER_FILE: f64 = 0.05;
/// Cap on the number of extra files penalized at the run level (5).
const RUN_EXTRA_PENALTY_CAP: i32 = 5;
/// Run-level composite penalty when the solver reached the global fallback
/// (0.10).
const RUN_FALLBACK_PENALTY: f64 = 0.10;

// ---------------------------------------------------------------------------
// Diagnostic data model (mirror of the structs in `InferenceDiagnostics.hpp`).
// ---------------------------------------------------------------------------

/// A single justification attached to a plugin/group/step decision. Mirror of
/// `mo2core::Reason`.
///
/// The C++ carries `detail` as a `nlohmann::json` (null when absent); the port
/// uses `Option<ReasonDetail>`. `None` reproduces the C++ null-json sentinel and
/// is skipped by [`serialize_reason`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    /// Stable machine-readable reason code.
    pub code: ReasonCode,
    /// Human-readable explanation.
    pub message: String,
    /// Optional structured payload; `None` when self-explanatory.
    pub detail: Option<ReasonDetail>,
}

/// Per-axis confidence breakdown, each component in `[0.0, 1.0]`. Mirror of
/// `mo2core::ConfidenceComponents`. Every field defaults to 1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfidenceComponents {
    /// Fraction of plugin files that uniquely match the target.
    pub evidence: f64,
    /// 1.0 if forced by propagation, 0.0 if CSP-decided.
    pub propagation: f64,
    /// 1 - (group-local mismatches / group-local targets).
    pub repro: f64,
    /// 1 - (close-evidence alternatives / total alternatives).
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

/// Composite confidence score with a derived band. Mirror of
/// `mo2core::ConfidenceScore`. Defaults to composite 1.0, band `"high"`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfidenceScore {
    /// Linear combination of `components`.
    pub composite: f64,
    /// Categorical bucket: `"high"` / `"medium"` / `"low"`.
    pub band: String,
    /// Per-axis breakdown.
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

/// Diagnostics for one plugin in one group. Mirror of
/// `mo2core::PluginDiagnostics`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginDiagnostics {
    /// Mirrors the boolean stored in `SolverResult::selections` for this slot.
    pub selected: bool,
    /// Confidence score for this plugin decision.
    pub confidence: ConfidenceScore,
    /// Reason chain in evaluation order.
    pub reasons: Vec<Reason>,
}

/// Diagnostics for one group in one step. Mirror of `mo2core::GroupDiagnostics`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupDiagnostics {
    /// Confidence score aggregated over this group's plugins.
    pub confidence: ConfidenceScore,
    /// Rule or solver phase that fixed the last unresolved plugin (empty ==
    /// unknown).
    pub resolved_by: String,
    /// Group-level reason chain.
    pub reasons: Vec<Reason>,
    /// Per-plugin diagnostics.
    pub plugins: Vec<PluginDiagnostics>,
}

/// Diagnostics for one installation step. Mirror of `mo2core::StepDiagnostics`.
/// `visible` defaults to `true`.
#[derive(Debug, Clone, PartialEq)]
pub struct StepDiagnostics {
    /// Confidence score aggregated over this step's groups.
    pub confidence: ConfidenceScore,
    /// Step-level reason chain (visibility decisions).
    pub reasons: Vec<Reason>,
    /// Whether the step is visible at install time.
    pub visible: bool,
    /// Per-group diagnostics.
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

/// Per-pipeline timings in milliseconds. Mirror of `mo2core::DiagnosticTimings`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticTimings {
    /// Archive-listing time.
    pub list_ms: i64,
    /// Installed-file scan time.
    pub scan_ms: i64,
    /// CSP solve time.
    pub solve_ms: i64,
    /// End-to-end inference time.
    pub total_ms: i64,
}

/// Group-resolution counters. Mirror of `mo2core::DiagnosticGroupCounts`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticGroupCounts {
    /// Total groups across all steps.
    pub total: i32,
    /// Groups resolved by propagation (or the Tier-1 cache).
    pub resolved_by_propagation: i32,
    /// Groups resolved by the CSP solver.
    pub resolved_by_csp: i32,
}

/// Cache-hit context. Mirror of `mo2core::DiagnosticCacheInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticCacheInfo {
    /// True when inference short-circuited via the Tier-1 meta.ini cache.
    pub hit: bool,
    /// Cache origin (currently `"fomod-plus"` when set).
    pub source: String,
}

/// Run-level diagnostic summary. Mirror of `mo2core::RunDiagnostics`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunDiagnostics {
    /// Whole-run composite confidence.
    pub confidence: ConfidenceScore,
    /// True iff the install reproduced the target tree exactly.
    pub exact_match: bool,
    /// Highest CSP phase that contributed (or `"tier1_cache"`).
    pub phase_reached: String,
    /// Total CSP search-tree nodes explored.
    pub nodes_explored: i32,
    /// Group-resolution counters.
    pub groups: DiagnosticGroupCounts,
    /// Reproduction metrics.
    pub repro: ReproMetrics,
    /// Pipeline timings.
    pub timings: DiagnosticTimings,
    /// Cache-hit context.
    pub cache: DiagnosticCacheInfo,
}

/// Top-level diagnostics tree mirroring the FOMOD installer hierarchy. Mirror
/// of `mo2core::InferenceDiagnostics`. `schema_version` defaults to 2.
#[derive(Debug, Clone, PartialEq)]
pub struct InferenceDiagnostics {
    /// Wire-format schema version (2 for this struct).
    pub schema_version: i32,
    /// Run-level summary.
    pub run: RunDiagnostics,
    /// Per-step diagnostics.
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

// ---------------------------------------------------------------------------
// Confidence-formula helpers (mirror of the file-scoped helpers in
// `src/InferenceDiagnostics.cpp:38-230`).
// ---------------------------------------------------------------------------

/// Band for a composite score. Mirror of `band_for`.
fn band_for(composite: f64) -> &'static str {
    if composite >= BAND_HIGH_THRESHOLD {
        "high"
    } else if composite >= BAND_MEDIUM_THRESHOLD {
        "medium"
    } else {
        "low"
    }
}

/// Clamp to `[0.0, 1.0]`. Mirror of `clamp01`.
///
/// `f64::clamp(0.0, 1.0)` is behaviorally identical to the C++ branch form for
/// every value the confidence formula produces: below-range -> 0.0,
/// above-range -> 1.0, in-range -> unchanged, and (never occurring here) NaN ->
/// NaN. The bounds are finite constants, so `clamp` cannot panic.
fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// Weighted mean with a `weight <= 0 -> 1.0` guard. Mirror of `weighted_mean`.
fn weighted_mean(sum: f64, weight: f64) -> f64 {
    if weight <= 0.0 { 1.0 } else { sum / weight }
}

/// Combine the four components into a composite. Mirror of `composite_from`.
///
/// The multiply-add expression order is preserved EXACTLY so the IEEE-754 bits
/// match the C++ (e.g. all-ones yields `0.9999999999999999`, not `1.0`). No FMA
/// contraction happens in either language for `a * b + c` unless requested, so
/// the two produce identical doubles.
fn composite_from(c: &ConfidenceComponents) -> f64 {
    clamp01(
        WEIGHT_EVIDENCE * c.evidence
            + WEIGHT_PROPAGATION * c.propagation
            + WEIGHT_REPRO * c.repro
            + WEIGHT_AMBIGUITY * c.ambiguity,
    )
}

/// True if any reason in the chain is a propagation-forcing code. Mirror of
/// `plugin_was_propagation_forced`.
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

/// Map a CSP phase id to its reason code. Mirror of `csp_phase_to_reason`.
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

/// Map a CSP phase id to its human message. Mirror of `csp_phase_message`.
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

/// Declared file count for a plugin (bounds-checked). Mirror of
/// `plugin_file_count`.
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

/// Per-plugin evidence component. Mirror of `evidence_component`: forced plugins
/// score 1.0; otherwise the first evidence reason wins
/// (`UniqueFileEvidence`->1.0, `NoUniqueEvidence`->0.5, `ExtraFileProduced`
/// ->0.3); with no evidence reason, selected plugins score 0.5, deselected 0.7.
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

/// Per-plugin propagation component. Mirror of `propagation_component`.
fn propagation_component(forced: bool) -> f64 {
    if forced { 1.0 } else { 0.0 }
}

/// Per-group ambiguity component. Mirror of `ambiguity_component`: 0 alts ->
/// 1.0, 1 -> 0.6, 2 -> 0.4, 3+ -> 0.2.
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

// ---------------------------------------------------------------------------
// InferenceDiagnosticsBuilder (mirror of the C++ accumulator).
// ---------------------------------------------------------------------------

/// Builder that accumulates per-decision reasons during inference and finalizes
/// the confidence formula. Mirror of `mo2core::InferenceDiagnosticsBuilder`.
///
/// Construction sizes the nested `steps`/`groups`/`plugins` vectors to the
/// installer hierarchy so `add_plugin_reason` can index directly. Reason
/// emission and metadata setters are write-once-or-append; [`finalize`] must run
/// last (subsequent setters are silently dropped, exactly as the C++
/// `finalized_` guard does).
///
/// [`finalize`]: InferenceDiagnosticsBuilder::finalize
#[derive(Debug, Clone)]
pub struct InferenceDiagnosticsBuilder {
    diag: InferenceDiagnostics,
    finalized: bool,
    target_file_count: i32,
}

impl InferenceDiagnosticsBuilder {
    /// Construct with the hierarchy sized to match `installer`. Mirror of the
    /// C++ constructor: every step/group/plugin slot is reachable and carries a
    /// default-initialized [`PluginDiagnostics`]; `run.groups.total` is the sum
    /// of group counts.
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

    /// Append a reason to a plugin's reason chain. Mirror of
    /// `add_plugin_reason`. Out-of-range indices (including negatives) are
    /// ignored; calls after [`finalize`](Self::finalize) are dropped.
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

    /// Append a reason to a group's reason chain. Mirror of `add_group_reason`.
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

    /// Set the rule or solver phase that resolved a group. Mirror of
    /// `set_group_resolved_by`.
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

    /// Record a step's visibility decision and its origin. Mirror of
    /// `set_step_visibility`.
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

    /// Record pipeline timings (milliseconds). Mirror of `set_run_timings`.
    pub fn set_run_timings(&mut self, list_ms: i64, scan_ms: i64, solve_ms: i64, total_ms: i64) {
        if self.finalized {
            return;
        }
        self.diag.run.timings.list_ms = list_ms;
        self.diag.run.timings.scan_ms = scan_ms;
        self.diag.run.timings.solve_ms = solve_ms;
        self.diag.run.timings.total_ms = total_ms;
    }

    /// Mark this run as a Tier-1 cache hit. Mirror of `set_cache_hit`: sets
    /// `phase_reached = "tier1_cache"`, every group's `resolved_by` to
    /// `"cache.fomod_plus"`, and pushes a `FOMOD_PLUS_CACHE` reason on every
    /// plugin.
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

    /// Record the target tree's file count for repro scoring. Mirror of
    /// `set_target_file_count`.
    pub fn set_target_file_count(&mut self, count: i32) {
        if self.finalized {
            return;
        }
        self.target_file_count = count;
    }

    /// Absorb propagation results into per-plugin and per-group reasons. Mirror
    /// of `absorb_propagation`.
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

        // Plugin reasons.
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

    /// Absorb solver results into per-group phase reasons, selection state, and
    /// group counts. Mirror of `absorb_solver`.
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

        // Per-group CSP phase -> reason on each selected plugin.
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

        // Mirror selection state into PluginDiagnostics.selected.
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

        // Group counts.
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

    /// Compute confidence components and bands top-down, then apply run-level
    /// penalties and backfill `reproduced`. Mirror of `finalize`. Must be called
    /// last.
    pub fn finalize(
        &mut self,
        result: &SolverResult,
        _propagation: &PropagationResult,
        installer: &FomodInstaller,
    ) {
        if self.finalized {
            return;
        }

        // Fraction of the target tree the chosen selections reproduce.
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

                // Group composite: all-forced short-circuits to 1.0, else the
                // file-count weighted mean of the four components.
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

            // Step composite: group-count weighted mean.
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

        // Run-level: step-count weighted mean + penalties.
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

        // Penalties.
        let extra_capped = result.extra.min(RUN_EXTRA_PENALTY_CAP);
        composite -= RUN_EXTRA_PENALTY_PER_FILE * extra_capped as f64;
        if self.diag.run.phase_reached == "csp.fallback" {
            composite -= RUN_FALLBACK_PENALTY;
        }
        self.diag.run.confidence.composite = clamp01(composite);
        let run_composite = self.diag.run.confidence.composite;
        self.diag.run.confidence.band = band_for(run_composite).to_string();

        // Backfill `reproduced` if not already populated.
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

        // One-line run summary. The C++ `{:.2f}` becomes `{:.2}`; both round the
        // exact binary value half-to-even, and this is a log line rather than
        // part of the JSON schema, so it is not a byte-parity surface.
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

    /// Access the accumulated diagnostics. Mirror of `diagnostics()`.
    pub fn diagnostics(&self) -> &InferenceDiagnostics {
        &self.diag
    }
}

// ---------------------------------------------------------------------------
// Serialization to the byte-faithful JSON model (mirror of `serialize_*`).
// ---------------------------------------------------------------------------

/// Serialize a [`ReasonDetail`] to its schema-v2 JSON object. The keys sort to
/// `count` then `files` (unique-file evidence) or `nodes` then `phase` (CSP
/// phase), matching the C++ `nlohmann::json` insert-then-sorted-emit behavior.
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

/// Serialize a single [`ConfidenceScore`]. Mirror of `serialize_confidence`.
/// Emitted keys sort to `band`, `components`, `composite`; the components object
/// sorts to `ambiguity`, `evidence`, `propagation`, `repro`.
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

/// Serialize a single [`Reason`]. Mirror of `serialize_reason`. Always emits
/// `code` and `message`; emits `detail` only when present (mirroring the C++
/// non-null-and-non-empty guard). Emitted keys sort to `code`, `detail`,
/// `message`.
pub fn serialize_reason(reason: &Reason) -> Value {
    let mut j = Value::object();
    j.insert("code", Value::string(reason_code_to_string(reason.code)));
    j.insert("message", Value::string(&reason.message));
    if let Some(detail) = &reason.detail {
        j.insert("detail", serialize_reason_detail(detail));
    }
    j
}

/// Serialize the [`RunDiagnostics`] summary. Mirror of
/// `serialize_run_diagnostics`.
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

    // --- integer values must match the C++ enum EXACTLY --------------------

    #[test]
    fn reason_code_int_values_match_cpp() {
        // Every value is pinned to src/InferenceDiagnostics.hpp lines 43-84.
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

    // --- string names must match the C++ enum-identifier text --------------

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

    // --- ReasonDetail carries files + full count ---------------------------

    #[test]
    fn reason_detail_unique_file_evidence_holds_files_and_count() {
        let detail = ReasonDetail::UniqueFileEvidence {
            files: vec!["a.dds".to_string(), "b.dds".to_string()],
            count: 5,
        };
        match detail {
            ReasonDetail::UniqueFileEvidence { files, count } => {
                assert_eq!(files, vec!["a.dds".to_string(), "b.dds".to_string()]);
                // count is the FULL number of hits, independent of files.len().
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

    // --- confidence-formula helpers (boundary tables) ----------------------

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
        // Load-bearing IEEE-754 sum: 0.40+0.30+0.20+0.10 != 1.0.
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
        // Forced short-circuits to 1.0 regardless of reasons.
        assert_eq!(
            evidence_component(&with(ReasonCode::NoUniqueEvidence, true), true),
            1.0
        );
        // First evidence reason wins.
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
        // No evidence reason -> selected 0.5 / deselected 0.7.
        let bare = |selected: bool| PluginDiagnostics {
            selected,
            ..PluginDiagnostics::default()
        };
        assert_eq!(evidence_component(&bare(true), false), 0.5);
        assert_eq!(evidence_component(&bare(false), false), 0.7);
    }

    // --- worked confidence examples through the builder --------------------

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
        // Two equal-weight plugins: SE_AE selected (non-forced -> evidence 0.5),
        // VR deselected (evidence 0.7); exact match so repro is 1.0. The group's
        // evidence averages to 0.6 and its composite to 0.54 (band "medium"),
        // matching `zip_exactlyone_mu_joint_fix/expected.json`.
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
        // A single propagation-forced plugin: the group composite is exactly 1.0
        // (all-forced short-circuit) even though the plugin composite is the
        // 0.9999999999999999 the weighted formula would produce.
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
        // extra=3 -> -0.05*3 = -0.15.
        assert!((base - run_composite(3, "") - 0.15).abs() < 1e-12);
        // extra is capped at 5 -> extra=10 still only -0.25.
        assert!((base - run_composite(10, "") - 0.25).abs() < 1e-12);
        // csp.fallback -> -0.10.
        assert!((base - run_composite(0, "csp.fallback") - 0.10).abs() < 1e-12);
        // A non-fallback phase carries no penalty.
        assert_eq!(base, run_composite(0, "csp.greedy"));
    }

    #[test]
    fn reproduced_backfill_both_branches() {
        // Target-derived branch: reproduced = target - missing - sm - hm.
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
        // Proxy branch (no target count): reproduced = selected file count -
        // missing, clamped at 0.
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

    // --- serialization key ordering + detail shapes ------------------------

    #[test]
    fn serialize_reason_key_and_detail_ordering() {
        // code, detail, message emitted in sorted key order; detail present.
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
