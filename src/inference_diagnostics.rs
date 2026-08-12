//! Diagnostics layer of the inference pipeline: the reason codes attached to
//! each decision, the confidence score derived from them, and the schema-v2
//! JSON they serialize to.
//!
//! - [`ReasonCode`] and [`reason_code_to_string`]: the integer-backed reason
//!   enumeration and its stable wire names.
//! - [`ReasonDetail`]: the structured payload carried beside a plugin reason.
//!   The propagator emits [`ReasonDetail::UniqueFileEvidence`]; the builder
//!   emits [`ReasonDetail::CspPhase`].
//! - The data model. [`InferenceDiagnostics`] (`schema_version` 2) holds
//!   [`RunDiagnostics`] plus a [`StepDiagnostics`] tree that descends through
//!   [`GroupDiagnostics`] to [`PluginDiagnostics`]. Every level carries a
//!   [`ConfidenceScore`] over [`ConfidenceComponents`] and a chain of
//!   [`Reason`]s; [`DiagnosticTimings`], [`DiagnosticGroupCounts`] and
//!   [`DiagnosticCacheInfo`] complete the run summary.
//! - [`InferenceDiagnosticsBuilder`], the accumulator. It absorbs the
//!   propagation result and the solver result, then computes the confidence
//!   formula in [`InferenceDiagnosticsBuilder::finalize`].
//! - [`serialize_confidence`], [`serialize_reason`] and
//!   [`serialize_run_diagnostics`], which build [`crate::json::Value`] trees
//!   for [`crate::fomod_inference_atoms::assemble_json`] and the Tier-1 emitter
//!   in [`crate::fomod_inference_service`].
//!
//! ## Wire stability
//!
//! Both the integer value and the string name of a [`ReasonCode`] are wire
//! format: the dashboard (`web/src/comps/WhyPanel.tsx`) keys its labels off the
//! names, never off the human message. Append new codes; never renumber or
//! repurpose an existing one.
//!
//! Six codes are reserved and have no producer anywhere in this crate, so they
//! never reach the wire: `NO_UNIQUE_EVIDENCE` (202), `CARDINALITY_FORCED`
//! (300), `CONDITION_FORCED_TRUE` (500), `CONDITION_FORCED_FALSE` (501),
//! `CONDITION_UNKNOWN` (502) and `EXTRA_FILE_PRODUCED` (600). The arms that
//! score or describe them, in `evidence_component` and in
//! [`InferenceDiagnosticsBuilder::absorb_propagation`], are dead paths rather
//! than live scoring. The table in the [`ReasonCode`] doc names the producer of
//! every code.
//!
//! ## The confidence formula
//!
//! [`InferenceDiagnosticsBuilder::finalize`] implements the block below, which
//! is the reference copy; nothing else in this file repeats it in full. Each
//! component is a plugin-level value. A group, step or run value on the same
//! axis is an aggregate of the level under it, never an independent
//! measurement.
//!
//! ```text
//! per plugin p, in group g of step s
//!   evidence    = 1.0                    if the reason chain is propagation-forced
//!               = first matching reason: UNIQUE_FILE_EVIDENCE -> 1.0
//!                                        NO_UNIQUE_EVIDENCE   -> 0.5  (no producer)
//!                                        EXTRA_FILE_PRODUCED  -> 0.3  (no producer)
//!               = 0.5 if selected, 0.7 if deselected   (no evidence reason at all)
//!   propagation = 1.0 if propagation-forced, else 0.0
//!   repro       = 1.0                    if deselected, or if the run matched exactly
//!               = 0.85 * repro_ratio     otherwise
//!   ambiguity   = alternatives_per_group[s][g]: 0 -> 1.0, 1 -> 0.6, 2 -> 0.4, 3+ -> 0.2
//!
//! run level, computed once
//!   repro_ratio = 1.0                    if exact_match, or target_file_count == 0
//!               = clamp01(1 - (missing + 0.5 * (size_mismatch + hash_mismatch))
//!                             / target_file_count)
//!
//! at every level
//!   composite   = clamp01(0.40*evidence + 0.30*propagation
//!                         + 0.20*repro  + 0.10*ambiguity)
//!   band        = "high" if composite >= 0.85
//!               = "medium" if composite >= 0.50
//!               = "low" otherwise
//!
//! aggregation, bottom-up, one weighted mean per axis (weight <= 0 -> 1.0)
//!   plugin -> group   weight = plugin_file_count + 1
//!                     a group whose plugins are all propagation-forced skips
//!                     the mean: all four components and the composite become 1.0
//!   group  -> step    weight = plugins.len() + 1
//!   step   -> run     weight = groups.len() + 1
//!
//! run composite, after the mean
//!   composite -= 0.05 * min(SolverResult::extra, 5)
//!   composite -= 0.10                    if phase_reached == "csp.fallback"
//!   composite  = clamp01(composite)      so the run composite is not recoverable
//!                                        from the four run components alone
//! ```
//!
//! Three constants in that block are easy to misread. The 0.5 gives a size or
//! hash mismatch half the weight of a miss, because the file exists at the
//! right destination and only its content or size differs. The 0.85 is a
//! deliberate ceiling on a selected plugin in a non-exact run, so such a plugin
//! never scores above 0.85 on the repro axis even when nothing is missing. The
//! `+ 1` in each aggregation weight stops a zero-file plugin, an empty group or
//! an empty step from carrying zero weight.

use crate::fomod_csp_types::{ReproMetrics, SolverResult};
use crate::fomod_ir::FomodInstaller;
use crate::fomod_propagator::PropagationResult;
use crate::json::Value;
use crate::logger::Logger;

/// Stable reason the inference engine attaches to a plugin, group or step
/// decision.
///
/// Codes are integer-backed (`#[repr(i32)]`) and stable across releases: the
/// dashboard maps the integer and the name to UI labels and never reads the
/// human message. New codes are appended; an existing code never changes its
/// integer value or its meaning. Read the numeric value with `code as i32`.
///
/// One row per code, naming what records it. "Reserved" means the integer is
/// claimed but nothing emits the code.
///
/// ```text
///  code                     value  recorded by
///  -----------------------  -----  -------------------------------------------
///  IMPLICIT_DEFAULT             0  default for an unfilled slot;
///                                  absorb_propagation skips it, and
///                                  csp_phase_to_reason returns it for an
///                                  unrecognised phase id
///  FORCED_REQUIRED            100  fomod_propagator rule 1 (plugin type)
///  FORCED_NOT_USABLE          101  fomod_propagator rule 1 (plugin type)
///  FORCED_SELECT_ALL          102  fomod_propagator rule 3 (cardinality)
///  FORCED_AT_LEAST_ONE        103  fomod_propagator rule 3 (cardinality)
///  FORCED_EXACTLY_ONE         104  fomod_propagator rule 3 (cardinality)
///  UNIQUE_FILE_EVIDENCE       200  fomod_propagator rule 2 (file evidence),
///                                  with a ReasonDetail::UniqueFileEvidence
///  NO_FILE_EVIDENCE           201  fomod_propagator rule 2 (file evidence)
///  NO_UNIQUE_EVIDENCE         202  reserved, no producer
///  CARDINALITY_FORCED         300  reserved, no producer; rule 3 records the
///                                  three FORCED_* codes instead
///  CSP_PHASE_GREEDY           400  absorb_solver, from phase_per_group
///  CSP_PHASE_LOCAL_SEARCH     401  absorb_solver
///  CSP_PHASE_BACKTRACK        402  absorb_solver
///  CSP_PHASE_REPAIR           403  absorb_solver
///  CSP_PHASE_FOCUSED          404  absorb_solver
///  CSP_PHASE_FALLBACK         405  absorb_solver
///  CONDITION_FORCED_TRUE      500  reserved, no producer
///  CONDITION_FORCED_FALSE     501  reserved, no producer
///  CONDITION_UNKNOWN          502  reserved, no producer
///  STEP_VISIBILITY_FORCED     510  fomod_inference_service, for a step whose
///                                  override is ForceTrue
///  STEP_VISIBILITY_UNKNOWN    511  fomod_inference_service, override Unknown
///  STEP_NOT_VISIBLE           512  fomod_inference_service, override ForceFalse
///  EXTRA_FILE_PRODUCED        600  reserved, no producer; only
///                                  evidence_component still scores it
///  FOMOD_PLUS_CACHE           700  set_cache_hit, on a Tier-1 meta.ini hit
/// ```
///
/// The propagator's numbered rules are listed in the
/// [`crate::fomod_propagator`] module doc: rule 1 evaluates plugin types, rule
/// 2 weighs file evidence, and rule 3 enforces cardinality once the group
/// resolves. Rule 3 is the one to watch, because the three cardinality codes
/// appear only after a group resolves, never during narrowing.
///
/// Variant identifiers are UpperCamelCase; the wire names are SCREAMING_SNAKE
/// and come from [`reason_code_to_string`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ReasonCode {
    /// No explicit reason recorded yet (default for unfilled entries).
    #[default]
    ImplicitDefault = 0,

    // Forced by plugin-type constraints (propagator rule 1).
    /// Plugin type Required pinned the selection on.
    ForcedRequired = 100,
    /// Plugin type NotUsable eliminated the plugin. Not recorded for a dynamic
    /// `dependencyType` evaluated without an external context: that outcome is
    /// not definitive enough to prune on.
    ForcedNotUsable = 101,

    // Forced by the cardinality rule (propagator rule 3), and only after the
    // group resolves. SelectAtMostOne and SelectAny resolve at zero usable
    // plugins and so record no Forced* code at all.
    /// SelectAll group forced this plugin on.
    ForcedSelectAll = 102,
    /// SelectAtLeastOne group left with exactly one usable plugin.
    ForcedAtLeastOne = 103,
    /// SelectExactlyOne group left with exactly one usable plugin.
    ForcedExactlyOne = 104,

    // File evidence (propagator rule 2).
    /// Plugin uniquely produces a target file; detail lists files.
    UniqueFileEvidence = 200,
    /// Every destination this plugin uniquely produces inside its group is
    /// absent from the target, so the plugin is eliminated. Only
    /// non-always-install, non-install-if-usable and non-excluded destinations
    /// count, and only those no other still-usable plugin of the same group
    /// produces. A plugin with no group-unique destination is never eliminated
    /// by this rule, however many of its files are absent.
    ///
    /// The message on the wire reads "All declared files absent from target",
    /// which is broader than the rule it describes. It is emitted text that the
    /// dashboard shows, so rewording it changes the document.
    NoFileEvidence = 201,
    /// Deselected because nothing in the target maps uniquely here. Reserved:
    /// nothing records this code.
    NoUniqueEvidence = 202,

    // Cardinality. Reserved: rule 3 records the three FORCED_* codes above
    // instead.
    /// Group narrowed to a single combination by its group type. Reserved: the
    /// cardinality rule records `ForcedSelectAll`, `ForcedExactlyOne` or
    /// `ForcedAtLeastOne` on the kept plugins instead.
    CardinalityForced = 300,

    // CSP solver phases.
    /// Picked by the greedy phase.
    CspPhaseGreedy = 400,
    /// Picked or improved by local search.
    CspPhaseLocalSearch = 401,
    /// Picked by systematic backtracking.
    CspPhaseBacktrack = 402,
    /// Picked by the residual-repair phase.
    CspPhaseRepair = 403,
    /// Picked by the focused-search phase.
    CspPhaseFocused = 404,
    /// Picked by the global-fallback phase.
    CspPhaseFallback = 405,

    // Condition and step-visibility overrides. `compute_overrides` decides
    // ForceTrue / ForceFalse / Unknown for conditional patterns, and the
    // forward simulator and the solver consume those decisions, but none of
    // them becomes a reason. Only its step-visibility half reaches the builder,
    // through the three STEP_* codes below.
    /// A conditional pattern was forced true. Reserved: no producer.
    ConditionForcedTrue = 500,
    /// A conditional pattern was forced false. Reserved: no producer.
    ConditionForcedFalse = 501,
    /// A conditional pattern could not be decided. Reserved: no producer.
    ConditionUnknown = 502,
    /// Step visibility condition forced true.
    StepVisibilityForced = 510,
    /// Could not determine step visibility.
    StepVisibilityUnknown = 511,
    /// Step skipped entirely because not visible.
    StepNotVisible = 512,

    // Penalties and scoring.
    /// The selection produces a file the target does not have. Reserved: no
    /// producer. `evidence_component` still scores it at 0.3, but nothing puts
    /// the code on a reason chain, so that arm is dead.
    ExtraFileProduced = 600,

    // Cache / shortcut.
    /// Selection lifted from meta.ini Tier-1 cache.
    FomodPlusCache = 700,
}

/// Wire name of a [`ReasonCode`], for example `"FORCED_REQUIRED"`. The same
/// text appears in the JSON document and in log lines.
///
/// The match is exhaustive and carries no wildcard arm, so there is no fallback
/// name. That is deliberate: a new variant without a name here fails the build
/// instead of reaching a consumer as an unnamed code.
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

/// Structured payload carried beside a plugin reason.
///
/// The constraint propagator emits [`ReasonDetail::UniqueFileEvidence`], the
/// target files a plugin uniquely produces.
/// [`InferenceDiagnosticsBuilder::absorb_solver`] emits
/// [`ReasonDetail::CspPhase`], the solver phase that fixed a selection.
/// `serialize_reason_detail` maps each variant to its schema-v2 shape.
///
/// A reason with no payload stores `None` rather than an empty variant, and
/// [`serialize_reason`] then omits the `detail` key entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasonDetail {
    /// Positive file evidence: the plugin uniquely produces at least one target
    /// file. `files` holds up to four examples, sorted byte-ascending so the
    /// document is deterministic; `count` is the full number of unique target
    /// hits and may exceed `files.len()`.
    UniqueFileEvidence {
        /// Up to four example destination paths, byte-ascending.
        files: Vec<String>,
        /// Total unique target hits, which may exceed `files.len()`.
        count: i32,
    },
    /// CSP-phase attribution: a solver phase selected the plugin. Emitted by
    /// [`InferenceDiagnosticsBuilder::absorb_solver`] only when all four
    /// conditions hold:
    ///
    /// 1. The group's `SolverResult::phase_per_group` entry is not empty.
    /// 2. `csp_phase_to_reason` recognises that entry. The recognised ids are
    ///    exactly `"csp.greedy"`, `"csp.local_search"`, `"csp.backtrack"`,
    ///    `"csp.repair"`, `"csp.focused"` and `"csp.fallback"`. Any other
    ///    non-empty id skips the whole group: no reason and no detail are
    ///    produced, although the group's `resolved_by` was already set to that
    ///    id.
    /// 3. The group is inside the `SolverResult::selections` grid.
    /// 4. The plugin index is below the plugin count the builder was sized to
    ///    from the installer.
    ///
    /// Within that, only a plugin whose `selections[step][group][plugin]` is
    /// `true` gets the reason. A consumer must therefore treat an absent
    /// `detail` as normal and not as a defect. The serialized keys sort to
    /// `nodes` then `phase`.
    CspPhase {
        /// `SolverResult::nodes_explored`, the run-level search-tree node
        /// total. The same value is stamped on every plugin of every group, so
        /// it is not a per-pick count.
        nodes: i32,
        /// Stable phase identifier, for example `"csp.greedy"` or
        /// `"csp.fallback"`.
        phase: String,
    },
}

/// Weight of the evidence axis in every composite score.
const WEIGHT_EVIDENCE: f64 = 0.40;
/// Weight of the propagation axis.
const WEIGHT_PROPAGATION: f64 = 0.30;
/// Weight of the repro axis.
const WEIGHT_REPRO: f64 = 0.20;
/// Weight of the ambiguity axis.
const WEIGHT_AMBIGUITY: f64 = 0.10;

/// Lowest composite that bands as `"high"`, inclusive.
const BAND_HIGH_THRESHOLD: f64 = 0.85;
/// Lowest composite that bands as `"medium"`, inclusive.
const BAND_MEDIUM_THRESHOLD: f64 = 0.50;

/// Run composite penalty per unexpected file produced.
const RUN_EXTRA_PENALTY_PER_FILE: f64 = 0.05;
/// Largest number of extra files the run penalty counts.
const RUN_EXTRA_PENALTY_CAP: i32 = 5;
/// Run composite penalty when the solver reached the global-fallback phase.
const RUN_FALLBACK_PENALTY: f64 = 0.10;

/// One justification attached to a plugin, group or step decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    /// Stable machine-readable reason code.
    pub code: ReasonCode,
    /// Human-readable explanation.
    pub message: String,
    /// Structured payload; `None` when the code speaks for itself, in which
    /// case [`serialize_reason`] omits the key.
    pub detail: Option<ReasonDetail>,
}

/// Per-axis confidence breakdown, each component in `[0.0, 1.0]` and defaulting
/// to 1.0.
///
/// The field docs below describe a plugin value. A group, step or run carries
/// the same four fields, but each one there is a weighted mean of the level
/// under it and not an independent measurement. The module doc holds the whole
/// formula, including the weights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfidenceComponents {
    /// Discrete score read off the plugin's reason chain by
    /// `evidence_component`: 1.0 when the chain holds a propagation-forcing
    /// code; otherwise the first matching reason in chain order wins, with
    /// `UniqueFileEvidence` giving 1.0, `NoUniqueEvidence` 0.5 and
    /// `ExtraFileProduced` 0.3; with no such reason, a selected plugin scores
    /// 0.5 and a deselected plugin 0.7.
    ///
    /// This is never a ratio, and the plugin's declared file count is never a
    /// numerator.
    pub evidence: f64,
    /// 1.0 when the plugin's reason chain holds a propagation-forcing code,
    /// 0.0 otherwise. `plugin_was_propagation_forced` defines the set:
    /// `ForcedRequired`, `ForcedNotUsable`, `ForcedSelectAll`,
    /// `ForcedAtLeastOne`, `ForcedExactlyOne`, `UniqueFileEvidence`,
    /// `NoFileEvidence`, `CardinalityForced` and `FomodPlusCache`.
    ///
    /// 0.0 means no such code is present, which covers both a plugin with no
    /// reasons at all and a plugin carrying only `CspPhase*` codes. It is not a
    /// positive statement that the CSP made the choice. A Tier-1 cache hit
    /// pushes `FomodPlusCache` onto every plugin, so this axis reads 1.0 for a
    /// whole cache-hit run even though the propagator never ran.
    pub propagation: f64,
    /// Run-level reproduction quality, not a group-local ratio. It is 1.0 for a
    /// deselected plugin and for a selected plugin in an exact-match run;
    /// otherwise `0.85 * repro_ratio`, where `repro_ratio` is computed once per
    /// run as `clamp01(1 - (missing + 0.5 * (size_mismatch + hash_mismatch)) /
    /// target_file_count)` and defaults to 1.0 when `target_file_count` is 0.
    ///
    /// A selected plugin in a non-exact run therefore never exceeds 0.85 on
    /// this axis, and scores exactly 0.85 when `target_file_count` is 0. Do not
    /// try to reconcile the value against per-group data: the counters behind
    /// it are run-level.
    pub repro: f64,
    /// Group ambiguity, looked up as
    /// `SolverResult::alternatives_per_group[step][group]`: 0 alternatives give
    /// 1.0, 1 gives 0.6, 2 gives 0.4, and 3 or more give 0.2. There is no
    /// division and no total-alternatives denominator, and every plugin of the
    /// group receives the same value. `solve_fomod_csp` fills
    /// `alternatives_per_group` with zeros and never computes a real count, so
    /// this axis reads 1.0 on every run today.
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

/// Composite confidence score with a derived band. Defaults to composite 1.0,
/// band `"high"`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfidenceScore {
    /// Weighted combination of `components`, computed by `composite_from` as
    /// `clamp01(0.40*evidence + 0.30*propagation + 0.20*repro +
    /// 0.10*ambiguity)`. Two exceptions.
    ///
    /// A group in which every plugin is propagation-forced gets all four
    /// components and this composite set to 1.0 directly, without the
    /// combination.
    ///
    /// At run level the combination runs first, then two penalties are
    /// subtracted before a final clamp: `RUN_EXTRA_PENALTY_PER_FILE` times
    /// `min(SolverResult::extra, RUN_EXTRA_PENALTY_CAP)`, so at most -0.25,
    /// plus `RUN_FALLBACK_PENALTY` when `phase_reached` is `"csp.fallback"`.
    /// A run composite is therefore not recoverable from the four published run
    /// components. The unit test `run_level_extra_and_fallback_penalties` pins
    /// both penalties.
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

/// Diagnostics for one plugin in one group.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginDiagnostics {
    /// The `SolverResult::selections` value for this slot, copied in by
    /// [`InferenceDiagnosticsBuilder::absorb_solver`].
    pub selected: bool,
    /// Confidence score for this plugin decision.
    pub confidence: ConfidenceScore,
    /// Reason chain in evaluation order.
    pub reasons: Vec<Reason>,
}

/// Diagnostics for one group in one step.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupDiagnostics {
    /// Confidence score aggregated over this group's plugins.
    pub confidence: ConfidenceScore,
    /// Identifier of the rule or CSP phase that resolved the group as a whole;
    /// empty when the group was not attributed. It carries no per-plugin
    /// information, which lives in `PluginDiagnostics::reasons`.
    ///
    /// The value space is:
    ///
    /// - `"propagation.select_all"`, `"propagation.unique_evidence"` or
    ///   `"propagation.cardinality"`, copied from `PropagationResult` by
    ///   [`InferenceDiagnosticsBuilder::absorb_propagation`].
    /// - A `"csp.*"` phase id, written by
    ///   [`InferenceDiagnosticsBuilder::absorb_solver`], but only if the field
    ///   is still empty.
    /// - `"cache.fomod_plus"`, written for every group by
    ///   [`InferenceDiagnosticsBuilder::set_cache_hit`].
    /// - Any string a caller passes to
    ///   [`InferenceDiagnosticsBuilder::set_group_resolved_by`].
    ///
    /// `absorb_solver` classifies the run-level group counters by string
    /// prefix: a `"propagation"` prefix or the exact string
    /// `"cache.fomod_plus"` counts as resolved by propagation, a `"csp."`
    /// prefix counts as resolved by the CSP, and anything else counts as
    /// neither. A new value must keep one of those prefixes or the run counters
    /// will silently miss it.
    pub resolved_by: String,
    /// Group-level reason chain.
    pub reasons: Vec<Reason>,
    /// Per-plugin diagnostics.
    pub plugins: Vec<PluginDiagnostics>,
}

/// Diagnostics for one installation step. `visible` defaults to `true`.
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

/// Pipeline timings in milliseconds.
///
/// These field names are not the wire keys. [`serialize_run_diagnostics`] drops
/// the `_ms` suffix and nests the four values under the wire key `timings_ms`,
/// so `list_ms` reaches a consumer as `timings_ms.list`.
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

/// Group-resolution counters, tallied once by
/// [`InferenceDiagnosticsBuilder::absorb_solver`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticGroupCounts {
    /// Total groups across all steps.
    pub total: i32,
    /// Groups resolved by propagation (or the Tier-1 cache).
    pub resolved_by_propagation: i32,
    /// Groups resolved by the CSP solver.
    pub resolved_by_csp: i32,
}

/// Cache-hit context.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticCacheInfo {
    /// True when inference short-circuited via the Tier-1 meta.ini cache.
    pub hit: bool,
    /// Cache origin (currently `"fomod-plus"` when set).
    pub source: String,
}

/// Run-level diagnostic summary.
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

/// Top-level diagnostics tree, shaped like the FOMOD installer hierarchy.
/// `schema_version` defaults to 2.
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

/// Band label for a composite score. Both thresholds are inclusive lower
/// bounds, which `band_for_thresholds` pins.
fn band_for(composite: f64) -> &'static str {
    if composite >= BAND_HIGH_THRESHOLD {
        "high"
    } else if composite >= BAND_MEDIUM_THRESHOLD {
        "medium"
    } else {
        "low"
    }
}

/// Clamp to `[0.0, 1.0]`. The bounds are finite constants, so this cannot
/// panic. A NaN would pass through, and the confidence formula never produces
/// one.
fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// Weighted mean, returning 1.0 when the weight is not positive. Every
/// aggregation level leans on that guard for an empty level.
fn weighted_mean(sum: f64, weight: f64) -> f64 {
    if weight <= 0.0 { 1.0 } else { sum / weight }
}

/// Combine the four components into a composite.
///
/// The order of the multiply-add expression is load-bearing: it fixes the
/// IEEE-754 result bit for bit, so all-ones yields `0.9999999999999999` rather
/// than `1.0`, and that is the number the emitted document carries.
/// Reassociating the sum changes the JSON. `composite_from_all_ones_bits` pins
/// it.
fn composite_from(c: &ConfidenceComponents) -> f64 {
    clamp01(
        WEIGHT_EVIDENCE * c.evidence
            + WEIGHT_PROPAGATION * c.propagation
            + WEIGHT_REPRO * c.repro
            + WEIGHT_AMBIGUITY * c.ambiguity,
    )
}

/// True when the chain holds any propagation-forcing code. The set is fixed and
/// includes `FomodPlusCache`, so a Tier-1 cache hit reads as propagation-forced
/// throughout the run.
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

/// Reason code for a CSP phase id. An unrecognised id maps to
/// `ImplicitDefault`, which `absorb_solver` reads as "record nothing for this
/// group".
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

/// Human message for a CSP phase id; empty for an unrecognised id.
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

/// Declared file count of one plugin. Any out-of-range index, negative
/// included, gives 0, which the aggregation weights then turn into 1.
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

/// Per-plugin evidence axis: a forced plugin scores 1.0; otherwise the first
/// evidence reason in chain order wins (`UniqueFileEvidence` 1.0,
/// `NoUniqueEvidence` 0.5, `ExtraFileProduced` 0.3); with no evidence reason, a
/// selected plugin scores 0.5 and a deselected one 0.7.
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

/// Per-plugin propagation axis.
fn propagation_component(forced: bool) -> f64 {
    if forced { 1.0 } else { 0.0 }
}

/// Per-group ambiguity axis: 0 or fewer alternatives give 1.0, 1 gives 0.6, 2
/// gives 0.4, and 3 or more give 0.2.
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

/// Accumulates per-decision reasons during inference, then computes the
/// confidence formula.
///
/// Construction sizes the nested `steps`/`groups`/`plugins` vectors to the
/// installer hierarchy so `add_plugin_reason` can index directly.
///
/// The call order is part of the contract, not a convention:
///
/// ```text
///   new(installer)              sizes steps/groups/plugins, sets run.groups.total
///         |
///         v
///   add_plugin_reason / add_group_reason / set_group_resolved_by
///   set_step_visibility / set_run_timings
///         |                     any order, repeatable
///         v
///   absorb_propagation(prop)    both must precede absorb_solver
///   set_cache_hit(source)
///         |
///         v
///   absorb_solver(result)       snapshot: run.groups.resolved_by_propagation
///         |                     and .resolved_by_csp are tallied here, once
///         v
///   set_target_file_count(n)    read only by finalize, so its position is free
///         |
///         v
///   finalize(result, _propagation, installer)
///         |                     sets the finalized flag and logs one line
///         v
///   diagnostics() -> &InferenceDiagnostics
/// ```
///
/// Two of those orderings are load-bearing, and violating either corrupts the
/// output silently:
///
/// 1. `absorb_solver` writes a group's `resolved_by` only while it is still
///    empty. `absorb_propagation` and `set_cache_hit` must therefore run first,
///    or the propagation or cache attribution is replaced by the CSP phase id.
/// 2. The run-level group counters are tallied once, at the end of
///    `absorb_solver`, by scanning every group's `resolved_by`. Anything that
///    changes a `resolved_by` after that point leaves the counters stale.
///
/// Reason chains append. `set_run_timings`, `set_target_file_count` and
/// `set_group_resolved_by` overwrite on every call, and `set_cache_hit` appends
/// one more `FomodPlusCache` reason each time. [`finalize`] must run last: it
/// latches a flag, and every setter after it is dropped in silence.
///
/// [`finalize`]: InferenceDiagnosticsBuilder::finalize
#[derive(Debug, Clone)]
pub struct InferenceDiagnosticsBuilder {
    diag: InferenceDiagnostics,
    finalized: bool,
    target_file_count: i32,
}

impl InferenceDiagnosticsBuilder {
    /// Size the hierarchy to `installer`: every step, group and plugin slot
    /// exists and holds a default [`PluginDiagnostics`]. `run.groups.total` is
    /// the group count across all steps.
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

    /// Append a reason to one plugin's chain. Any out-of-range index, negative
    /// included, is ignored; calls after [`finalize`](Self::finalize) are
    /// dropped.
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

    /// Append a reason to one group's chain, under the same index and
    /// post-finalize rules as [`add_plugin_reason`](Self::add_plugin_reason).
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

    /// Overwrite the rule or solver phase credited with resolving a group. See
    /// [`GroupDiagnostics::resolved_by`] for the value space and the prefix
    /// rule the run counters depend on.
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

    /// Set a step's `visible` flag and append one reason carrying `code` and a
    /// message picked from it. Index and post-finalize rules match
    /// [`add_plugin_reason`](Self::add_plugin_reason).
    ///
    /// The message table holds one defensive arm: `StepVisibilityForced` with
    /// `visible == false` gives "Visibility condition evaluated false". The
    /// only caller in the tree pairs `StepVisibilityForced` with
    /// `visible == true` and uses `StepNotVisible` for the false case, so that
    /// arm is never taken today.
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

    /// Overwrite the four pipeline timings, in milliseconds.
    pub fn set_run_timings(&mut self, list_ms: i64, scan_ms: i64, solve_ms: i64, total_ms: i64) {
        if self.finalized {
            return;
        }
        self.diag.run.timings.list_ms = list_ms;
        self.diag.run.timings.scan_ms = scan_ms;
        self.diag.run.timings.solve_ms = solve_ms;
        self.diag.run.timings.total_ms = total_ms;
    }

    /// Mark this run as a Tier-1 cache hit. Four effects, all unconditional:
    ///
    /// - `run.cache.hit` becomes `true` and `run.cache.source` becomes
    ///   `source`, written verbatim. The only value used in the tree is
    ///   `"fomod-plus"`.
    /// - `run.phase_reached` becomes `"tier1_cache"`.
    /// - Every group's `resolved_by` is overwritten with `"cache.fomod_plus"`,
    ///   including a group that already carries a propagation attribution.
    /// - Every plugin gains a `FomodPlusCache` reason with the message "Cached
    ///   selection from meta.ini".
    ///
    /// The reason append is not de-duplicated, so a second call leaves two
    /// identical reasons on every plugin. Like every setter, the call is a
    /// no-op once [`finalize`](Self::finalize) has run.
    ///
    /// The Tier-1 path in `fomod_inference_service` emits its own schema-v2
    /// document through `build_tier1_json` and does not call this method, which
    /// is here for callers that drive the builder end to end.
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

    /// Record the target tree's file count, the denominator of `repro_ratio`.
    /// Only [`finalize`](Self::finalize) reads it, so any position before that
    /// call works.
    pub fn set_target_file_count(&mut self, count: i32) {
        if self.finalized {
            return;
        }
        self.target_file_count = count;
    }

    /// Copy the propagation result into per-group `resolved_by` values and
    /// per-plugin reason chains, giving each code its human message.
    ///
    /// An empty `resolved_by` from the propagator leaves the current value
    /// alone, and an `ImplicitDefault` plugin code records no reason at all.
    /// Every nested loop runs to the smaller of the two dimensions, so a
    /// propagation result shaped differently from the installer truncates
    /// rather than panicking.
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

    /// Copy the solver result in: the run repro counters, a per-group phase
    /// reason on each selected plugin, the per-plugin `selected` flags, and the
    /// run-level group counters.
    ///
    /// A group's `resolved_by` is written only while it is still empty, so
    /// [`absorb_propagation`](Self::absorb_propagation) and
    /// [`set_cache_hit`](Self::set_cache_hit) must run before this call or
    /// their attribution is replaced by the CSP phase id. The group counters
    /// are tallied at the end of this call and never recomputed, so any later
    /// change to a `resolved_by` leaves them stale. The type doc holds the full
    /// call-order contract.
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

        // Copy selection state into PluginDiagnostics.selected.
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

    /// Compute the confidence components and bands, apply the run-level
    /// penalties, and backfill `reproduced`. Must run last: it latches the
    /// `finalized` flag, so every later setter is dropped in silence and a
    /// second call returns at once.
    ///
    /// The module doc holds the whole formula in one block. The parts that are
    /// easiest to get wrong:
    ///
    /// - `repro_ratio` is computed once per run. It is 1.0 when
    ///   `result.exact_match` holds or `target_file_count` is 0, and otherwise
    ///   `clamp01(1 - (missing + 0.5 * (size_mismatch + hash_mismatch)) /
    ///   target_file_count)`. A size or hash mismatch counts as half a miss,
    ///   because the file exists at the right destination and only its content
    ///   or size differs.
    /// - A plugin's `repro` is 1.0 when the plugin is deselected or the run
    ///   matched exactly, and otherwise `0.85 * repro_ratio`. The 0.85 is a
    ///   deliberate ceiling: a selected plugin in a non-exact run never scores
    ///   above 0.85 on that axis, even when nothing is missing.
    /// - The aggregation weights are `plugin_file_count + 1` for plugin into
    ///   group, `plugins.len() + 1` for group into step, and `groups.len() + 1`
    ///   for step into run. The `+ 1` stops a zero-file plugin, an empty group
    ///   or an empty step from carrying zero weight.
    /// - A group whose plugins are all propagation-forced skips the mean: all
    ///   four components and the group composite become exactly 1.0.
    /// - The run composite subtracts `RUN_EXTRA_PENALTY_PER_FILE` per extra
    ///   file, capped at `RUN_EXTRA_PENALTY_CAP` files, and
    ///   `RUN_FALLBACK_PENALTY` when `phase_reached` is `"csp.fallback"`, then
    ///   clamps again.
    ///
    /// `_propagation` is never read. Everything the confidence math needs was
    /// already folded in by [`absorb_propagation`](Self::absorb_propagation),
    /// so passing `PropagationResult::default()` changes nothing and the
    /// in-crate tests do exactly that. See `PARITY-NOTES.md`.
    ///
    /// Side effect: on the call that actually finalizes, this writes exactly
    /// one line through the global `Logger`, after setting the flag:
    ///
    /// ```text
    /// [infer] Diagnostics: confidence=<composite, 2 decimals> (<band>),
    ///   phase=<phase_reached>, repro=miss:<missing>/extra:<extra>/
    ///   sm:<size_mismatch>/hm:<hash_mismatch>
    /// ```
    ///
    /// The real line is not wrapped, and `phase` reads "n/a" when
    /// `phase_reached` is empty. There is no quiet mode: a host suppresses or
    /// redirects the line by registering a log callback through
    /// `setLogCallback`.
    pub fn finalize(
        &mut self,
        result: &SolverResult,
        _propagation: &PropagationResult,
        installer: &FomodInstaller,
    ) {
        if self.finalized {
            return;
        }

        // Fraction of the target tree the chosen selections reproduce, computed
        // once for the whole run. The 0.5 gives a size or hash mismatch half the
        // weight of a miss: the file exists at the right destination and only
        // its content or size is wrong.
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
                    // The 0.85 is a deliberate ceiling, not a scale factor: a
                    // selected plugin in a non-exact run never scores above
                    // 0.85 on the repro axis, even when nothing is missing.
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

        // Backfill `reproduced`. The zero test is defensive: no path writes
        // `run.repro.reproduced` before this point and `finalize` cannot run
        // twice, so the condition is always true today. The two branches
        // produce very different numbers. With a target count, `reproduced` is
        // the target size minus the misses and the mismatches. Without one it
        // is a proxy: the declared file count of the selected plugins, minus
        // the misses. Both clamp at 0.
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

        // One-line run summary. This is a log line and not part of the emitted
        // JSON, so its rounding is not a wire contract.
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

    /// Borrow the accumulated diagnostics.
    pub fn diagnostics(&self) -> &InferenceDiagnostics {
        &self.diag
    }
}

// ---------------------------------------------------------------------------
// Serialization to the schema-v2 JSON model. Insertion order does not decide
// the output: `Value::dump` emits object keys sorted, so the orders named below
// are what a consumer reads.
// ---------------------------------------------------------------------------

/// Serialize a [`ReasonDetail`] to its schema-v2 object. Keys emit as `count`
/// then `files` for unique-file evidence, `nodes` then `phase` for a CSP phase.
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

/// Serialize one [`ConfidenceScore`]. Keys emit as `band`, `components`,
/// `composite`, and the components object as `ambiguity`, `evidence`,
/// `propagation`, `repro`.
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

/// Serialize one [`Reason`]. `code` and `message` are always present; `detail`
/// appears only when the reason carries one. Keys emit as `code`, `detail`,
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

/// Serialize the [`RunDiagnostics`] summary. This is the only serializer here
/// that renames fields on the way to the wire.
///
/// ```text
/// {                                  keys emit in sorted order on dump
///   "cache":          { "hit": bool, "source": str }
///   "confidence":     serialize_confidence(run.confidence)
///   "exact_match":    bool
///   "groups":         { "resolved_by_csp", "resolved_by_propagation", "total" }
///   "nodes_explored": int
///   "phase_reached":  str
///   "repro":          { "extra", "hash_mismatch", "missing", "reproduced",
///                       "size_mismatch" }
///   "timings_ms":     { "list"  <- list_ms,   "scan"  <- scan_ms,
///                       "solve" <- solve_ms,  "total" <- total_ms }
/// }
/// ```
///
/// Only the timings are renamed: the [`DiagnosticTimings`] fields lose their
/// `_ms` suffix and move under the `timings_ms` object, so a consumer looking
/// for `list_ms` will not find it.
///
/// Every confidence number is a `Value::Double` and every counter a
/// `Value::Int`. The split decides the emitted text, because the same zero
/// dumps as `0.0` on one side and `0` on the other.
///
/// [`crate::fomod_inference_atoms::assemble_json`] and the Tier-1 emitter in
/// `fomod_inference_service` nest the whole object under the top-level key
/// `diagnostics`.
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

    // --- integer values are wire format and must not drift -----------------

    #[test]
    fn reason_code_int_values_match_cpp() {
        // Renumbering any of these silently remaps every stored diagnostic.
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

    // --- string names are wire format too ----------------------------------

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
                // count is the full number of hits, independent of files.len().
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
        // The test name matches no fixture in the tree; this pins arithmetic
        // only. Two equal-weight plugins in one SelectExactlyOne group, neither
        // propagation-forced: the selected plugin scores evidence 0.5 and the
        // deselected one 0.7 (the fallthrough branch of `evidence_component`),
        // the run matched exactly so repro is 1.0, and zero alternatives make
        // ambiguity 1.0. The group's file-count-weighted evidence mean is 0.6
        // and its composite is 0.40*0.6 + 0.30*0.0 + 0.20*1.0 + 0.10*1.0 = 0.54,
        // band "medium" (>= BAND_MEDIUM_THRESHOLD, < BAND_HIGH_THRESHOLD).
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
