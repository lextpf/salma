//! Datatypes shared by the CSP solver, its precompute step and the forward
//! simulator.
//!
//! Nothing here computes. The module holds the solver's data model plus two sets
//! of constants: the SelectAny option caps and the [`SolverConfig`] budgets.
//! [`crate::fomod_csp_solver`] runs the phases that consume these types,
//! [`crate::fomod_csp_precompute`] builds [`Precompute`], and
//! [`crate::fomod_csp_options`] builds [`CachedOptions`].
//!
//! - Scoring: [`ReproMetrics`] counts how closely a simulated install
//!   reproduces the target file tree, and [`ReproMetrics::better_than`] decides
//!   which of two candidates wins.
//! - Addressing: [`GroupRef`], [`GroupOption`], [`OptionProfile`],
//!   [`CachedOptions`], and the central read-only [`Precompute`].
//! - Search: [`SolverState`] and its parts, [`SolverStats`], [`FlagDelta`],
//!   [`SearchPlan`], the cache keys [`OptionCacheKey`] and [`MemoKey`], and the
//!   output [`SolverResult`].
//! - External state: [`InferenceOverrides`] forces the conditions inference
//!   cannot evaluate for itself.
//!
//! [`Precompute`] borrows its seven inputs as `&'a` or `Option<&'a>` and owns
//! only the indices it derives from them, so it can never outlive the installer,
//! atoms or target tree it indexes.
//!
//! Both cache keys derive `Hash`. The hash is never observable, because the maps
//! are used through lookup and insert and are never iterated into output; only
//! their field equality is load-bearing.
//!
//! ## SelectAny option caps
//!
//! Three constants bound how many options a SelectAny or SelectAtLeastOne group
//! may enumerate. The phase chooses the cap; this module only names the values.
//!
//! ```text
//! constant                value  used by
//! SELECT_ANY_CAP_NARROW      64  phases 1-4 (greedy, local search, targeted
//!                                repair, component decomposition, residual
//!                                repair, focused search) and the first
//!                                phase-5 global pass
//! SELECT_ANY_CAP_MEDIUM     256  the phase-5 "global-widened" and
//!                                "global-targeted" passes only
//! SELECT_ANY_CAP_FULL         0  the phase-5 "global-full" pass, and any group
//!                                put in exact mode by
//!                                effective_select_any_cap
//! ```
//!
//! A cap of 0 has two effects, not one. `reduce_options` applies no cap, and the
//! medium-group force-heuristic gate in `generate_raw_options` switches off
//! (that gate tests `select_any_cap > 0`). Switching the gate off widens
//! enumeration: an 8-plugin no-evidence SelectAny group then enumerates the full
//! 256-mask powerset instead of 10 heuristic options.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::fomod_atom::{AtomIndex, ExpandedAtoms, TargetTree};
use crate::fomod_dependency_evaluator::ExternalConditionOverride;
use crate::fomod_ir::FomodInstaller;
use crate::fomod_propagator::PropagationResult;

/// How closely a simulated install reproduces the target file tree, counted per
/// destination file.
///
/// The four error counters are better the lower they are; `reproduced` is better
/// the higher it is. All start at 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReproMetrics {
    /// Target files not produced by the simulation.
    pub missing: i32,
    /// Simulated files absent from the target.
    pub extra: i32,
    /// Files present in both but with differing (nonzero) sizes.
    pub size_mismatch: i32,
    /// Files whose nonzero content hashes differ and that were not already
    /// counted as a size mismatch. The simulator tests size first and reaches
    /// the hash test only in the `else` branch, so this bucket also holds files
    /// whose sizes differ when either side reports size 0 (unknown). Do not read
    /// it as "same size, different content".
    pub hash_mismatch: i32,
    /// Files successfully reproduced (size and hash match, or fall through).
    pub reproduced: i32,
}

impl ReproMetrics {
    /// True when all four error counters are zero. `reproduced` is deliberately
    /// ignored, so an all-zero tree (nothing reproduced, nothing wrong) counts
    /// as exact.
    pub fn exact(&self) -> bool {
        self.missing == 0 && self.extra == 0 && self.size_mismatch == 0 && self.hash_mismatch == 0
    }

    /// Strict-weak lexicographic ordering over the five counters.
    ///
    /// `self` is better than `rhs` exactly when
    ///
    /// ```text
    /// (missing, extra, size_mismatch, hash_mismatch, -reproduced)
    ///   <  (rhs.missing, rhs.extra, rhs.size_mismatch, rhs.hash_mismatch,
    ///       -rhs.reproduced)
    /// ```
    ///
    /// compared lexicographically. Only the last key runs the other way (more
    /// reproduced files is better), which the negation above makes explicit.
    /// Equal on all five keys returns `false`, so the relation is irreflexive
    /// and the solver keeps the first candidate it found at any given tuple.
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

/// Tri-state overrides for the external dependencies inference cannot evaluate.
///
/// Some FOMOD conditions depend on state that is not available during inference,
/// such as the game version or which other mods are installed. The caller forces
/// an individual conditional pattern or step-visibility condition to
/// [`ExternalConditionOverride::ForceTrue`] or
/// [`ExternalConditionOverride::ForceFalse`], or leaves it
/// [`ExternalConditionOverride::Unknown`] so the solver may prune or explore the
/// branch. Either vector may be shorter than the installer's own counts; a
/// missing entry falls back to normal evaluation in the forward simulator.
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
// SelectAny option caps. Phases 1 through 4 all run at the narrow cap; only
// phase 5 widens. The module doc's "SelectAny option caps" table is the
// authoritative per-phase attribution.
// ---------------------------------------------------------------------------

/// Cap used by solver phases 1 through 4 (greedy, local search, targeted
/// repair, component decomposition, residual repair, focused search) and by the
/// first phase-5 global pass.
pub const SELECT_ANY_CAP_NARROW: i32 = 64;
/// Cap used by the phase-5 `global-widened` and `global-targeted` passes only.
/// No earlier phase ever sees it.
pub const SELECT_ANY_CAP_MEDIUM: i32 = 256;
/// No cap: enumerate every valid combination. Used by the phase-5 `global-full`
/// pass and by any group put in exact mode. Because the force-heuristic gate
/// tests `select_any_cap > 0`, 0 also widens raw enumeration for medium
/// no-evidence SelectAny groups; see the module doc.
pub const SELECT_ANY_CAP_FULL: i32 = 0;

/// Reference to one plugin group, addressed by its `(step, group)` index pair.
///
/// `flat_start` is the group's offset into the global flat plugin array, so
/// per-plugin precomputed data (evidence, unique support) can be looked up
/// without re-walking the step hierarchy. It is captured in installer document
/// order, before the solver's per-step priority sort reorders the `GroupRef`
/// list, and therefore stays valid after that sort. See the "Index spaces"
/// figure on [`Precompute`].
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

/// Boolean mask of the plugins selected within one group, indexed by the group's
/// local plugin position rather than by the global flat index. `option[i]` true
/// means the i-th plugin of the group is selected.
pub type GroupOption = Vec<bool>;

/// Output of the CSP solver: plugin selections and match-quality counters.
///
/// `selections` is a 3-D boolean grid indexed `[step][group][plugin]`. The three
/// diagnostic fields (`phase_per_group`, `alternatives_per_group`,
/// `phase_reached`) are filled in one assembly block at the end of
/// [`crate::fomod_csp_solver::solve_fomod_csp`], not by the individual phases.
/// Read each of their docs before trusting them: none carries per-group
/// provenance, and `alternatives_per_group` is always zero.
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
    /// The run-level phase label, repeated for every group the propagator did
    /// not resolve, and the empty string for every group that is in
    /// `PropagationResult::resolved_groups`. Indexed `[step][group]`.
    ///
    /// This is not per-group provenance. Every unresolved group in every step
    /// receives the same string as `phase_reached`, including groups the phase
    /// never touched. The one distinction the field carries is "resolved by
    /// propagation" versus "left to the CSP".
    pub phase_per_group: Vec<Vec<String>>,
    /// Always all zeros. Indexed `[step][group]`.
    ///
    /// No ambiguity count is computed anywhere: a zero row is assigned per step
    /// and nothing writes to it again. The zeros are the output contract, not an
    /// unfinished feature, so do not "implement" the count. Downstream,
    /// `inference_diagnostics` maps a zero alternative count to the maximum
    /// ambiguity component (1.0), which is why every plugin scores that
    /// component at its ceiling. See `PARITY-NOTES.md`.
    pub alternatives_per_group: Vec<Vec<i32>>,
    /// Stable identifier of the highest CSP phase entered, one of `csp.greedy`,
    /// `csp.local_search`, `csp.repair`, `csp.focused`, `csp.fallback`. Empty
    /// when no diagnostic state was recorded.
    ///
    /// A phase sets its `ran_phaseN` marker before it is called, so a phase that
    /// returns immediately on its own precondition (component decomposition with
    /// one component, residual repair on a best that is not near-perfect) still
    /// wins the label. `csp.local_search` is the label of phase 2, which is
    /// component decomposition. The spelling is part of the emitted output, so
    /// renaming it to match the phase changes what consumers see.
    pub phase_reached: String,
}

/// Mutable search state carried through backtracking and local search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverSearchState {
    /// Current per-step/group/plugin selections.
    pub selections: Vec<Vec<Vec<bool>>>,
    /// Current FOMOD condition-flag values.
    pub flags: HashMap<String, String>,
    /// Total search nodes visited across all passes. Never reset between
    /// phases, which is what makes every `*_node_limit` cumulative.
    pub nodes_explored: i32,
    /// True when an exact (zero-error) reproduction has been found.
    pub found_exact: bool,
}

/// The best solution found so far in this solve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverBestResult {
    /// The best selection set and its error counts.
    pub best: SolverResult,
    /// Metrics corresponding to `best`.
    pub best_metrics: ReproMetrics,
    /// False until the first candidate is recorded.
    pub has_best: bool,
}

/// Progress reporting and deadline enforcement for one solve. A default
/// instance starts both time points at `Instant::now()` and leaves the deadline
/// unset.
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
    /// `solve_fomod_csp` sets it to `now + CONFIG.time_limit_seconds`.
    pub deadline: Option<Instant>,
    /// Set when the backtracker observes the deadline as passed. The check runs
    /// only on frame initialisation and only every 64th node, so the flag can
    /// lag the deadline. Once set it stops the current search and gates phases 2
    /// through 5, which makes it the one timing-dependent input to the result.
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

/// Search state, best result and progress, bundled so one value threads through
/// every phase.
#[derive(Debug, Clone, Default)]
pub struct SolverState {
    /// Current mutable search state.
    pub search: SolverSearchState,
    /// Best solution found so far.
    pub best: SolverBestResult,
    /// Progress tracking and deadline.
    pub progress: SolverProgress,
}

/// Diagnostic counters for option pruning and search behavior.
///
/// [`crate::fomod_csp_options`] writes the option-generation counters
/// (`dropped_extra_only_options`, `collapsed_equivalent_options`,
/// `capped_select_any_options`) and `logged_group_options`; the backtracker in
/// [`crate::fomod_csp_solver`] writes the search-tree pruning counters. All of
/// them accumulate over one whole solve, and nothing branches on them:
/// `solve_fomod_csp` logs them and that is their only use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverStats {
    /// Options dropped because they only produce extra files.
    pub dropped_extra_only_options: i32,
    /// Options collapsed because their produced atom keys (`"dest|source"`) and
    /// their written flags fold to the same signature. Two options that produce
    /// the same destinations from different sources do not collapse, and neither
    /// do two options with the same destinations but different flag writes. This
    /// counts duplicate encounters, including the case where the newcomer
    /// replaces the incumbent, so it is not "options removed".
    pub collapsed_equivalent_options: i32,
    /// Groups forced to a single option (unique evidence). Nothing increments
    /// this, so it stays 0 for every solve.
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
    /// Subtrees abandoned because the solve-wide node count reached the plan's
    /// `node_limit`.
    pub pruned_node_limit: i32,
    /// Branches abandoned because the backtrack stack exceeded the max depth.
    pub max_depth_aborts: i32,
    /// Write-only marker, set for a group on every option-cache miss for that
    /// group. Sized to `pre.groups.len()` by the solver entry point.
    ///
    /// Nothing in the crate reads this flag, so it suppresses no log output: a
    /// group emits its two `[solver]` lines again on each miss, and a group
    /// legitimately has one cache entry per (flag signature, effective cap,
    /// exact mode) tuple.
    pub logged_group_options: Vec<bool>,
}

/// The effect of selecting one plugin combination in a group: which destinations
/// it produces, how much evidence supports it, and which condition flags it
/// writes. Drives pruning and ordering during option enumeration.
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

/// One group's surviving options and their profiles. The two vectors share an
/// index: `profiles[i]` describes `options[i]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CachedOptions {
    /// Valid selection combinations for this group.
    pub options: Vec<GroupOption>,
    /// Corresponding profile for each option.
    pub profiles: Vec<OptionProfile>,
}

/// The read-only data every CSP solver phase shares, built once before the solve
/// by [`crate::fomod_csp_precompute::build_precompute`].
///
/// The seven input fields are borrowed for `'a`, so a `Precompute` cannot
/// outlive the installer, atoms or target tree it indexes; every other field is
/// a derived index this struct owns.
///
/// ## Index spaces
///
/// Three index spaces coexist here. Mixing them is the failure mode this figure
/// exists to prevent.
///
/// ```text
/// installer.steps                 flat plugin index space
///   step 0                         0   1   2   3   4   5
///     group A (2 plugins)  ----->  [ A0  A1 ]
///     group B (1 plugin)   ----->           [ B0 ]
///   step 1
///     group C (3 plugins)  ----->                [ C0  C1  C2 ]
///
/// GroupRef { step_idx, group_idx, flat_start, plugin_count }
///   A: { 0, 0, flat_start = 0, count = 2 }
///   B: { 0, 1, flat_start = 2, count = 1 }
///   C: { 1, 0, flat_start = 3, count = 3 }
///
/// indexed by flat plugin index (len = total plugins in the installer):
///   evidence[]   plugin_unique_support[]   plugin_to_group[]
///
/// indexed by group index, i.e. a position in `groups` (len = groups.len()):
///   group_sets_flags[]  group_reads_flags[]  group_cache_flags[]
///   group_dests[]
///
/// group indices also appear as values, never as keys, in:
///   plugin_to_group[]  dest_to_groups  dest_to_size_match_groups
///   dest_to_hash_capable_groups  flag_to_setter_groups  components[][]
///
/// local option-mask position = flat plugin index - flat_start
/// ```
///
/// A group index is a position in this struct's `groups` vector, which the
/// solver has already priority-sorted per step. It is not `GroupRef::group_idx`,
/// the position inside the step. `flat_start` is captured before that sort, so
/// the flat space still follows document order.
///
/// ## Keying asymmetry between the reverse indices
///
/// `group_dests` and `dest_to_groups` are built from the atoms, so they cover
/// every non-excluded destination any plugin can produce, whether or not the
/// target tree holds it. `dest_to_plugins`, `dest_to_size_match_groups`,
/// `dest_to_hash_capable_groups` and `conditional_dests` are built from the
/// target tree, so their only keys are non-excluded destinations present in the
/// target. `dest_to_plugins.get(d)` returning `None` therefore means "not a
/// target file", not "no producer".
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

    /// Every flag name read by any of four condition sources: step-visibility
    /// conditions, plugin `type_patterns` conditions, plugin `dependencies`
    /// conditions, and installer conditional-pattern conditions.
    ///
    /// Membership is load-bearing twice over: it decides
    /// `OptionProfile::sets_needed_flag`, which exempts an option from the
    /// extra-only drop, and it seeds `memo_flags`.
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

    /// Groups that can produce each destination (sorted-unique). Keyed by every
    /// non-excluded destination any plugin can produce, target file or not.
    pub dest_to_groups: HashMap<String, Vec<i32>>,
    /// Flat plugin indices that can produce each destination (sorted-unique).
    /// Keyed by non-excluded target destinations only. No size or hash filter is
    /// applied: every plugin-origin atom on the destination is listed.
    pub dest_to_plugins: HashMap<String, Vec<i32>>,
    /// Groups that can produce a size-matching file for the destination
    /// (sorted-unique). Keyed by non-excluded target destinations only. A size
    /// of 0 on either side counts as a match, because 0 means "unknown".
    pub dest_to_size_match_groups: HashMap<String, Vec<i32>>,
    /// Groups that can produce a hash-matching file for the destination
    /// (sorted-unique). Keyed by non-excluded target destinations only. This is
    /// the size-match test plus a hash test in which a 0 hash on either side
    /// counts as a match, because 0 means "not hashed".
    pub dest_to_hash_capable_groups: HashMap<String, Vec<i32>>,

    /// Non-excluded target destinations that at least one conditional-pattern
    /// atom can produce. A destination gated only by plugin selection, or by a
    /// flag-gated plugin, is not in this set even though its production also
    /// depends on flags.
    pub conditional_dests: HashSet<String>,
    /// Every flat plugin index (sorted ascending, unique) that produces at least
    /// one non-excluded destination present in the target tree, whether or not
    /// that destination is actually contested.
    ///
    /// The name is wider than it sounds: each plugin-origin atom's plugin index
    /// is inserted unconditionally, so the later "more than one source" widening
    /// pass adds nothing. Do not narrow this to true conflicts as an
    /// optimisation. The set feeds `contested_signature`, so a narrower set
    /// changes every `MemoKey` and therefore which subtrees the backtracker
    /// prunes.
    pub contested_plugins: Vec<i32>,
    /// Connected components of the group dependency graph. Two groups are in the
    /// same component when they share a produced destination or a flag (setter
    /// to reader, or setter to setter). Groups in different components share
    /// neither, so each component can be searched without regard to the others.
    ///
    /// Ordered size-descending with a min-member-ascending tiebreak; each
    /// component is itself sorted ascending. Nothing runs concurrently: phase 2
    /// walks the components in one sequential loop and phase 5 flattens them
    /// into a single order.
    pub components: Vec<Vec<i32>>,
}

/// Cache key identifying one group's enumerated options under one flag state.
/// Equality over all four fields is what the option cache relies on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OptionCacheKey {
    /// Index into `Precompute::groups`.
    pub group_idx: i32,
    /// Hash of the subset of flag values this group's conditions read, built by
    /// `hash_flag_subset(flags, group_cache_flags[gidx])`.
    pub flags_sig: u64,
    /// The effective SelectAny cap already resolved for this group, that is
    /// `effective_select_any_cap(gidx, cap, exact_groups)`, so an exact-mode
    /// group stores 0 here. A cap change is a deliberate cache miss: it
    /// re-enumerates and re-reduces the group.
    pub select_any_cap: i32,
    /// True when the group is in exact mode. Exact mode both zeroes
    /// `select_any_cap` and disables the extra-only option drop, so it is a
    /// separate key field rather than a shorthand for `select_any_cap == 0`.
    pub exact_mode: bool,
}

/// Memoization key for subtree pruning during backtracking.
///
/// `flag_state_sig` is `hash_flag_subset(flags, memo_flags)` and `contested_sig`
/// is `contested_signature(...)`, both byte-exact folds computed in
/// [`crate::fomod_csp_solver`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MemoKey {
    /// Next group index in the search order.
    pub next_idx: i32,
    /// Hash of current condition-flag values.
    pub flag_state_sig: u64,
    /// Hash of current selections for contested plugins.
    pub contested_sig: u64,
}

/// Undo record for a single flag change during backtracking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagDelta {
    /// Flag that was modified.
    pub name: String,
    /// True if the flag existed before the change.
    pub had_value: bool,
    /// Previous value (meaningful only when `had_value` is true).
    pub old_value: String,
}

/// Configuration for a single backtracking pass over a set of groups.
#[derive(Debug, Clone, Default)]
pub struct SearchPlan {
    /// Group indices in the order they will be explored.
    pub order: Vec<i32>,
    /// Inverse map: `order_pos[group_idx]` = position in `order` (-1 if absent).
    pub order_pos: Vec<i32>,
    /// Ceiling on the solve-wide node count, not a per-pass budget; 0 is
    /// unlimited.
    ///
    /// The backtracker compares this against `SolverSearchState::nodes_explored`,
    /// which counts every candidate evaluated since the solve began. Every
    /// earlier phase therefore spends the same budget, and a late pass whose
    /// limit is already exceeded explores no nodes at all. The `SolverConfig`
    /// per-phase limits work the same way.
    pub node_limit: i32,
    /// Subtree memoization table.
    pub memo: HashMap<MemoKey, ReproMetrics>,
    /// When true, flags are updated incrementally rather than rebuilt.
    pub incremental_flags: bool,
}

/// Time budgets, search-space caps and node limits for each phase of the FOMOD
/// CSP solver.
///
/// Every `*_node_limit` field is a cumulative ceiling on the solve-wide node
/// counter, not an allowance for its own phase. The counter starts at 0 once per
/// `solve_fomod_csp` call and is never reset, so a phase whose limit is already
/// below the current count explores nothing. After phase 2 spends 2,000,000
/// nodes, phase 3's `residual_node_limit` of 3,000,000 leaves 1,000,000.
///
/// The `*_space_cap` fields are unrelated. They bound the combination-count
/// estimate, which saturates at `cap + 1`. For phases 2, 4 and 5 that estimate
/// only chooses between "no limit" and the matching node limit;
/// `greedy_space_cap` bounds the estimate every backtrack pass reports in its
/// `space=` log line and uses as the progress-bar denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolverConfig {
    /// Maximum wall-clock time for the entire solve (seconds). Enforced from the
    /// start of `solve_fomod_csp` and checked inside the backtracker; when it
    /// expires the solve returns the best result found so far.
    pub time_limit_seconds: i32,
    /// Maximum number of state checkpoints kept during backtracking. Reaching it
    /// abandons the branch and logs a warning.
    pub max_checkpoints: usize,
    /// Estimate cap passed to `estimate_search_space` for every
    /// `run_backtrack_pass` (combinations). The estimate saturates at
    /// `cap + 1`; the value only drives the progress bar and the log line.
    pub greedy_space_cap: u64,
    /// Estimate cap per component in Phase 2 (combinations). An estimate at or
    /// below `component_node_limit` runs the component pass unlimited.
    pub component_space_cap: u64,
    /// Solve-wide node ceiling for a Phase 2 component backtrack.
    pub component_node_limit: i32,
    /// Solve-wide node ceiling for the Phase 3 residual-repair backtrack.
    pub residual_node_limit: i32,
    /// Estimate cap for mismatch-focused search in Phase 4 (combinations).
    pub focused_space_cap: u64,
    /// Solve-wide node ceiling for the Phase 4 focused backtrack.
    pub focused_node_limit: i32,
    /// Estimate cap for the exact-match focused fallback in Phase 4
    /// (combinations).
    pub exact_focused_space_cap: u64,
    /// Solve-wide node ceiling for the exact-match focused backtrack.
    pub exact_focused_node_limit: i32,
    /// Estimate cap for the global fallback passes in Phase 5 (combinations).
    pub global_space_cap: u64,
    /// Solve-wide node ceiling for a Phase 5 global backtrack pass.
    pub global_node_limit: i32,
    /// Extra solve-wide node ceiling applied to an uncapped (SelectAny cap 0)
    /// global pass. Used unless a best result exists and still has missing or
    /// extra files.
    pub full_pass_default_limit: i32,
    /// Replaces `full_pass_default_limit` on an uncapped global pass when a best
    /// result exists and still has missing or extra files.
    pub full_pass_imperfect_limit: i32,
}

impl SolverConfig {
    /// The default solver configuration. An associated `const` so [`CONFIG`]
    /// can itself be a `const`.
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

/// The solver configuration every phase reads. Nothing overrides it at runtime.
pub const CONFIG: SolverConfig = SolverConfig::DEFAULT;

#[cfg(test)]
mod tests {
    use super::*;

    // --- defaults ---

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
            // reproduced, descending, is the final tiebreak.
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

    // --- caps, config, and datatype defaults ---

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

    // Field equality is the only load-bearing property of both key types (the
    // derived hash is never observable), so pin the field-wise comparison.
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
