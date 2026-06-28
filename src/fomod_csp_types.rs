/*!
 * @brief defines shared CSP state, indices, metrics, and budgets.
 * @author Alex (https://github.com/lextpf)
 *
 * SelectAny caps are 64 for narrow search, 256 for widened search, and 0 for full search.
 * Precompute borrows source data and owns only derived indices.
 */

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::fomod_atom::{AtomIndex, ExpandedAtoms, TargetTree};
use crate::fomod_dependency_evaluator::ExternalConditionOverride;
use crate::fomod_ir::FomodInstaller;
use crate::fomod_propagator::PropagationResult;

/**
 * @struct ReproMetrics
 * @brief count destination differences between simulated and target trees.
 * @author Alex (https://github.com/lextpf)
 *
 * the four error counters are better the lower they are; `reproduced` is better the higher it is.
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReproMetrics {
    pub missing: i32,
    pub extra: i32,
    pub size_mismatch: i32,
    /**
     * @brief count hash mismatches not already counted as size mismatches.
     * @author Alex (https://github.com/lextpf)
     */
    pub hash_mismatch: i32,
    pub reproduced: i32,
}

impl ReproMetrics {
    pub fn exact(&self) -> bool {
        self.missing == 0 && self.extra == 0 && self.size_mismatch == 0 && self.hash_mismatch == 0
    }

    /**
     * @fn better_than(&self, &ReproMetrics) -> bool
     * @brief strict-weak lexicographic ordering over the five counters.
     * @author Alex (https://github.com/lextpf)
     *
     */
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

/**
 * @struct InferenceOverrides
 * @brief tri-state overrides for the external dependencies inference cannot evaluate.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InferenceOverrides {
    pub conditional_active: Vec<ExternalConditionOverride>,
    pub step_visible: Vec<ExternalConditionOverride>,
}

/**
 * @brief cap SelectAny enumeration before global widened or targeted passes.
 * @author Alex (https://github.com/lextpf)
 */
pub const SELECT_ANY_CAP_NARROW: i32 = 64;
/**
 * @brief cap used by the phase-5 global-widened and global-targeted passes only.
 * @author Alex (https://github.com/lextpf)
 */
pub const SELECT_ANY_CAP_MEDIUM: i32 = 256;
/**
 * @brief no cap: enumerate every valid combination.
 * @author Alex (https://github.com/lextpf)
 *
 * because the force-heuristic gate tests `select_any_cap > 0`, 0 also widens raw enumeration for
 * medium no-evidence SelectAny groups.
 */
pub const SELECT_ANY_CAP_FULL: i32 = 0;

/**
 * @struct GroupRef
 * @brief reference to one plugin group, addressed by its (step, group) index pair.
 * @author Alex (https://github.com/lextpf)
 *
 * `flat_start` is the group's offset into the global flat plugin array, so per-plugin precomputed
 * data (evidence, unique support) can be looked up without re-walking the step hierarchy.
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupRef {
    pub step_idx: i32,
    pub group_idx: i32,
    pub flat_start: i32,
    pub plugin_count: i32,
}

/**
 * @brief select plugins by local position within one group.
 * @author Alex (https://github.com/lextpf)
 *
 */
pub type GroupOption = Vec<bool>;

/**
 * @struct SolverResult
 * @brief output of the CSP solver: plugin selections and match-quality counters.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverResult {
    pub selections: Vec<Vec<Vec<bool>>>,
    pub inferred_flags: HashMap<String, String>,
    pub exact_match: bool,
    /**
     * @brief CSP search-tree node count (diagnostics).
     * @author Alex (https://github.com/lextpf)
     */
    pub nodes_explored: i32,
    pub missing: i32,
    pub extra: i32,
    pub size_mismatch: i32,
    pub hash_mismatch: i32,
    /**
     * @brief record the highest phase for unresolved groups and empty strings for resolved groups.
     * @author Alex (https://github.com/lextpf)
     *
     * every unresolved group in every step receives the same string as `phase_reached`, including
     * groups the phase never touched.
     */
    pub phase_per_group: Vec<Vec<String>>,
    /**
     * @brief reserved alternative counts; the current solver writes zero.
     * @author Alex (https://github.com/lextpf)
     *
     * zero maps to the maximum ambiguity component, 1.0.
     */
    pub alternatives_per_group: Vec<Vec<i32>>,
    /**
     * @brief record the highest entered phase as a stable CSP identifier.
     * @author Alex (https://github.com/lextpf)
     *
     * empty when no diagnostic state was recorded.
     */
    pub phase_reached: String,
}

/**
 * @struct SolverSearchState
 * @brief mutable search state carried through backtracking and local search.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverSearchState {
    pub selections: Vec<Vec<Vec<bool>>>,
    pub flags: HashMap<String, String>,
    /**
     * @brief total search nodes visited across all passes.
     * @author Alex (https://github.com/lextpf)
     *
     * never reset between phases, which is what makes every `*_node_limit` cumulative.
     */
    pub nodes_explored: i32,
    /**
     * @brief true when an exact (zero-error) reproduction has been found.
     * @author Alex (https://github.com/lextpf)
     */
    pub found_exact: bool,
}

/**
 * @struct SolverBestResult
 * @brief the best solution found so far in this solve.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverBestResult {
    /**
     * @brief the best selection set and its error counts.
     * @author Alex (https://github.com/lextpf)
     */
    pub best: SolverResult,
    pub best_metrics: ReproMetrics,
    pub has_best: bool,
}

/**
 * @struct SolverProgress
 * @brief progress reporting and deadline enforcement for one solve.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone)]
pub struct SolverProgress {
    pub last_progress_nodes: i32,
    pub last_progress_time: Instant,
    pub estimated_total: i64,
    pub pass_start_time: Instant,
    pub pass_start_nodes: i64,
    pub deadline: Option<Instant>,
    /**
     * @brief set when the backtracker observes the deadline as passed.
     * @author Alex (https://github.com/lextpf)
     */
    pub deadline_exceeded: bool,
}

impl SolverProgress {
    /**
     * @brief minimum nodes between progress log lines.
     * @author Alex (https://github.com/lextpf)
     */
    pub const PROGRESS_NODE_INTERVAL: i32 = 1_000;
    /**
     * @brief minimum milliseconds between progress log lines.
     * @author Alex (https://github.com/lextpf)
     */
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

/**
 * @struct SolverState
 * @brief search state, best result and progress, bundled so one value threads through every phase.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default)]
pub struct SolverState {
    pub search: SolverSearchState,
    pub best: SolverBestResult,
    pub progress: SolverProgress,
}

/**
 * @struct SolverStats
 * @brief diagnostic counters for option pruning and search behavior.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolverStats {
    /**
     * @brief options dropped because they only produce extra files.
     * @author Alex (https://github.com/lextpf)
     */
    pub dropped_extra_only_options: i32,
    /**
     * @brief count options collapsed by destination, source and flag effects.
     * @author Alex (https://github.com/lextpf)
     */
    pub collapsed_equivalent_options: i32,
    pub forced_unique_options: i32,
    pub capped_select_any_options: i32,
    /**
     * @brief subtrees pruned because all options are extra-only.
     * @author Alex (https://github.com/lextpf)
     */
    pub pruned_extra_only: i32,
    pub pruned_lower_bound: i32,
    pub pruned_memo: i32,
    /**
     * @brief groups skipped because their step is not visible.
     * @author Alex (https://github.com/lextpf)
     */
    pub skipped_invisible: i32,
    /**
     * @brief subtrees abandoned because the solve-wide node count reached the plan's node_limit.
     * @author Alex (https://github.com/lextpf)
     */
    pub pruned_node_limit: i32,
    /**
     * @brief branches abandoned because the backtrack stack exceeded the max depth.
     * @author Alex (https://github.com/lextpf)
     */
    pub max_depth_aborts: i32,
    /**
     * @brief write-only marker, set for a group on every option-cache miss for that group.
     * @author Alex (https://github.com/lextpf)
     */
    pub logged_group_options: Vec<bool>,
}

/**
 * @struct OptionProfile
 * @brief describe one group option's files, evidence and flag writes.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OptionProfile {
    pub option: GroupOption,
    pub evidence_score: i32,
    pub unique_support: i32,
    pub useful_dests: i32,
    pub extra_dests: i32,
    pub sets_needed_flag: bool,
    pub produced: HashSet<String>,
    pub produced_atoms: HashSet<String>,
    pub flags_written: HashMap<String, String>,
}

/**
 * @struct CachedOptions
 * @brief one group's surviving options and their profiles.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CachedOptions {
    pub options: Vec<GroupOption>,
    pub profiles: Vec<OptionProfile>,
}

/**
 * @struct Precompute
 * @brief hold immutable solver data built before phase execution.
 * @author Alex (https://github.com/lextpf)
 *
 * source data is borrowed for the solve. derived indices belong to this value.
 *
 * ### :material-link-variant: index spaces
 *
 * @verbatim
 * installer hierarchy          flat plugin indices
 * step 0                       0   1   2   3   4   5
 *   group a (2 plugins) -----> (a0  a1)
 *   group b (1 plugin)  ----->         (b0)
 * step 1
 *   group c (3 plugins) ----->             (c0  c1  c2)
 *
 * GroupRef   step_idx   group_idx   flat_start   plugin_count
 * a          0          0           0            2
 * b          0          1           2            1
 * c          1          0           3            3
 *
 * local option index = flat plugin index - flat_start
 * @endverbatim
 *
 * a position in `groups` is not `GroupRef::group_idx`, which is a position within one step.
 * `flat_start` follows document order even when priority sorting changes `groups`.
 *
 * ### :material-shield-lock: destination invariants
 *
 * `group_dests` and `dest_to_groups` cover every non-excluded produced destination.
 * `dest_to_plugins`, `dest_to_size_match_groups`, `dest_to_hash_capable_groups`, and
 * `conditional_dests` cover only target destinations.
 */
#[derive(Debug, PartialEq)]
pub struct Precompute<'a> {
    pub installer: &'a FomodInstaller,
    pub atoms: &'a ExpandedAtoms,
    pub atom_index: &'a AtomIndex,
    pub target: &'a TargetTree,
    pub excluded: &'a HashSet<String>,
    pub overrides: Option<&'a InferenceOverrides>,
    pub propagation: Option<&'a PropagationResult>,

    /**
     * @brief all groups across all steps, in the caller-provided order.
     * @author Alex (https://github.com/lextpf)
     */
    pub groups: Vec<GroupRef>,
    pub evidence: Vec<i32>,

    pub plugin_to_group: Vec<i32>,
    pub plugin_unique_support: Vec<i32>,

    /**
     * @brief collect flags read by visibility, type, dependency or conditional rules.
     * @author Alex (https://github.com/lextpf)
     */
    pub needed_flags: HashSet<String>,
    pub group_sets_flags: Vec<HashSet<String>>,
    pub group_reads_flags: Vec<HashSet<String>>,
    /**
     * @brief per-group: sorted flag keys for cache hashing (byte-ascending).
     * @author Alex (https://github.com/lextpf)
     */
    pub group_cache_flags: Vec<Vec<String>>,
    pub flag_to_setter_groups: HashMap<String, Vec<i32>>,
    pub memo_flags: Vec<String>,
    pub group_dests: Vec<HashSet<String>>,

    pub dest_to_groups: HashMap<String, Vec<i32>>,
    /**
     * @brief flat plugin indices that can produce each destination (sorted-unique).
     * @author Alex (https://github.com/lextpf)
     */
    pub dest_to_plugins: HashMap<String, Vec<i32>>,
    /**
     * @brief groups that can produce a size-matching file for the destination (sorted-unique).
     * @author Alex (https://github.com/lextpf)
     *
     * a size of 0 on either side counts as a match, because 0 means "unknown".
     */
    pub dest_to_size_match_groups: HashMap<String, Vec<i32>>,
    /**
     * @brief groups that can produce a hash-matching file for the destination (sorted-unique).
     * @author Alex (https://github.com/lextpf)
     *
     * this is the size-match test plus a hash test in which a 0 hash on either side counts as a
     * match, because 0 means "not hashed".
     */
    pub dest_to_hash_capable_groups: HashMap<String, Vec<i32>>,

    /**
     * @brief collect non-excluded target destinations produced by conditional atoms.
     * @author Alex (https://github.com/lextpf)
     */
    pub conditional_dests: HashSet<String>,
    /**
     * @brief list sorted plugins that produce non-excluded target destinations.
     * @author Alex (https://github.com/lextpf)
     */
    pub contested_plugins: Vec<i32>,
    /**
     * @brief connected components of the group dependency graph.
     * @author Alex (https://github.com/lextpf)
     *
     * groups in different components share neither, so each component can be searched without
     * regard to the others.
     */
    pub components: Vec<Vec<i32>>,
}

/**
 * @struct OptionCacheKey
 * @brief cache key identifying one group's enumerated options under one flag state.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OptionCacheKey {
    pub group_idx: i32,
    pub flags_sig: u64,
    /**
     * @brief store the effective SelectAny cap; zero means exact mode.
     * @author Alex (https://github.com/lextpf)
     */
    pub select_any_cap: i32,
    /**
     * @brief true when the group is in exact mode.
     * @author Alex (https://github.com/lextpf)
     */
    pub exact_mode: bool,
}

/**
 * @struct MemoKey
 * @brief memoization key for subtree pruning during backtracking.
 * @author Alex (https://github.com/lextpf)
 *
 * `flag_state_sig` is `hash_flag_subset(flags, memo_flags)` and `contested_sig` is
 * `contested_signature(...)`, both byte-exact folds computed in [`crate::fomod_csp_solver`].
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MemoKey {
    /**
     * @brief next group index in the search order.
     * @author Alex (https://github.com/lextpf)
     */
    pub next_idx: i32,
    pub flag_state_sig: u64,
    pub contested_sig: u64,
}

/**
 * @struct FlagDelta
 * @brief undo record for a single flag change during backtracking.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagDelta {
    pub name: String,
    pub had_value: bool,
    pub old_value: String,
}

/**
 * @struct SearchPlan
 * @brief configuration for a single backtracking pass over a set of groups.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default)]
pub struct SearchPlan {
    /**
     * @brief group indices in the order they will be explored.
     * @author Alex (https://github.com/lextpf)
     */
    pub order: Vec<i32>,
    /**
     * @brief map each group index to its search-order position or minus one.
     * @author Alex (https://github.com/lextpf)
     */
    pub order_pos: Vec<i32>,
    /**
     * @brief ceiling on the solve-wide node count, not a per-pass budget; 0 is unlimited.
     * @author Alex (https://github.com/lextpf)
     *
     * every earlier phase therefore spends the same budget, and a late pass whose limit is already
     * exceeded explores no nodes at all.
     */
    pub node_limit: i32,
    pub memo: HashMap<MemoKey, ReproMetrics>,
    pub incremental_flags: bool,
}

/**
 * @struct SolverConfig
 * @brief time budgets, search-space caps and node limits for each phase of the FOMOD CSP solver.
 * @author Alex (https://github.com/lextpf)
 *
 * the counter starts at 0 once per `solve_fomod_csp` call and is never reset, so a phase whose
 * limit is already below the current count explores nothing.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolverConfig {
    /**
     * @brief maximum wall-clock time for the entire solve (seconds).
     * @author Alex (https://github.com/lextpf)
     */
    pub time_limit_seconds: i32,
    /**
     * @brief maximum number of state checkpoints kept during backtracking.
     * @author Alex (https://github.com/lextpf)
     */
    pub max_checkpoints: usize,
    /**
     * @brief limit estimated combinations for each backtracking pass.
     * @author Alex (https://github.com/lextpf)
     */
    pub greedy_space_cap: u64,
    pub component_space_cap: u64,
    pub component_node_limit: i32,
    pub residual_node_limit: i32,
    pub focused_space_cap: u64,
    pub focused_node_limit: i32,
    pub exact_focused_space_cap: u64,
    pub exact_focused_node_limit: i32,
    pub global_space_cap: u64,
    pub global_node_limit: i32,
    pub full_pass_default_limit: i32,
    pub full_pass_imperfect_limit: i32,
}

impl SolverConfig {
    /**
     * @brief the default solver configuration.
     * @author Alex (https://github.com/lextpf)
     */
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

/**
 * @brief the solver configuration every phase reads.
 * @author Alex (https://github.com/lextpf)
 */
pub const CONFIG: SolverConfig = SolverConfig::DEFAULT;

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn exact_true_when_all_errors_zero_regardless_of_reproduced() {
        assert!(ReproMetrics::default().exact());
        assert!(
            ReproMetrics {
                reproduced: 999,
                ..Default::default()
            }
            .exact()
        );
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
        let cases: &[(ReproMetrics, ReproMetrics, bool)] = &[
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
            (m(3, 3, 3, 3, 5), m(3, 3, 3, 3, 4), true),
            (m(3, 3, 3, 3, 4), m(3, 3, 3, 3, 5), false),
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

    // equality is fieldwise; key hashes are not serialized or iterated.
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
