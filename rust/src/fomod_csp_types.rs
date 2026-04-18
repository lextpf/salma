//! CSP solver datatypes - Rust port of `src/FomodCSPTypes.hpp`.
//!
//! Task 6 ported the two types the forward simulator and its scoring oracle
//! need:
//!
//! - [`ReproMetrics`] - reproduction quality counters (mirror of
//!   `mo2core::ReproMetrics` in `src/FomodCSPTypes.hpp`).
//! - [`InferenceOverrides`] - tri-state external-dependency overrides. Its C++
//!   home is `src/FomodCSPSolver.hpp` (NOT `FomodCSPTypes.hpp`); it is placed
//!   here so the simulator can consume it without pulling in the full solver
//!   header.
//!
//! Task 8 EXTENDS this module with the rest of `FomodCSPTypes.hpp` plus the two
//! solver types the datatype set needs ([`SolverResult`] from
//! `src/FomodCSPSolver.hpp` and [`SolverConfig`]/[`CONFIG`] from
//! `src/FomodCSPSolverInternal.hpp`):
//!
//! - [`GroupRef`], [`GroupOption`], [`OptionProfile`], [`CachedOptions`]
//! - [`Precompute`] - the central read-only solver data structure. It BORROWS
//!   its seven inputs as `&'a`/`Option<&'a>` references (mirroring the C++
//!   non-owning `const*` fields) and owns the derived reverse indices; see
//!   `rust/PARITY-NOTES.md` "Task 8" for the borrow-shape rationale.
//! - [`OptionCacheKey`]/[`MemoKey`] - map keys. Their C++ std::hash functors
//!   are NOT observable (find/emplace only), so `#[derive(Hash)]` is used; only
//!   the `PartialEq`/`Eq` field comparison is load-bearing.
//! - [`SolverState`] (with [`SolverSearchState`], [`SolverBestResult`],
//!   [`SolverProgress`]), [`SolverStats`], [`FlagDelta`], [`SearchPlan`],
//!   [`SolverConfig`] - the solver-phase state carried through Task 9. Several
//!   are defined-but-not-yet-consumed here; Task 9 uses them.
//! - The SelectAny caps ([`SELECT_ANY_CAP_NARROW`], [`SELECT_ANY_CAP_MEDIUM`],
//!   [`SELECT_ANY_CAP_FULL`]).

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::fomod_atom::{AtomIndex, ExpandedAtoms, TargetTree};
use crate::fomod_dependency_evaluator::ExternalConditionOverride;
use crate::fomod_ir::FomodInstaller;
use crate::fomod_propagator::PropagationResult;

/// Reproduction quality metrics comparing a simulated install to the target
/// file tree. Mirror of `mo2core::ReproMetrics` in `src/FomodCSPTypes.hpp`.
///
/// Counts are computed per destination file. Lower counts in every error
/// category means a better reproduction. Counters are `i32` to match the C++
/// `int` fields; all default to 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReproMetrics {
    /// Target files not produced by the simulation.
    pub missing: i32,
    /// Simulated files absent from the target.
    pub extra: i32,
    /// Files present in both but with differing (nonzero) sizes.
    pub size_mismatch: i32,
    /// Files matching in size but differing in (nonzero) content hash.
    pub hash_mismatch: i32,
    /// Files successfully reproduced (size and hash match, or fall through).
    pub reproduced: i32,
}

impl ReproMetrics {
    /// True when every error counter is zero (missing, extra, size_mismatch,
    /// hash_mismatch). Mirror of C++ `exact()`: `reproduced` is intentionally
    /// ignored, so an all-zero tree (nothing reproduced, nothing wrong) is
    /// still "exact".
    pub fn exact(&self) -> bool {
        self.missing == 0 && self.extra == 0 && self.size_mismatch == 0 && self.hash_mismatch == 0
    }

    /// Strict-weak (lexicographic) ordering, mirror of C++ `better_than`:
    /// `missing` asc, then `extra` asc, then `size_mismatch` asc, then
    /// `hash_mismatch` asc, then `reproduced` DESC (more reproduced is
    /// better). When all five compare equal, returns `false` (not strictly
    /// better than an equal metric).
    pub fn better_than(&self, rhs: &ReproMetrics) -> bool {
        if self.missing != rhs.missing {
            return self.missing < rhs.missing;
        }
        if self.extra != rhs.extra {
            return self.extra < rhs.extra;
        }
        if self.size_mismatch != rhs.size_mismatch {
            return self.size_mismatch < rhs.size_mismatch;
        }
        if self.hash_mismatch != rhs.hash_mismatch {
            return self.hash_mismatch < rhs.hash_mismatch;
        }
        if self.reproduced != rhs.reproduced {
            return self.reproduced > rhs.reproduced;
        }
        false
    }
}

/// Tri-state overrides for external dependencies the solver cannot evaluate.
/// Mirror of `mo2core::InferenceOverrides` (declared in
/// `src/FomodCSPSolver.hpp`).
///
/// Some FOMOD conditions depend on external state (game version, other
/// installed mods) unavailable during inference. These overrides let the
/// caller force individual conditional patterns or step-visibility conditions
/// to [`ExternalConditionOverride::ForceTrue`],
/// [`ExternalConditionOverride::ForceFalse`], or leave them
/// [`ExternalConditionOverride::Unknown`] so the solver can prune or explore
/// branches accordingly. Both vectors may be shorter than the installer's
/// counts; missing entries fall back to normal evaluation (see the forward
/// simulator).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InferenceOverrides {
    /// Per conditional pattern: tri-state external dependency override.
    /// Indexed by the pattern index in `FomodInstaller::conditional_patterns`.
    pub conditional_active: Vec<ExternalConditionOverride>,
    /// Per step visibility condition: tri-state external dependency override.
    /// Indexed by the step index in `FomodInstaller::steps`.
    pub step_visible: Vec<ExternalConditionOverride>,
}

// ---------------------------------------------------------------------------
// SelectAny option caps (mirror of `csp_detail::kSelectAnyCap*` in
// `src/FomodCSPTypes.hpp`, re-exported into `mo2core`). Lower caps speed up
// early solver phases; later phases widen or remove the cap.
// ---------------------------------------------------------------------------

/// Tight cap for initial greedy/local search. Mirror of
/// `kSelectAnyCapNarrow` (64).
pub const SELECT_ANY_CAP_NARROW: i32 = 64;
/// Medium cap for component and repair phases. Mirror of
/// `kSelectAnyCapMedium` (256).
pub const SELECT_ANY_CAP_MEDIUM: i32 = 256;
/// No cap (0 = enumerate all valid combinations). Mirror of
/// `kSelectAnyCapFull` (0).
pub const SELECT_ANY_CAP_FULL: i32 = 0;

/// Reference to a plugin group within the FOMOD installer, with flat plugin
/// offsets. Mirror of `mo2core::GroupRef`.
///
/// Groups are addressed by their `(step, group)` index pair. `flat_start` gives
/// the offset into the global flat plugin array so per-plugin precomputed data
/// (evidence, unique support, ...) can be looked up without re-walking the step
/// hierarchy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupRef {
    /// Index of the containing step in `FomodInstaller::steps`.
    pub step_idx: i32,
    /// Index of the group within the step.
    pub group_idx: i32,
    /// Offset of the first plugin in the global flat plugin array.
    pub flat_start: i32,
    /// Number of plugins in this group.
    pub plugin_count: i32,
}

/// Boolean mask of selected plugins within a single group. Mirror of
/// `mo2core::GroupOption` (`std::vector<bool>`).
///
/// Indexed by the group's local plugin position (not the global flat index).
/// `option[i] == true` means the i-th plugin of the group is selected for this
/// candidate option.
pub type GroupOption = Vec<bool>;

/// Output of the CSP solver: plugin selections and match-quality metrics.
/// Mirror of `mo2core::SolverResult` in `src/FomodCSPSolver.hpp`.
///
/// `selections` is a 3-D boolean grid indexed `[step][group][plugin]`. The
/// diagnostic fields (`phase_per_group`, `alternatives_per_group`,
/// `phase_reached`) are populated by the solver phases (Task 9); empty signals
/// that no diagnostic state was recorded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverResult {
    /// `[step][group][plugin]` selection grid.
    pub selections: Vec<Vec<Vec<bool>>>,
    /// Flags set by the chosen plugins.
    pub inferred_flags: HashMap<String, String>,
    /// True iff the install reproduces the target tree exactly.
    pub exact_match: bool,
    /// CSP search-tree node count (diagnostics).
    pub nodes_explored: i32,
    /// Target dests not produced by selections.
    pub missing: i32,
    /// Selections that produce dests not in target.
    pub extra: i32,
    /// Dests produced with wrong uncompressed size.
    pub size_mismatch: i32,
    /// Dests produced with wrong content hash.
    pub hash_mismatch: i32,
    /// Per-group identifier for the CSP phase that fixed the group's selection.
    /// Indexed `[step][group]`.
    pub phase_per_group: Vec<Vec<String>>,
    /// Per-group count of options whose evidence score was within the
    /// ambiguity window of the chosen option's score. Indexed `[step][group]`.
    pub alternatives_per_group: Vec<Vec<i32>>,
    /// Stable identifier of the highest CSP phase whose work contributed to the
    /// final result. Empty when no diagnostic state was recorded.
    pub phase_reached: String,
}

/// Mutable search state carried through backtracking and local search. Mirror
/// of `mo2core::SolverSearchState`. Consumed by the Task 9 phases.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverSearchState {
    /// Current per-step/group/plugin selections.
    pub selections: Vec<Vec<Vec<bool>>>,
    /// Current FOMOD condition-flag values.
    pub flags: HashMap<String, String>,
    /// Total search nodes visited across all passes.
    pub nodes_explored: i32,
    /// True when an exact (zero-error) reproduction has been found.
    pub found_exact: bool,
}

/// Tracks the best solution found so far during the solve. Mirror of
/// `mo2core::SolverBestResult`. Consumed by the Task 9 phases.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverBestResult {
    /// The best selection set and its error counts.
    pub best: SolverResult,
    /// Metrics corresponding to `best`.
    pub best_metrics: ReproMetrics,
    /// False until the first candidate is recorded.
    pub has_best: bool,
}

/// Progress tracking and deadline enforcement for the solver. Mirror of
/// `mo2core::SolverProgress`. Consumed by the Task 9 phases.
///
/// The C++ time points are `std::chrono::steady_clock::time_point`; the port
/// uses [`std::time::Instant`]. The C++ default-inits `deadline` to the clock
/// epoch as a placeholder overwritten before use; the port models the
/// not-yet-set state as `None` instead. `PROGRESS_NODE_INTERVAL` /
/// `PROGRESS_TIME_INTERVAL_MS` are associated constants matching the C++
/// `static constexpr` members.
#[derive(Debug, Clone)]
pub struct SolverProgress {
    /// Node count at the last progress report.
    pub last_progress_nodes: i32,
    /// Wall-clock time of the last progress report.
    pub last_progress_time: Instant,
    /// Estimated total nodes for the current pass (0 = unknown).
    pub estimated_total: i64,
    /// Wall-clock time at the start of the current pass.
    pub pass_start_time: Instant,
    /// Node count at the start of the current pass.
    pub pass_start_nodes: i64,
    /// Hard wall-clock deadline for the entire solve (`None` until set).
    pub deadline: Option<Instant>,
    /// Set to true once `deadline` is passed.
    pub deadline_exceeded: bool,
}

impl SolverProgress {
    /// Minimum nodes between progress log lines.
    pub const PROGRESS_NODE_INTERVAL: i32 = 1_000;
    /// Minimum milliseconds between progress log lines.
    pub const PROGRESS_TIME_INTERVAL_MS: i32 = 1_000;
}

impl Default for SolverProgress {
    fn default() -> Self {
        let now = Instant::now();
        SolverProgress {
            last_progress_nodes: 0,
            last_progress_time: now,
            estimated_total: 0,
            pass_start_time: now,
            pass_start_nodes: 0,
            deadline: None,
            deadline_exceeded: false,
        }
    }
}

/// Composite solver state bundling search state, best result, and progress.
/// Mirror of `mo2core::SolverState`. Consumed by the Task 9 phases.
#[derive(Debug, Clone, Default)]
pub struct SolverState {
    /// Current mutable search state.
    pub search: SolverSearchState,
    /// Best solution found so far.
    pub best: SolverBestResult,
    /// Progress tracking and deadline.
    pub progress: SolverProgress,
}

/// Diagnostic statistics for option pruning and search behavior. Mirror of
/// `mo2core::SolverStats`.
///
/// The option-generation counters (`dropped_extra_only_options`,
/// `collapsed_equivalent_options`, `capped_select_any_options`) and
/// `logged_group_options` are populated by Task 8's option enumeration; the
/// search-tree pruning counters are populated by Task 9. `forced_unique_options`
/// is DEAD (never incremented anywhere in the C++); it is kept for field parity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverStats {
    /// Options dropped because they only produce extra files.
    pub dropped_extra_only_options: i32,
    /// Options collapsed due to identical destination sets.
    pub collapsed_equivalent_options: i32,
    /// Groups forced to a single option (unique evidence). DEAD counter: the
    /// C++ declares it but never increments it. Kept for field parity.
    pub forced_unique_options: i32,
    /// Options trimmed by the SelectAny cap.
    pub capped_select_any_options: i32,
    /// Subtrees pruned because all options are extra-only.
    pub pruned_extra_only: i32,
    /// Subtrees pruned by lower-bound comparison against best.
    pub pruned_lower_bound: i32,
    /// Subtrees pruned by memoization hit.
    pub pruned_memo: i32,
    /// Groups skipped because their step is not visible.
    pub skipped_invisible: i32,
    /// Subtrees abandoned after exceeding the per-pass node limit.
    pub pruned_node_limit: i32,
    /// Branches abandoned because the backtrack stack exceeded the max depth.
    pub max_depth_aborts: i32,
    /// Tracks which groups have had their options logged (avoids duplicate log
    /// output). Sized to `pre.groups.len()` by the solver entry point.
    pub logged_group_options: Vec<bool>,
}

/// Precomputed profile of a single group selection option. Mirror of
/// `mo2core::OptionProfile`.
///
/// Captures the effect of selecting a particular plugin combination: which
/// destinations it produces, how much evidence supports it, and which condition
/// flags it writes. Used for pruning and ordering during option enumeration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OptionProfile {
    /// Boolean mask of selected plugins in this group.
    pub option: GroupOption,
    /// Sum of per-plugin evidence scores for selected plugins.
    pub evidence_score: i32,
    /// Count of target destinations uniquely supplied by this option.
    pub unique_support: i32,
    /// Destinations produced that exist in the target.
    pub useful_dests: i32,
    /// Destinations produced that are not in the target.
    pub extra_dests: i32,
    /// True if this option sets a flag required by a step/condition.
    pub sets_needed_flag: bool,
    /// Destination paths produced.
    pub produced: HashSet<String>,
    /// Atom keys produced (`"dest|source"` identifiers).
    pub produced_atoms: HashSet<String>,
    /// Condition flags set by the selected plugins.
    pub flags_written: HashMap<String, String>,
}

/// Cached group options and their precomputed profiles. Mirror of
/// `mo2core::CachedOptions`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CachedOptions {
    /// Valid selection combinations for this group.
    pub options: Vec<GroupOption>,
    /// Corresponding profile for each option.
    pub profiles: Vec<OptionProfile>,
}

/// Central precomputed data structure for the CSP solver. Mirror of
/// `mo2core::Precompute`.
///
/// Built once before the solve begins (see
/// [`crate::fomod_csp_precompute::build_precompute`]) and shared read-only by
/// all solver phases. The seven input fields are non-owning references (the C++
/// holds raw `const*`); the borrow lifetime `'a` ties the `Precompute` to its
/// inputs. `overrides`/`propagation` are `Option<&_>` because the C++ pointers
/// are nullable.
#[derive(Debug, PartialEq)]
pub struct Precompute<'a> {
    /// The FOMOD installer definition.
    pub installer: &'a FomodInstaller,
    /// Expanded file-install atoms.
    pub atoms: &'a ExpandedAtoms,
    /// Reverse index mapping destination paths to atoms.
    pub atom_index: &'a AtomIndex,
    /// Target file tree (dest -> size/hash).
    pub target: &'a TargetTree,
    /// Destination paths excluded from scoring.
    pub excluded: &'a HashSet<String>,
    /// Optional external condition overrides.
    pub overrides: Option<&'a InferenceOverrides>,
    /// Optional constraint propagation result.
    pub propagation: Option<&'a PropagationResult>,

    /// All groups across all steps, in the caller-provided order.
    pub groups: Vec<GroupRef>,
    /// Per-flat-plugin evidence score (higher = more likely needed).
    pub evidence: Vec<i32>,

    /// Maps flat plugin index to its group index (-1 if none).
    pub plugin_to_group: Vec<i32>,
    /// Count of target dests uniquely supplied by this plugin.
    pub plugin_unique_support: Vec<i32>,

    /// All flags read by step-visibility or plugin-type conditions.
    pub needed_flags: HashSet<String>,
    /// Per-group: flags written by plugins.
    pub group_sets_flags: Vec<HashSet<String>>,
    /// Per-group: flags read by conditions.
    pub group_reads_flags: Vec<HashSet<String>>,
    /// Per-group: sorted flag keys for cache hashing (byte-ascending).
    pub group_cache_flags: Vec<Vec<String>>,
    /// Maps flag name to groups that can set it (sorted-unique).
    pub flag_to_setter_groups: HashMap<String, Vec<i32>>,
    /// Sorted union of needed and written flags, for memoization signatures.
    pub memo_flags: Vec<String>,
    /// Per-group: destination paths any plugin in the group can produce.
    pub group_dests: Vec<HashSet<String>>,

    /// Groups that can produce each destination (sorted-unique).
    pub dest_to_groups: HashMap<String, Vec<i32>>,
    /// Flat plugin indices that can produce each destination (sorted-unique).
    pub dest_to_plugins: HashMap<String, Vec<i32>>,
    /// Groups that can produce a size-matching file for the destination
    /// (sorted-unique).
    pub dest_to_size_match_groups: HashMap<String, Vec<i32>>,
    /// Groups that can produce a hash-matching file for the destination
    /// (sorted-unique).
    pub dest_to_hash_capable_groups: HashMap<String, Vec<i32>>,

    /// Destinations whose production depends on flags or conditions.
    pub conditional_dests: HashSet<String>,
    /// Flat plugin indices that appear in multiple destination-conflict sets
    /// (sorted-unique).
    pub contested_plugins: Vec<i32>,
    /// Independent group components (groups sharing no destinations or flags),
    /// for parallel backtracking. Ordered size-descending, min-member-ascending.
    pub components: Vec<Vec<i32>>,
}

/// Cache key for option computations, identifying a group under a specific flag
/// state. Mirror of `mo2core::OptionCacheKey`.
///
/// The C++ `OptionCacheKeyHash` functor is NOT observable (the cache is only
/// used via find/emplace, never iterated to output), so `#[derive(Hash)]` here
/// need not reproduce the golden-ratio combine; only `PartialEq`/`Eq` on
/// `(group_idx, flags_sig, select_any_cap, exact_mode)` is load-bearing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OptionCacheKey {
    /// Index into `Precompute::groups`.
    pub group_idx: i32,
    /// Hash of the subset of flag values this group's conditions read; built via
    /// `hash_flag_subset(flags, group_cache_flags[gidx])`.
    pub flags_sig: u64,
    /// Maximum number of SelectAny options to enumerate.
    pub select_any_cap: i32,
    /// When true, uncapped enumeration is used for this group.
    pub exact_mode: bool,
}

/// Memoization key for subtree pruning during backtracking. Mirror of
/// `mo2core::MemoKey`. Consumed by the Task 9 phases.
///
/// As with [`OptionCacheKey`], the C++ hash functor is not observable, so
/// `#[derive(Hash)]` is used; only the `(next_idx, flag_state_sig,
/// contested_sig)` field equality matters. `flag_state_sig`/`contested_sig` are
/// produced by Task 9.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MemoKey {
    /// Next group index in the search order.
    pub next_idx: i32,
    /// Hash of current condition-flag values.
    pub flag_state_sig: u64,
    /// Hash of current selections for contested plugins.
    pub contested_sig: u64,
}

/// Undo record for a single flag change during backtracking. Mirror of
/// `mo2core::FlagDelta`. Consumed by the Task 9 phases.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagDelta {
    /// Flag that was modified.
    pub name: String,
    /// True if the flag existed before the change.
    pub had_value: bool,
    /// Previous value (meaningful only when `had_value` is true).
    pub old_value: String,
}

/// Configuration for a single backtracking pass over a set of groups. Mirror of
/// `mo2core::SearchPlan`. Consumed by the Task 9 phases.
#[derive(Debug, Clone, Default)]
pub struct SearchPlan {
    /// Group indices in the order they will be explored.
    pub order: Vec<i32>,
    /// Inverse map: `order_pos[group_idx]` = position in `order` (-1 if absent).
    pub order_pos: Vec<i32>,
    /// Maximum nodes to explore in this pass (0 = unlimited).
    pub node_limit: i32,
    /// Subtree memoization table.
    pub memo: HashMap<MemoKey, ReproMetrics>,
    /// When true, flags are updated incrementally rather than rebuilt.
    pub incremental_flags: bool,
}

/// Tuning constants controlling time budgets, search-space caps, and node
/// limits for each phase of the FOMOD CSP solver. Mirror of
/// `mo2core::SolverConfig` in `src/FomodCSPSolverInternal.hpp`. Consumed by the
/// Task 9 phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolverConfig {
    /// Maximum wall-clock time for the entire solve (seconds).
    pub time_limit_seconds: i32,
    /// Maximum number of state checkpoints kept during backtracking.
    pub max_checkpoints: usize,
    /// Search space estimate cap for the greedy phase (combinations).
    pub greedy_space_cap: u64,
    /// Search space estimate cap per component in Phase 2 (combinations).
    pub component_space_cap: u64,
    /// Maximum nodes explored per component backtrack in Phase 2.
    pub component_node_limit: i32,
    /// Maximum nodes explored during residual repair in Phase 3.
    pub residual_node_limit: i32,
    /// Search space estimate cap for mismatch-focused search in Phase 4.
    pub focused_space_cap: u64,
    /// Maximum nodes explored during focused backtrack in Phase 4.
    pub focused_node_limit: i32,
    /// Search space estimate cap for exact-match focused fallback in Phase 4.
    pub exact_focused_space_cap: u64,
    /// Maximum nodes explored during exact-match focused backtrack.
    pub exact_focused_node_limit: i32,
    /// Search space estimate cap for global fallback passes in Phase 5.
    pub global_space_cap: u64,
    /// Maximum nodes explored per global backtrack pass in Phase 5.
    pub global_node_limit: i32,
    /// Node limit for full-width passes when the current best is perfect.
    pub full_pass_default_limit: i32,
    /// Node limit for full-width passes when mismatches remain.
    pub full_pass_imperfect_limit: i32,
}

impl SolverConfig {
    /// The default solver configuration, matching the C++ in-struct
    /// initializers. Provided as an associated `const` so [`CONFIG`] can be a
    /// `const` (C++ `extern const SolverConfig kConfig`).
    pub const DEFAULT: SolverConfig = SolverConfig {
        time_limit_seconds: 600,
        max_checkpoints: 4096,
        greedy_space_cap: 1_000_000_000,
        component_space_cap: 10_000_000,
        component_node_limit: 2_000_000,
        residual_node_limit: 3_000_000,
        focused_space_cap: 10_000_000,
        focused_node_limit: 6_000_000,
        exact_focused_space_cap: 12_000_000,
        exact_focused_node_limit: 8_000_000,
        global_space_cap: 10_000_000,
        global_node_limit: 10_000_000,
        full_pass_default_limit: 2_000_000,
        full_pass_imperfect_limit: 6_000_000,
    };
}

impl Default for SolverConfig {
    fn default() -> Self {
        SolverConfig::DEFAULT
    }
}

/// Global default solver configuration instance. Mirror of `mo2core::kConfig`.
pub const CONFIG: SolverConfig = SolverConfig::DEFAULT;

#[cfg(test)]
mod tests {
    use super::*;

    // --- defaults (must match the C++ in-struct initializers) ---

    #[test]
    fn repro_metrics_defaults_are_zero() {
        let m = ReproMetrics::default();
        assert_eq!(m.missing, 0);
        assert_eq!(m.extra, 0);
        assert_eq!(m.size_mismatch, 0);
        assert_eq!(m.hash_mismatch, 0);
        assert_eq!(m.reproduced, 0);
    }

    #[test]
    fn inference_overrides_default_is_empty() {
        let o = InferenceOverrides::default();
        assert!(o.conditional_active.is_empty());
        assert!(o.step_visible.is_empty());
    }

    // --- exact() ignores reproduced ---

    #[test]
    fn exact_true_when_all_errors_zero_regardless_of_reproduced() {
        assert!(ReproMetrics::default().exact());
        // reproduced > 0 with zero errors is still exact.
        assert!(
            ReproMetrics {
                reproduced: 999,
                ..Default::default()
            }
            .exact()
        );
        // zero reproduced and zero errors is also exact (empty vs empty).
        assert!(
            ReproMetrics {
                reproduced: 0,
                ..Default::default()
            }
            .exact()
        );
    }

    #[test]
    fn exact_false_when_any_error_nonzero() {
        for m in [
            ReproMetrics {
                missing: 1,
                ..Default::default()
            },
            ReproMetrics {
                extra: 1,
                ..Default::default()
            },
            ReproMetrics {
                size_mismatch: 1,
                ..Default::default()
            },
            ReproMetrics {
                hash_mismatch: 1,
                ..Default::default()
            },
        ] {
            assert!(!m.exact(), "{m:?} must not be exact");
        }
    }

    // --- better_than lexicographic table ---

    fn m(missing: i32, extra: i32, size: i32, hash: i32, repro: i32) -> ReproMetrics {
        ReproMetrics {
            missing,
            extra,
            size_mismatch: size,
            hash_mismatch: hash,
            reproduced: repro,
        }
    }

    #[test]
    fn better_than_orders_each_counter_in_priority() {
        // (a, b, a.better_than(b)) covering every tiebreak level.
        let cases: &[(ReproMetrics, ReproMetrics, bool)] = &[
            // missing dominates: fewer missing wins even with everything else worse.
            (m(1, 9, 9, 9, 0), m(2, 0, 0, 0, 999), true),
            (m(2, 0, 0, 0, 999), m(1, 9, 9, 9, 0), false),
            // extra breaks a missing tie.
            (m(3, 1, 9, 9, 0), m(3, 2, 0, 0, 999), true),
            (m(3, 2, 0, 0, 0), m(3, 1, 9, 9, 999), false),
            // size_mismatch breaks a missing+extra tie.
            (m(3, 3, 1, 9, 0), m(3, 3, 2, 0, 999), true),
            (m(3, 3, 2, 0, 0), m(3, 3, 1, 9, 999), false),
            // hash_mismatch breaks a missing+extra+size tie.
            (m(3, 3, 3, 1, 0), m(3, 3, 3, 2, 999), true),
            (m(3, 3, 3, 2, 0), m(3, 3, 3, 1, 999), false),
            // reproduced DESC is the final tiebreak.
            (m(3, 3, 3, 3, 5), m(3, 3, 3, 3, 4), true),
            (m(3, 3, 3, 3, 4), m(3, 3, 3, 3, 5), false),
            // fully equal -> false (irreflexive).
            (m(3, 3, 3, 3, 3), m(3, 3, 3, 3, 3), false),
            (ReproMetrics::default(), ReproMetrics::default(), false),
        ];
        for (a, b, expected) in cases {
            assert_eq!(a.better_than(b), *expected, "{a:?}.better_than({b:?})");
        }
    }

    #[test]
    fn better_than_is_irreflexive_for_arbitrary_values() {
        let samples = [
            ReproMetrics::default(),
            m(1, 2, 3, 4, 5),
            m(0, 0, 0, 0, 7),
            m(9, 0, 0, 0, 0),
        ];
        for s in samples {
            assert!(!s.better_than(&s), "{s:?} must not be better than itself");
        }
    }

    // --- Task 8 additions: caps, config, and datatype defaults ---

    #[test]
    fn select_any_caps_match_cpp() {
        assert_eq!(SELECT_ANY_CAP_NARROW, 64);
        assert_eq!(SELECT_ANY_CAP_MEDIUM, 256);
        assert_eq!(SELECT_ANY_CAP_FULL, 0);
    }

    #[test]
    fn solver_config_default_matches_cpp() {
        let c = SolverConfig::default();
        assert_eq!(c, CONFIG);
        assert_eq!(c.time_limit_seconds, 600);
        assert_eq!(c.max_checkpoints, 4096);
        assert_eq!(c.greedy_space_cap, 1_000_000_000);
        assert_eq!(c.component_space_cap, 10_000_000);
        assert_eq!(c.component_node_limit, 2_000_000);
        assert_eq!(c.residual_node_limit, 3_000_000);
        assert_eq!(c.focused_space_cap, 10_000_000);
        assert_eq!(c.focused_node_limit, 6_000_000);
        assert_eq!(c.exact_focused_space_cap, 12_000_000);
        assert_eq!(c.exact_focused_node_limit, 8_000_000);
        assert_eq!(c.global_space_cap, 10_000_000);
        assert_eq!(c.global_node_limit, 10_000_000);
        assert_eq!(c.full_pass_default_limit, 2_000_000);
        assert_eq!(c.full_pass_imperfect_limit, 6_000_000);
    }

    #[test]
    fn solver_progress_constants_and_default() {
        assert_eq!(SolverProgress::PROGRESS_NODE_INTERVAL, 1_000);
        assert_eq!(SolverProgress::PROGRESS_TIME_INTERVAL_MS, 1_000);
        let p = SolverProgress::default();
        assert_eq!(p.last_progress_nodes, 0);
        assert_eq!(p.estimated_total, 0);
        assert_eq!(p.pass_start_nodes, 0);
        assert!(p.deadline.is_none());
        assert!(!p.deadline_exceeded);
    }

    #[test]
    fn group_ref_and_option_cache_key_defaults() {
        let g = GroupRef::default();
        assert_eq!(
            (g.step_idx, g.group_idx, g.flat_start, g.plugin_count),
            (0, 0, 0, 0)
        );
        let k = OptionCacheKey::default();
        assert_eq!(k.group_idx, 0);
        assert_eq!(k.flags_sig, 0);
        assert_eq!(k.select_any_cap, 0);
        assert!(!k.exact_mode);
    }

    // OptionCacheKey / MemoKey equality is the only load-bearing property (the
    // hash functor is not observable); assert the field-wise comparison.
    #[test]
    fn option_cache_key_equality_is_field_wise() {
        let base = OptionCacheKey {
            group_idx: 3,
            flags_sig: 0xDEAD_BEEF,
            select_any_cap: 64,
            exact_mode: false,
        };
        assert_eq!(base, base);
        assert_ne!(
            base,
            OptionCacheKey {
                group_idx: 4,
                ..base
            }
        );
        assert_ne!(
            base,
            OptionCacheKey {
                flags_sig: 0xDEAD_BEEE,
                ..base
            }
        );
        assert_ne!(
            base,
            OptionCacheKey {
                select_any_cap: 256,
                ..base
            }
        );
        assert_ne!(
            base,
            OptionCacheKey {
                exact_mode: true,
                ..base
            }
        );
    }

    #[test]
    fn memo_key_equality_is_field_wise() {
        let base = MemoKey {
            next_idx: 2,
            flag_state_sig: 10,
            contested_sig: 20,
        };
        assert_eq!(base, base);
        assert_ne!(
            base,
            MemoKey {
                next_idx: 3,
                ..base
            }
        );
        assert_ne!(
            base,
            MemoKey {
                flag_state_sig: 11,
                ..base
            }
        );
        assert_ne!(
            base,
            MemoKey {
                contested_sig: 21,
                ..base
            }
        );
    }

    #[test]
    fn solver_stats_and_option_profile_defaults_are_zero() {
        let s = SolverStats::default();
        assert_eq!(s.dropped_extra_only_options, 0);
        assert_eq!(s.collapsed_equivalent_options, 0);
        assert_eq!(s.forced_unique_options, 0);
        assert_eq!(s.capped_select_any_options, 0);
        assert!(s.logged_group_options.is_empty());

        let p = OptionProfile::default();
        assert!(p.option.is_empty());
        assert_eq!(p.evidence_score, 0);
        assert_eq!(p.unique_support, 0);
        assert_eq!(p.useful_dests, 0);
        assert_eq!(p.extra_dests, 0);
        assert!(!p.sets_needed_flag);
        assert!(p.produced.is_empty());
        assert!(p.produced_atoms.is_empty());
        assert!(p.flags_written.is_empty());
    }
}
