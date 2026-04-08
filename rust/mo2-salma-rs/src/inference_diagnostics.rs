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
/// The constraint propagator emits exactly one variant,
/// [`ReasonDetail::UniqueFileEvidence`] (the target files a plugin uniquely
/// produces). Tasks 8-10 add further variants as the CSP solver and diagnostics
/// assembler grow; Task 10 maps each variant to its schema-v2 JSON shape. A
/// missing detail is represented as `None` at the storage site rather than an
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
        }
    }
}
