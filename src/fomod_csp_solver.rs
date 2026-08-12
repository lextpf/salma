//! Multi-phase CSP solver for FOMOD selection inference.
//!
//! The single public entry point [`solve_fomod_csp`] compares a FOMOD
//! installer's option space against an already-installed target file tree and
//! returns the best-scoring `[step][group][plugin]` selection grid it can find.
//!
//! ## The five phases
//!
//! Phases run in order, each behind a gate. A phase runs only while the solve is
//! still inexact and inside its time budget.
//!
//! ```text
//!   seed: every plugin deselected, select_any_cap = 64 (narrow)
//!     |
//!    [1] run_initial_phases
//!     |     greedy -> local search (at most 5 passes) -> targeted repair
//!     |
//!    gate  !exact && !deadline
//!     |
//!    [2] run_component_decomposition
//!     |     per component: local search, then a backtrack pass
//!     |
//!    gate  !exact && !deadline
//!     |
//!    [3] run_residual_repair
//!     |     local search, then a backtrack pass, over the affected groups
//!     |
//!    gate  !exact && !deadline
//!     |
//!    [4] run_focused_search
//!     |     local search -> backtrack -> focused-exact backtrack
//!     |
//!    gate  !exact && !deadline
//!     |
//!    [5] run_global_fallback: one group order, a widening cap ladder
//!           global(64) -> global-widened(256)
//!                      -> global-targeted(256, affected groups in exact mode)
//!                      -> global-full(0)
//!
//!   gate legend
//!     exact    = SolverSearchState::found_exact, set by evaluate_candidate the
//!                moment a candidate reproduces the target with zero errors
//!     deadline = SolverProgress::deadline_exceeded, a 600 s wall clock
//!                (CONFIG.time_limit_seconds)
//!
//!   phases 2, 3 and 4 can also return without doing anything:
//!     [2] when the installer decomposes into 1 component or fewer
//!     [3] when there is no best yet, when that best is not near-perfect
//!         (missing 0, extra 0, size_mm <= 1, hash_mm <= 2), or when the
//!         mismatch-affected group set is empty or covers every group
//!     [4] when there is no best yet, or when the mismatch-affected group set
//!         is empty or covers every group
//! ```
//!
//! Those gates are the whole control flow. Nothing else short-circuits, and no
//! phase is skipped because constraint propagation resolved a group.
//!
//! ## Scoring oracle and the first-found-wins tie rule
//!
//! Every candidate is scored by replaying it through the forward simulator
//! ([`simulate`]) and diffing against the target ([`compare_trees`]).
//! [`ReproMetrics::better_than`](crate::fomod_csp_types::ReproMetrics::better_than)
//! rejects an equal metric tuple, so [`evaluate_candidate`] keeps the first
//! candidate discovered at any metric tuple. Whenever ties exist, every
//! visitation, iteration and sort order is therefore observable in the final
//! grid, which is why each of those orders carries a total tiebreak.
//!
//! For a given phase coverage the solve is run-to-run deterministic. Phase
//! coverage itself is not, because the 600 s wall-clock deadline
//! (`CONFIG.time_limit_seconds`) gates phases 2 through 5 and can cut a running
//! backtrack short: a heavily loaded machine can stop at an earlier phase and
//! return a different grid for the same input. That deadline is the only
//! timing-dependent input to the result; everything else is a pure function of
//! the installer, the atoms and the target tree.
//!
//! ## Iterative backtracker
//!
//! [`backtrack`] walks an explicit stack instead of recursing, so a deep
//! installer cannot overflow the native stack. It carries branch-and-bound
//! pruning ([`lower_bound`] / [`cannot_beat`]), subtree memoization keyed by
//! [`hash_flag_subset`] plus [`contested_signature`], an extra-only option
//! prune, and node-limit and deadline guards.
//!
//! ## Progress logging
//!
//! Phases narrate through the `[solver]` log tag, including a tqdm-style
//! progress bar built by [`format_count`], [`format_duration`],
//! [`format_option_cap`] and [`build_tqdm_bar`]. The per-node progress check in
//! [`evaluate_candidate`] runs only when `estimated_total > 1`, so a pass that
//! never sets an estimate never reads the clock.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::fomod_atom::{AtomIndex, ExpandedAtoms, TargetTree};
use crate::fomod_csp_options::{get_options_for_group, group_name, is_exact_group_mode};
use crate::fomod_csp_precompute::{build_precompute, compute_evidence, hash_flag_subset};
use crate::fomod_csp_types::{
    CONFIG, CachedOptions, GroupOption, GroupRef, InferenceOverrides, MemoKey, OptionCacheKey,
    Precompute, ReproMetrics, SELECT_ANY_CAP_FULL, SELECT_ANY_CAP_MEDIUM, SELECT_ANY_CAP_NARROW,
    SearchPlan, SolverProgress, SolverResult, SolverState, SolverStats,
};
use crate::fomod_dependency_evaluator::{
    ExternalConditionOverride, evaluate_condition_inferred, evaluate_plugin_type,
};
use crate::fomod_forward_simulator::{
    collect_mismatched_dests, compare_trees, compare_trees_impl, simulate,
};
use crate::fomod_ir::{FomodGroupType, FomodInstaller, FomodStep};
use crate::fomod_propagator::PropagationResult;
use crate::logger::Logger;
use crate::types::PluginType;
use crate::utils::hash_combine;

/// Maximum number of memoization entries kept before the whole table is cleared.
const MAX_MEMO_ENTRIES: usize = 100_000;

/// Maximum backtrack stack depth before a branch is abandoned.
const MAX_BACKTRACK_DEPTH: usize = 500;

/// Reconstruct the condition-flag map by replaying selected plugins up to a
/// given `(step, group)` pair.
///
/// `stop_step_idx < 0` replays the whole installer. Steps are walked in document
/// order; the incremental [`advance_flags_past_group`] path instead advances
/// groups in the priority-sorted plan order. The two orders differ inside a
/// step, so each call site uses the one its algorithm needs and they are not
/// interchangeable.
///
/// The stop position takes effect only when its step is visible. The stop test
/// lives inside the group loop, and an invisible step is skipped wholesale
/// before that loop runs, so a `stop_step_idx` naming a step that is invisible
/// under the flags accumulated so far never stops the replay: it runs to the end
/// of the installer and the returned map holds flags set by later steps. Both
/// callers (local search and the backtracker) then use that map to decide the
/// visibility of that very step. Moving the stop test out of the group loop
/// changes which flags those callers see, and with them the selections the
/// solver returns; it is a behavior change, not a cleanup. See
/// `PARITY-NOTES.md`.
fn rebuild_flags(
    installer: &FomodInstaller,
    selections: &[Vec<Vec<bool>>],
    overrides: Option<&InferenceOverrides>,
    stop_step_idx: i32,
    stop_group_idx: i32,
) -> HashMap<String, String> {
    let mut flags: HashMap<String, String> = HashMap::new();

    for (si, step) in installer.steps.iter().enumerate() {
        let mut visible = true;
        if let Some(vis) = &step.visible {
            let mode = overrides
                .and_then(|o| o.step_visible.get(si).copied())
                .unwrap_or(ExternalConditionOverride::Unknown);
            visible = evaluate_condition_inferred(vis, &flags, mode, None);
        }
        if !visible {
            continue;
        }

        for (gi, group) in step.groups.iter().enumerate() {
            if stop_step_idx >= 0 && si as i32 == stop_step_idx && gi as i32 >= stop_group_idx {
                return flags;
            }

            for (pi, plugin) in group.plugins.iter().enumerate() {
                let mut selected = selections
                    .get(si)
                    .and_then(|g| g.get(gi))
                    .and_then(|p| p.get(pi))
                    .copied()
                    .unwrap_or(false);

                if evaluate_plugin_type(plugin, &flags, None) == PluginType::Required {
                    selected = true;
                }

                if selected {
                    for (fn_name, fv) in &plugin.condition_flags {
                        flags.insert(fn_name.clone(), fv.clone());
                    }
                }
            }
        }
    }

    flags
}

/// Is this step visible under the current flags and any external visibility
/// override? A step with no `visible` condition is always visible.
fn step_visible_with_flags(
    step: &FomodStep,
    step_idx: usize,
    flags: &HashMap<String, String>,
    overrides: Option<&InferenceOverrides>,
) -> bool {
    let Some(vis) = &step.visible else {
        return true;
    };
    let mode = overrides
        .and_then(|o| o.step_visible.get(step_idx).copied())
        .unwrap_or(ExternalConditionOverride::Unknown);
    evaluate_condition_inferred(vis, flags, mode, None)
}

/// Human-readable node count: `1234` -> `1k`, `2_500_000` -> `2.5M`. The `k`
/// tier prints no decimals; the `M` and `G` tiers print one.
fn format_count(n: i64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}G", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.0}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

/// Elapsed seconds as `00:SS` under a minute, `MM:SS` under an hour, and
/// `H:MM:SS` above it.
fn format_duration(s: i64) -> String {
    if s < 60 {
        format!("00:{s:02}")
    } else if s < 3600 {
        format!("{:02}:{:02}", s / 60, s % 60)
    } else {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    }
}

/// A SelectAny cap for display. A non-positive cap means uncapped and prints as
/// `full`.
fn format_option_cap(select_any_cap: i32) -> String {
    if select_any_cap <= 0 {
        "full".to_string()
    } else {
        select_any_cap.to_string()
    }
}

/// Render a tqdm-style progress bar, 20 cells wide.
///
/// The `>` head overwrites the cell after the filled run, so a bar at 0% reads
/// `>...................` and a full bar carries no head at all.
fn build_tqdm_bar(current: i64, total: i64, elapsed_s: i64) -> String {
    const WIDTH: usize = 20;
    let total = if total <= 0 { 1 } else { total };
    let ratio = (current as f64 / total as f64).min(1.0);
    let filled = (ratio * WIDTH as f64) as usize;
    let pct = (ratio * 100.0) as i64;

    let mut bar = vec![b'.'; WIDTH];
    for cell in bar.iter_mut().take(filled.min(WIDTH)) {
        *cell = b'=';
    }
    if filled < WIDTH {
        bar[filled] = b'>';
    }
    let bar = String::from_utf8_lossy(&bar);

    let timing = if elapsed_s > 0 && current > 0 {
        let rate = current as f64 / elapsed_s as f64;
        let remaining_s = if ratio < 1.0 {
            ((total - current) as f64 / rate) as i64
        } else {
            0
        };
        format!(
            " [{}<{}, {}/s]",
            format_duration(elapsed_s),
            format_duration(remaining_s),
            format_count(rate as i64)
        )
    } else {
        format!(" [{}<?, ?/s]", format_duration(elapsed_s))
    };

    format!(
        "{pct:>3}%|{bar}| {}/{} nodes{timing}",
        format_count(current),
        format_count(total)
    )
}

/// Emit one `After <phase>: exact=..., missing=..., extra=..., size_mm=...,
/// hash_mm=...` line. Five call sites share it: greedy, local search, targeted
/// repair, component solve and focused search. Residual repair (phase 3) emits
/// no such line, so the log narrative skips it.
///
/// `exact` is the run-wide `search.found_exact` flag; the four counters come
/// from `best.best`.
fn log_phase_metrics(state: &SolverState, phase: &str) {
    Logger::instance().log(&format!(
        "[solver] After {phase}: exact={}, missing={}, extra={}, size_mm={}, hash_mm={}",
        state.search.found_exact,
        state.best.best.missing,
        state.best.best.extra,
        state.best.best.size_mismatch,
        state.best.best.hash_mismatch
    ));
}

/// Render `affected` group indices as a `"; "`-joined list of group names.
fn join_group_names(pre: &Precompute<'_>, affected: &[i32]) -> String {
    affected
        .iter()
        .map(|&g| group_name(pre, &pre.groups[g as usize]))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Simulate the current selections, score them against the target tree, and
/// update the best-solution state on a strict improvement.
///
/// The only site that increments the node count. An equal metric tuple does not
/// replace the best, because
/// [`ReproMetrics::better_than`](crate::fomod_csp_types::ReproMetrics::better_than)
/// rejects it, so the first candidate found at a given tuple wins. Sets
/// `search.found_exact` as soon as a candidate reproduces the target exactly.
fn evaluate_candidate(
    state: &mut SolverState,
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    target: &TargetTree,
    excluded: &HashSet<String>,
    overrides: Option<&InferenceOverrides>,
) -> ReproMetrics {
    let sim = simulate(installer, atoms, &state.search.selections, None, overrides);
    let metrics = compare_trees(&sim, target, excluded);

    state.search.nodes_explored += 1;

    // Periodic progress logging, from every phase (greedy, local search,
    // backtrack). The `estimated_total > 1` guard keeps a pass that never sets
    // an estimate from reading the clock once per node.
    if state.progress.estimated_total > 1 {
        let now = Instant::now();
        let elapsed_ms = now
            .duration_since(state.progress.last_progress_time)
            .as_millis() as i64;
        let nodes_since = (state.search.nodes_explored - state.progress.last_progress_nodes) as i64;
        if nodes_since >= SolverProgress::PROGRESS_NODE_INTERVAL as i64
            || (elapsed_ms >= SolverProgress::PROGRESS_TIME_INTERVAL_MS as i64
                && state.search.nodes_explored > state.progress.last_progress_nodes)
        {
            state.progress.last_progress_nodes = state.search.nodes_explored;
            state.progress.last_progress_time = now;
            let pass_nodes = state.search.nodes_explored as i64 - state.progress.pass_start_nodes;
            let pass_elapsed_s =
                now.duration_since(state.progress.pass_start_time).as_secs() as i64;
            let bar = build_tqdm_bar(pass_nodes, state.progress.estimated_total, pass_elapsed_s);
            if state.best.has_best {
                Logger::instance().log(&format!(
                    "[solver] {bar} | best: m={} e={}",
                    state.best.best_metrics.missing, state.best.best_metrics.extra
                ));
            } else {
                Logger::instance().log(&format!("[solver] {bar} | no solution yet"));
            }
        }
    }

    if !state.best.has_best || metrics.better_than(&state.best.best_metrics) {
        state.best.best_metrics = metrics;
        state.best.best.selections = state.search.selections.clone();
        state.best.best.inferred_flags =
            rebuild_flags(installer, &state.search.selections, overrides, -1, -1);
        state.best.best.exact_match = metrics.exact();
        state.best.best.nodes_explored = state.search.nodes_explored;
        state.best.best.missing = metrics.missing;
        state.best.best.extra = metrics.extra;
        state.best.best.size_mismatch = metrics.size_mismatch;
        state.best.best.hash_mismatch = metrics.hash_mismatch;
        state.best.has_best = true;
        if metrics.exact() {
            state.search.found_exact = true;
        }
    }

    metrics
}

/// Find every group whose selection could influence the given mismatched dests.
///
/// Seeds from the direct producers plus, for conditional dests, every group that
/// sets a needed flag, then expands through the flag dependency chain (BFS). The
/// returned group indices are sorted, so the hash-set iteration order during the
/// expansion cannot leak into the result.
fn groups_for_mismatches(pre: &Precompute, mismatched: &[String]) -> Vec<i32> {
    let mut groups: HashSet<i32> = HashSet::new();
    let mut queue: Vec<i32> = Vec::new();

    for dest in mismatched {
        if let Some(v) = pre.dest_to_groups.get(dest) {
            for &gidx in v {
                if groups.insert(gidx) {
                    queue.push(gidx);
                }
            }
        }

        if pre.conditional_dests.contains(dest) {
            for fn_name in &pre.needed_flags {
                let Some(sit) = pre.flag_to_setter_groups.get(fn_name) else {
                    continue;
                };
                for &gidx in sit {
                    if groups.insert(gidx) {
                        queue.push(gidx);
                    }
                }
            }
        }
    }

    let mut q = 0;
    while q < queue.len() {
        let gidx = queue[q];
        q += 1;
        if gidx < 0 || gidx as usize >= pre.group_reads_flags.len() {
            continue;
        }
        for fn_name in &pre.group_reads_flags[gidx as usize] {
            let Some(sit) = pre.flag_to_setter_groups.get(fn_name) else {
                continue;
            };
            for &setter in sit {
                if groups.insert(setter) {
                    queue.push(setter);
                }
            }
        }
    }

    let mut out: Vec<i32> = groups.into_iter().collect();
    out.sort_unstable();
    out
}

/// Number of selected plugins in an option.
fn selected_count(option: &[bool]) -> i32 {
    option.iter().filter(|&&b| b).count() as i32
}

/// Build the per-group toggle-bit set for the targeted repair pass.
///
/// A repair group contributes the local plugins that produce a mismatched dest,
/// plus whatever it currently has selected in the best solution. Only SelectAny
/// and SelectAtLeastOne groups with more than one plugin qualify. Candidates
/// sort by evidence ascending, then by local index; that second key keeps the
/// order total, so equal-evidence plugins cannot swap between runs. The list is
/// capped at 11 bits, and a group is kept only when at least 2 bits survive.
fn build_repair_plugin_map(
    state: &SolverState,
    pre: &Precompute,
    repair_groups: &[i32],
    mismatched: &[String],
) -> HashMap<i32, Vec<i32>> {
    let repair_group_set: HashSet<i32> = repair_groups.iter().copied().collect();
    let mut per_group: HashMap<i32, HashSet<i32>> = HashMap::new();

    for dest in mismatched {
        let Some(v) = pre.dest_to_plugins.get(dest) else {
            continue;
        };
        for &flat_plugin in v {
            if flat_plugin < 0 || flat_plugin as usize >= pre.plugin_to_group.len() {
                continue;
            }
            let gidx = pre.plugin_to_group[flat_plugin as usize];
            if !repair_group_set.contains(&gidx) {
                continue;
            }
            let gref = pre.groups[gidx as usize];
            let local_pi = flat_plugin - gref.flat_start;
            if local_pi < 0 || local_pi >= gref.plugin_count {
                continue;
            }
            per_group.entry(gidx).or_default().insert(local_pi);
        }
    }

    for &gidx in repair_groups {
        let gref = pre.groups[gidx as usize];
        if gref.step_idx as usize >= state.best.best.selections.len()
            || gref.group_idx as usize >= state.best.best.selections[gref.step_idx as usize].len()
        {
            continue;
        }
        let selected = &state.best.best.selections[gref.step_idx as usize][gref.group_idx as usize];
        for (pi, &sel) in selected.iter().enumerate() {
            if sel {
                per_group.entry(gidx).or_default().insert(pi as i32);
            }
        }
    }

    const MAX_REPAIR_BITS: usize = 11;
    let mut out: HashMap<i32, Vec<i32>> = HashMap::new();
    for &gidx in repair_groups {
        let Some(locals_set) = per_group.get(&gidx) else {
            continue;
        };
        let gref = pre.groups[gidx as usize];
        let group = &pre.installer.steps[gref.step_idx as usize].groups[gref.group_idx as usize];
        if group.r#type != FomodGroupType::SelectAny
            && group.r#type != FomodGroupType::SelectAtLeastOne
        {
            continue;
        }
        if gref.plugin_count <= 1 {
            continue;
        }

        let mut locals: Vec<i32> = locals_set.iter().copied().collect();
        locals.sort_by(|&a, &b| {
            pre.evidence[(gref.flat_start + a) as usize]
                .cmp(&pre.evidence[(gref.flat_start + b) as usize])
                .then(a.cmp(&b))
        });
        if locals.len() > MAX_REPAIR_BITS {
            locals.truncate(MAX_REPAIR_BITS);
        }
        if locals.len() >= 2 {
            out.insert(gidx, locals);
        }
    }

    out
}

/// Exhaustive bit-flip repair over a small plugin neighborhood.
///
/// For each repair group (SelectAny and SelectAtLeastOne only) enumerates all
/// `2^k` on/off combinations of its toggle bits, `k` capped at 11, over at most
/// 2 passes, keeping strict improvements. Every combination is scored through
/// [`evaluate_candidate`]. Does nothing when there is no best solution, no
/// repair group or no mismatch, and stops as soon as an exact match appears.
fn targeted_repair_search(
    state: &mut SolverState,
    pre: &Precompute,
    repair_groups: &[i32],
    mismatched: &[String],
) {
    if repair_groups.is_empty()
        || mismatched.is_empty()
        || !state.best.has_best
        || state.search.found_exact
    {
        return;
    }

    let plugin_map = build_repair_plugin_map(state, pre, repair_groups, mismatched);
    if plugin_map.is_empty() {
        return;
    }

    Logger::instance().log(&format!(
        "[solver] Targeted repair neighborhood: {} groups",
        plugin_map.len()
    ));

    state.search.selections = state.best.best.selections.clone();

    let mut improved = true;
    let mut pass = 0;
    const MAX_PASSES: i32 = 2;

    while improved && !state.search.found_exact && pass < MAX_PASSES {
        improved = false;
        pass += 1;

        for &gidx in repair_groups {
            if state.search.found_exact {
                return;
            }

            let Some(bits) = plugin_map.get(&gidx) else {
                continue;
            };
            if bits.len() < 2 || bits.len() > 20 {
                continue;
            }

            let gref = pre.groups[gidx as usize];
            let is_at_least_one =
                pre.installer.steps[gref.step_idx as usize].groups[gref.group_idx as usize].r#type
                    == FomodGroupType::SelectAtLeastOne;
            let baseline =
                state.search.selections[gref.step_idx as usize][gref.group_idx as usize].clone();
            let mut best_group = baseline.clone();
            let mut prev_best = state.best.best_metrics;

            let variants: u64 = 1u64 << bits.len();
            for mask in 0..variants {
                if state.search.found_exact {
                    break;
                }

                let mut var = baseline.clone();
                for (bi, &local_pi) in bits.iter().enumerate() {
                    if local_pi < 0 || local_pi as usize >= var.len() {
                        continue;
                    }
                    var[local_pi as usize] = (mask & (1u64 << bi)) != 0;
                }

                if is_at_least_one && selected_count(&var) == 0 {
                    continue;
                }

                state.search.selections[gref.step_idx as usize][gref.group_idx as usize] =
                    var.clone();
                evaluate_candidate(
                    state,
                    pre.installer,
                    pre.atoms,
                    pre.target,
                    pre.excluded,
                    pre.overrides,
                );

                if state.best.best_metrics.better_than(&prev_best) {
                    improved = true;
                    prev_best = state.best.best_metrics;
                    best_group = var;
                    if state.search.found_exact {
                        return;
                    }
                }
            }

            state.search.selections[gref.step_idx as usize][gref.group_idx as usize] = best_group;
        }
    }

    if state.best.has_best {
        state.search.selections = state.best.best.selections.clone();
    }
}

/// Is any producer group for `dest` still unassigned (`order_pos >= next_idx`)?
fn has_remaining_group(
    map: &HashMap<String, Vec<i32>>,
    dest: &str,
    order_pos: &[i32],
    next_idx: i32,
) -> bool {
    let Some(v) = map.get(dest) else {
        return false;
    };
    for &g in v {
        if g >= 0 && (g as usize) < order_pos.len() && order_pos[g as usize] >= next_idx {
            return true;
        }
    }
    false
}

/// A conditional-only dest stays repairable while any group that sets a needed
/// flag remains unassigned.
fn conditional_repair_remaining(
    pre: &Precompute,
    dest: &str,
    order_pos: &[i32],
    next_idx: i32,
) -> bool {
    if !pre.conditional_dests.contains(dest) {
        return false;
    }
    for fn_name in &pre.needed_flags {
        let Some(v) = pre.flag_to_setter_groups.get(fn_name) else {
            continue;
        };
        for &g in v {
            if g >= 0 && (g as usize) < order_pos.len() && order_pos[g as usize] >= next_idx {
                return true;
            }
        }
    }
    false
}

/// Admissible lower bound: simulate the current selections and count only the
/// mismatches that no still-unassigned group can fix. Feeds [`cannot_beat`].
fn lower_bound(
    state: &SolverState,
    pre: &Precompute,
    plan: &SearchPlan,
    next_idx: i32,
) -> ReproMetrics {
    let sim = simulate(
        pre.installer,
        pre.atoms,
        &state.search.selections,
        None,
        pre.overrides,
    );

    let can_fix_missing = |dest: &str| {
        !has_remaining_group(&pre.dest_to_groups, dest, &plan.order_pos, next_idx)
            && !conditional_repair_remaining(pre, dest, &plan.order_pos, next_idx)
    };
    let can_fix_size = |dest: &str| {
        !has_remaining_group(
            &pre.dest_to_size_match_groups,
            dest,
            &plan.order_pos,
            next_idx,
        ) && !conditional_repair_remaining(pre, dest, &plan.order_pos, next_idx)
    };
    let can_fix_hash = |dest: &str| {
        !has_remaining_group(
            &pre.dest_to_hash_capable_groups,
            dest,
            &plan.order_pos,
            next_idx,
        ) && !conditional_repair_remaining(pre, dest, &plan.order_pos, next_idx)
    };

    compare_trees_impl(
        &sim,
        pre.target,
        pre.excluded,
        can_fix_missing,
        can_fix_size,
        can_fix_hash,
    )
}

/// Strict lexicographic `>` on `(missing, extra, size_mismatch, hash_mismatch)`:
/// true when the lower bound already loses to the best, which makes the branch
/// safe to prune.
fn cannot_beat(lb: &ReproMetrics, best: &ReproMetrics) -> bool {
    if lb.missing > best.missing {
        return true;
    }
    if lb.missing == best.missing && lb.extra > best.extra {
        return true;
    }
    if lb.missing == best.missing && lb.extra == best.extra && lb.size_mismatch > best.size_mismatch
    {
        return true;
    }
    if lb.missing == best.missing
        && lb.extra == best.extra
        && lb.size_mismatch == best.size_mismatch
        && lb.hash_mismatch > best.hash_mismatch
    {
        return true;
    }
    false
}

/// FNV fold of the selected, already-assigned contested plugins.
///
/// Iterates `pre.contested_plugins` (sorted ascending) and folds
/// `(flat_plugin + 1)` through [`hash_combine`] for each contested plugin whose
/// group is already assigned (`0 <= order_pos < next_idx`) and currently
/// selected. The value is a `MemoKey` equality field: two search states that
/// fold to the same signature are treated as the same subtree, so the seed, the
/// iteration order and the `+ 1` all have to stay as they are.
fn contested_signature(
    state: &SolverState,
    pre: &Precompute,
    plan: &SearchPlan,
    next_idx: i32,
) -> u64 {
    let mut sig: u64 = 14695981039346656037;

    for &flat_plugin in &pre.contested_plugins {
        if flat_plugin < 0 || flat_plugin as usize >= pre.plugin_to_group.len() {
            continue;
        }
        let gidx = pre.plugin_to_group[flat_plugin as usize];
        if gidx < 0 || gidx as usize >= plan.order_pos.len() {
            continue;
        }
        let pos = plan.order_pos[gidx as usize];
        if pos < 0 || pos >= next_idx {
            continue;
        }
        let gref = pre.groups[gidx as usize];
        let local_pi = flat_plugin - gref.flat_start;
        if local_pi < 0 || local_pi >= gref.plugin_count {
            continue;
        }
        let s = gref.step_idx as usize;
        let g = gref.group_idx as usize;
        if s >= state.search.selections.len()
            || g >= state.search.selections[s].len()
            || local_pi as usize >= state.search.selections[s][g].len()
        {
            continue;
        }
        if state.search.selections[s][g][local_pi as usize] {
            hash_combine(&mut sig, (flat_plugin + 1) as u64);
        }
    }

    sig
}

/// Write an option into the selection grid for a group. Plugins past the end of
/// `option` are cleared, so a short option deselects the tail of the group.
fn apply_option(selections: &mut [Vec<Vec<bool>>], gref: &GroupRef, option: &[bool]) {
    let s = gref.step_idx as usize;
    let g = gref.group_idx as usize;
    for pi in 0..gref.plugin_count as usize {
        selections[s][g][pi] = pi < option.len() && option[pi];
    }
}

/// Advance the incremental flag map past one group's plugins, recording undo
/// deltas for [`undo_flags_to`].
///
/// Its callers walk the priority-sorted plan order, unlike [`rebuild_flags`],
/// which walks document order.
fn advance_flags_past_group(
    flags: &mut HashMap<String, String>,
    installer: &FomodInstaller,
    selections: &[Vec<Vec<bool>>],
    gref: &GroupRef,
    undo: &mut Vec<crate::fomod_csp_types::FlagDelta>,
) {
    let group = &installer.steps[gref.step_idx as usize].groups[gref.group_idx as usize];
    for (pi, plugin) in group.plugins.iter().enumerate() {
        let mut selected = (gref.step_idx as usize) < selections.len()
            && (gref.group_idx as usize) < selections[gref.step_idx as usize].len()
            && pi < selections[gref.step_idx as usize][gref.group_idx as usize].len()
            && selections[gref.step_idx as usize][gref.group_idx as usize][pi];
        if evaluate_plugin_type(plugin, flags, None) == PluginType::Required {
            selected = true;
        }
        if selected {
            for (fn_name, fv) in &plugin.condition_flags {
                let (had_value, old_value) = match flags.get(fn_name) {
                    Some(v) => (true, v.clone()),
                    None => (false, String::new()),
                };
                undo.push(crate::fomod_csp_types::FlagDelta {
                    name: fn_name.clone(),
                    had_value,
                    old_value,
                });
                flags.insert(fn_name.clone(), fv.clone());
            }
        }
    }
}

/// Roll the incremental flag map back to a prior undo-stack mark.
fn undo_flags_to(
    flags: &mut HashMap<String, String>,
    undo: &mut Vec<crate::fomod_csp_types::FlagDelta>,
    target_size: usize,
) {
    while undo.len() > target_size {
        let d = undo.pop().unwrap();
        if d.had_value {
            flags.insert(d.name, d.old_value);
        } else {
            flags.remove(&d.name);
        }
    }
}

/// Greedy forward pass: give each group its highest-ranked option, in
/// `pre.groups` order, tracking flags incrementally, then score once at the end.
///
/// "Highest-ranked" is the head of [`get_options_for_group`]'s list, ordered by
/// the option-reduction heuristic: evidence descending, unique descending,
/// useful descending, extra ascending, raw index ascending. That heuristic is
/// not the simulator oracle. Greedy never simulates an individual option and
/// calls [`evaluate_candidate`] exactly once, on the completed grid; reserve the
/// word "score" for that oracle.
///
/// A group whose step is not visible under the flags so far is left untouched,
/// and a group with an empty option list only advances the flag map.
fn greedy_solve(
    state: &mut SolverState,
    pre: &Precompute,
    select_any_cap: i32,
    exact_groups: Option<&HashSet<i32>>,
    cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) {
    state.search.flags.clear();
    let mut flag_undo: Vec<crate::fomod_csp_types::FlagDelta> = Vec::new();
    let mut prev_step = -1i32;
    let mut prev_step_visible = false;

    for gidx in 0..pre.groups.len() as i32 {
        let gref = pre.groups[gidx as usize];

        if gref.step_idx != prev_step {
            prev_step = gref.step_idx;
            let step = &pre.installer.steps[gref.step_idx as usize];
            prev_step_visible = step_visible_with_flags(
                step,
                gref.step_idx as usize,
                &state.search.flags,
                pre.overrides,
            );
        }

        if !prev_step_visible {
            continue;
        }

        let first_option: Option<GroupOption> = {
            let cached = get_options_for_group(
                gidx,
                pre,
                &state.search.flags,
                select_any_cap,
                exact_groups,
                cache,
                stats,
            );
            cached.options.first().cloned()
        };

        match first_option {
            None => {
                advance_flags_past_group(
                    &mut state.search.flags,
                    pre.installer,
                    &state.search.selections,
                    &gref,
                    &mut flag_undo,
                );
            }
            Some(opt) => {
                apply_option(&mut state.search.selections, &gref, &opt);
                advance_flags_past_group(
                    &mut state.search.flags,
                    pre.installer,
                    &state.search.selections,
                    &gref,
                    &mut flag_undo,
                );
            }
        }
    }

    evaluate_candidate(
        state,
        pre.installer,
        pre.atoms,
        pre.target,
        pre.excluded,
        pre.overrides,
    );
}

/// Iterative improvement: re-solve each group in `order`, keeping strictly
/// improving changes, for at most `max_passes` passes. Returns as soon as an
/// exact match appears.
///
/// Flags are rebuilt from document order with [`rebuild_flags`] before each
/// group rather than tracked incrementally, because `order` is arbitrary.
#[allow(clippy::too_many_arguments)]
fn local_search(
    state: &mut SolverState,
    pre: &Precompute,
    order: &[i32],
    max_passes: i32,
    select_any_cap: i32,
    exact_groups: Option<&HashSet<i32>>,
    cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) {
    let mut improved = true;
    let mut pass = 0;

    while improved && !state.search.found_exact && pass < max_passes {
        improved = false;
        pass += 1;

        for &gidx in order {
            if state.search.found_exact {
                return;
            }

            let gref = pre.groups[gidx as usize];

            state.search.flags = rebuild_flags(
                pre.installer,
                &state.search.selections,
                pre.overrides,
                gref.step_idx,
                gref.group_idx,
            );

            {
                let step = &pre.installer.steps[gref.step_idx as usize];
                if !step_visible_with_flags(
                    step,
                    gref.step_idx as usize,
                    &state.search.flags,
                    pre.overrides,
                ) {
                    continue;
                }
            }

            let opts: Vec<GroupOption> = {
                let cached = get_options_for_group(
                    gidx,
                    pre,
                    &state.search.flags,
                    select_any_cap,
                    exact_groups,
                    cache,
                    stats,
                );
                cached.options.clone()
            };
            if opts.is_empty() {
                continue;
            }

            let s = gref.step_idx as usize;
            let g = gref.group_idx as usize;
            let mut best_group = state.search.selections[s][g].clone();
            let mut prev_best = state.best.best_metrics;

            for opt in &opts {
                apply_option(&mut state.search.selections, &gref, opt);
                evaluate_candidate(
                    state,
                    pre.installer,
                    pre.atoms,
                    pre.target,
                    pre.excluded,
                    pre.overrides,
                );

                if state.best.best_metrics.better_than(&prev_best) {
                    improved = true;
                    prev_best = state.best.best_metrics;
                    best_group = state.search.selections[s][g].clone();
                    if state.search.found_exact {
                        return;
                    }
                }
            }

            state.search.selections[s][g] = best_group;
        }
    }
}

/// One level of the explicit backtracker stack.
struct Frame {
    next_idx: i32,
    branch_idx: i32,
    gidx: i32,
    opt_idx: usize,
    saved_group: GroupOption,
    flag_mark: usize,
    skip_flag_mark: usize,
    checkpoint_mark: usize,
}

/// A saved group selection, put back when its frame unwinds.
struct CheckpointEntry {
    step_idx: i32,
    group_idx: i32,
    saved: GroupOption,
}

/// Save a group's current selection as a checkpoint, refusing past
/// `max_checkpoints`.
///
/// Returns false, after logging one `[solver]` warning, when the checkpoint
/// stack is already at `max_checkpoints`; the caller must then abandon the
/// branch. Returns true after pushing on every other path.
fn save_checkpoint(
    checkpoints: &mut Vec<CheckpointEntry>,
    selections: &[Vec<Vec<bool>>],
    gref: &GroupRef,
    max_checkpoints: usize,
) -> bool {
    if checkpoints.len() >= max_checkpoints {
        Logger::instance().log_warning("[solver] Checkpoint limit reached, abandoning branch");
        return false;
    }
    checkpoints.push(CheckpointEntry {
        step_idx: gref.step_idx,
        group_idx: gref.group_idx,
        saved: selections[gref.step_idx as usize][gref.group_idx as usize].clone(),
    });
    true
}

/// Restore checkpointed group selections back to a prior mark.
fn restore_checkpoints_to(
    checkpoints: &mut Vec<CheckpointEntry>,
    selections: &mut [Vec<Vec<bool>>],
    mark: usize,
) {
    while checkpoints.len() > mark {
        let e = checkpoints.pop().unwrap();
        selections[e.step_idx as usize][e.group_idx as usize] = e.saved;
    }
}

/// Unwind one frame: restore its branching group's selection, then roll flags
/// and checkpoints back to the marks that frame recorded.
fn unwind_frame(
    f: &mut Frame,
    pre: &Precompute,
    incremental_flags: bool,
    state: &mut SolverState,
    flag_undo: &mut Vec<crate::fomod_csp_types::FlagDelta>,
    checkpoints: &mut Vec<CheckpointEntry>,
) {
    if f.branch_idx >= 0 {
        let gref = pre.groups[f.gidx as usize];
        state.search.selections[gref.step_idx as usize][gref.group_idx as usize] =
            std::mem::take(&mut f.saved_group);
    }
    if incremental_flags {
        undo_flags_to(&mut state.search.flags, flag_undo, f.flag_mark);
    }
    restore_checkpoints_to(checkpoints, &mut state.search.selections, f.checkpoint_mark);
}

/// Iterative branch-and-bound backtracking over the plan's group order. The
/// stack is explicit rather than recursive, so a deep installer cannot overflow
/// the native stack.
///
/// Each frame has two states, selected by `branch_idx`. A frame starts in the
/// init state (`branch_idx < 0`), walks forward past groups it does not need to
/// branch on, then becomes a branch frame that hands one option at a time to a
/// child frame. Every pop path bumps a different [`SolverStats`] counter, which
/// is what makes the closing pruning-summary log line readable:
///
/// ```text
///   push root { next_idx = start_idx, branch_idx = -1 }
///        |
///   +--> init (branch_idx < 0)
///   |      stack depth > 500 ......... pop  (max_depth_aborts, warn once)
///   |      nodes >= node_limit ....... pop  (pruned_node_limit)
///   |      every 64th node, past the deadline .. pop (sets deadline_exceeded)
///   |      skip run, advancing next_idx past:
///   |        invisible groups (clear the selection, checkpoint it first)
///   |                                       (skipped_invisible)
///   |        single-option groups (apply the option, advance flags)
///   |      checkpoint stack full during the skip run .. pop
///   |      order exhausted ........... evaluate_candidate, then pop
///   |      at the branching index ci, only when a best exists with mismatches,
///   |      ci >= 4, and ci is a multiple of 'bound_stride':
///   |        lower_bound cannot_beat best ... pop (pruned_lower_bound)
///   |        memo hit that is no better ..... pop (pruned_memo)
///   |      otherwise: branch_idx = ci, save the group, opt_idx = 0
///   |
///   +--- branch (branch_idx >= 0)
///          advance opt_idx past extra-only options  (pruned_extra_only)
///          no option left, or found_exact, or deadline, or node limit .. pop
///          else apply_option, advance flags, push child
///                { next_idx = branch_idx + 1, branch_idx = -1 }
///
///   pop == unwind_frame: restore the frame's saved group selection, undo flags
///                        back to its mark, restore checkpoints to its mark
/// ```
///
/// The loop returns when the stack empties, and unwinds every remaining frame at
/// once as soon as `found_exact` or `deadline_exceeded` becomes true.
/// `plan.memo` is cleared wholesale when it reaches `MAX_MEMO_ENTRIES`.
#[allow(clippy::too_many_arguments)]
fn backtrack(
    state: &mut SolverState,
    pre: &Precompute,
    plan: &mut SearchPlan,
    start_idx: i32,
    select_any_cap: i32,
    exact_groups: Option<&HashSet<i32>>,
    cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) {
    let mut checkpoints: Vec<CheckpointEntry> = Vec::new();
    let mut flag_undo: Vec<crate::fomod_csp_types::FlagDelta> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();

    stack.push(Frame {
        next_idx: start_idx,
        branch_idx: -1,
        gidx: 0,
        opt_idx: 0,
        saved_group: Vec::new(),
        flag_mark: flag_undo.len(),
        skip_flag_mark: flag_undo.len(),
        checkpoint_mark: checkpoints.len(),
    });

    while !stack.is_empty() {
        if state.search.found_exact || state.progress.deadline_exceeded {
            while let Some(mut f) = stack.pop() {
                unwind_frame(
                    &mut f,
                    pre,
                    plan.incremental_flags,
                    state,
                    &mut flag_undo,
                    &mut checkpoints,
                );
            }
            return;
        }

        let top = stack.len() - 1;

        // ------------------------------------------------------------------
        // Init state: skip single-option groups, then run the bounds.
        // ------------------------------------------------------------------
        if stack[top].branch_idx < 0 {
            if stack.len() > MAX_BACKTRACK_DEPTH {
                // Warn on the first abort only, before the counter moves.
                if stats.max_depth_aborts == 0 {
                    Logger::instance().log_warning(&format!(
                        "[solver] kMaxBacktrackDepth ({MAX_BACKTRACK_DEPTH}) exceeded; abandoning branch (logged once per solve)"
                    ));
                }
                stats.max_depth_aborts += 1;
                let mut f = stack.pop().unwrap();
                unwind_frame(
                    &mut f,
                    pre,
                    plan.incremental_flags,
                    state,
                    &mut flag_undo,
                    &mut checkpoints,
                );
                continue;
            }
            if plan.node_limit > 0 && state.search.nodes_explored >= plan.node_limit {
                stats.pruned_node_limit += 1;
                let mut f = stack.pop().unwrap();
                unwind_frame(
                    &mut f,
                    pre,
                    plan.incremental_flags,
                    state,
                    &mut flag_undo,
                    &mut checkpoints,
                );
                continue;
            }
            if let Some(deadline) = state.progress.deadline {
                if (state.search.nodes_explored & 63) == 0 && Instant::now() >= deadline {
                    state.progress.deadline_exceeded = true;
                    let mut f = stack.pop().unwrap();
                    unwind_frame(
                        &mut f,
                        pre,
                        plan.incremental_flags,
                        state,
                        &mut flag_undo,
                        &mut checkpoints,
                    );
                    continue;
                }
            }

            stack[top].flag_mark = flag_undo.len();
            stack[top].checkpoint_mark = checkpoints.len();

            // Skip through single-option and invisible groups.
            let mut ci = stack[top].next_idx;
            let mut bail = false;
            loop {
                if ci >= plan.order.len() as i32 {
                    break;
                }
                let gidx = plan.order[ci as usize];
                let gref = pre.groups[gidx as usize];

                if !plan.incremental_flags {
                    state.search.flags = rebuild_flags(
                        pre.installer,
                        &state.search.selections,
                        pre.overrides,
                        gref.step_idx,
                        gref.group_idx,
                    );
                }

                let visible = {
                    let step = &pre.installer.steps[gref.step_idx as usize];
                    step_visible_with_flags(
                        step,
                        gref.step_idx as usize,
                        &state.search.flags,
                        pre.overrides,
                    )
                };

                if !visible {
                    let has_true = state.search.selections[gref.step_idx as usize]
                        [gref.group_idx as usize]
                        .iter()
                        .any(|&b| b);
                    if has_true {
                        if !save_checkpoint(
                            &mut checkpoints,
                            &state.search.selections,
                            &gref,
                            CONFIG.max_checkpoints,
                        ) {
                            bail = true;
                            break;
                        }
                        if state.progress.deadline_exceeded {
                            bail = true;
                            break;
                        }
                        for b in state.search.selections[gref.step_idx as usize]
                            [gref.group_idx as usize]
                            .iter_mut()
                        {
                            *b = false;
                        }
                    }
                    stats.skipped_invisible += 1;
                    ci += 1;
                    continue;
                }

                let (opt_len, first_opt) = {
                    let cached = get_options_for_group(
                        gidx,
                        pre,
                        &state.search.flags,
                        select_any_cap,
                        exact_groups,
                        cache,
                        stats,
                    );
                    (cached.options.len(), cached.options.first().cloned())
                };

                if opt_len == 1 {
                    let first = first_opt.unwrap();
                    let differs = state.search.selections[gref.step_idx as usize]
                        [gref.group_idx as usize]
                        != first;
                    if differs {
                        if !save_checkpoint(
                            &mut checkpoints,
                            &state.search.selections,
                            &gref,
                            CONFIG.max_checkpoints,
                        ) {
                            bail = true;
                            break;
                        }
                        if state.progress.deadline_exceeded {
                            bail = true;
                            break;
                        }
                        state.search.selections[gref.step_idx as usize][gref.group_idx as usize] =
                            first;
                    }
                    if plan.incremental_flags {
                        advance_flags_past_group(
                            &mut state.search.flags,
                            pre.installer,
                            &state.search.selections,
                            &gref,
                            &mut flag_undo,
                        );
                    }
                    ci += 1;
                    continue;
                }

                break; // multi-option group found
            }

            if bail {
                let mut f = stack.pop().unwrap();
                unwind_frame(
                    &mut f,
                    pre,
                    plan.incremental_flags,
                    state,
                    &mut flag_undo,
                    &mut checkpoints,
                );
                continue;
            }

            if ci >= plan.order.len() as i32 {
                evaluate_candidate(
                    state,
                    pre.installer,
                    pre.atoms,
                    pre.target,
                    pre.excluded,
                    pre.overrides,
                );
                let mut f = stack.pop().unwrap();
                unwind_frame(
                    &mut f,
                    pre,
                    plan.incremental_flags,
                    state,
                    &mut flag_undo,
                    &mut checkpoints,
                );
                continue;
            }

            // Bounds checking and memoization for the branching group.
            let gidx = plan.order[ci as usize];
            let gref = pre.groups[gidx as usize];
            if !plan.incremental_flags {
                state.search.flags = rebuild_flags(
                    pre.installer,
                    &state.search.selections,
                    pre.overrides,
                    gref.step_idx,
                    gref.group_idx,
                );
            }

            let bm = state.best.best_metrics;
            let mismatch_pressure = bm.missing + bm.extra + bm.size_mismatch + bm.hash_mismatch;
            let enable_bounds = state.best.has_best && mismatch_pressure > 0;
            let bound_stride = if mismatch_pressure > 1500 {
                64
            } else if mismatch_pressure > 800 {
                48
            } else if mismatch_pressure > 400 {
                24
            } else if mismatch_pressure > 150 {
                12
            } else if mismatch_pressure > 60 {
                6
            } else if mismatch_pressure > 20 {
                3
            } else if mismatch_pressure > 5 {
                2
            } else {
                4
            };

            let run_bounds_here = enable_bounds && ci >= 4 && (ci % bound_stride == 0);
            if run_bounds_here {
                let lb = lower_bound(state, pre, plan, ci);
                if cannot_beat(&lb, &state.best.best_metrics) {
                    stats.pruned_lower_bound += 1;
                    let mut f = stack.pop().unwrap();
                    unwind_frame(
                        &mut f,
                        pre,
                        plan.incremental_flags,
                        state,
                        &mut flag_undo,
                        &mut checkpoints,
                    );
                    continue;
                }

                let bm2 = state.best.best_metrics;
                let enable_memo = bm2.missing <= 24
                    && bm2.extra <= 24
                    && (bm2.size_mismatch + bm2.hash_mismatch) <= 24;
                if enable_memo {
                    let mk = MemoKey {
                        next_idx: ci,
                        flag_state_sig: hash_flag_subset(&state.search.flags, &pre.memo_flags),
                        contested_sig: contested_signature(state, pre, plan, ci),
                    };
                    let stored = plan.memo.get(&mk).copied();
                    if let Some(s) = stored {
                        if !lb.better_than(&s) {
                            stats.pruned_memo += 1;
                            let mut f = stack.pop().unwrap();
                            unwind_frame(
                                &mut f,
                                pre,
                                plan.incremental_flags,
                                state,
                                &mut flag_undo,
                                &mut checkpoints,
                            );
                            continue;
                        }
                    }
                    let should_store = match stored {
                        None => true,
                        Some(s) => lb.better_than(&s),
                    };
                    if should_store {
                        if plan.memo.len() >= MAX_MEMO_ENTRIES {
                            plan.memo.clear();
                        }
                        plan.memo.insert(mk, lb);
                    }
                }
            }

            // Prepare for branching.
            stack[top].branch_idx = ci;
            stack[top].gidx = gidx;
            stack[top].saved_group =
                state.search.selections[gref.step_idx as usize][gref.group_idx as usize].clone();
            stack[top].skip_flag_mark = flag_undo.len();
            stack[top].opt_idx = 0;
        }

        // ------------------------------------------------------------------
        // Branch state: try the next option for the branching group.
        // ------------------------------------------------------------------
        let gidx = stack[top].gidx;
        let gref = pre.groups[gidx as usize];

        let (opts, profile_flags): (Vec<GroupOption>, Vec<(i32, i32, bool)>) = {
            let cached = get_options_for_group(
                gidx,
                pre,
                &state.search.flags,
                select_any_cap,
                exact_groups,
                cache,
                stats,
            );
            let opts = cached.options.clone();
            let pf: Vec<(i32, i32, bool)> = cached
                .profiles
                .iter()
                .map(|p| (p.extra_dests, p.useful_dests, p.sets_needed_flag))
                .collect();
            (opts, pf)
        };

        let opt_idx0 = stack[top].opt_idx;
        if plan.incremental_flags && opt_idx0 > 0 {
            undo_flags_to(
                &mut state.search.flags,
                &mut flag_undo,
                stack[top].skip_flag_mark,
            );
        }

        let exact_mode = is_exact_group_mode(gidx, exact_groups);
        let mut opt_idx = opt_idx0;
        let mut found_option = false;
        while opt_idx < opts.len() {
            if state.search.found_exact || state.progress.deadline_exceeded {
                break;
            }
            if plan.node_limit > 0 && state.search.nodes_explored >= plan.node_limit {
                stats.pruned_node_limit += 1;
                break;
            }

            let (extra_dests, useful_dests, sets_needed_flag) = profile_flags[opt_idx];
            if !exact_mode && extra_dests > 0 && useful_dests == 0 && !sets_needed_flag {
                stats.pruned_extra_only += 1;
                opt_idx += 1;
                continue;
            }

            found_option = true;
            break;
        }

        if !found_option {
            if plan.incremental_flags {
                undo_flags_to(
                    &mut state.search.flags,
                    &mut flag_undo,
                    stack[top].skip_flag_mark,
                );
            }
            let mut f = stack.pop().unwrap();
            unwind_frame(
                &mut f,
                pre,
                plan.incremental_flags,
                state,
                &mut flag_undo,
                &mut checkpoints,
            );
            continue;
        }

        apply_option(&mut state.search.selections, &gref, &opts[opt_idx]);
        stack[top].opt_idx = opt_idx + 1;

        if plan.incremental_flags {
            advance_flags_past_group(
                &mut state.search.flags,
                pre.installer,
                &state.search.selections,
                &gref,
                &mut flag_undo,
            );
        }

        let branch_idx = stack[top].branch_idx;
        stack.push(Frame {
            next_idx: branch_idx + 1,
            branch_idx: -1,
            gidx: 0,
            opt_idx: 0,
            saved_group: Vec::new(),
            flag_mark: flag_undo.len(),
            skip_flag_mark: flag_undo.len(),
            checkpoint_mark: checkpoints.len(),
        });
    }
}

/// Estimate the number of candidate combinations for `order` as the product of
/// each group's option count. Returns the sentinel `limit + 1` as soon as the
/// running product would exceed `limit`.
///
/// `limit + 1` is not a count. Callers log it as `space=` and feed it to the
/// progress bar as a denominator, so a saturated estimate shows as a
/// suspiciously round figure one above the cap. A group with zero options counts
/// as 1, so it never zeroes the product.
///
/// Side effect: primes the option cache, so a later backtrack over the same
/// groups hits the cache and does not re-count domain-reduction stats.
#[allow(clippy::too_many_arguments)]
fn estimate_search_space(
    pre: &Precompute,
    order: &[i32],
    flags: &HashMap<String, String>,
    select_any_cap: i32,
    exact_groups: Option<&HashSet<i32>>,
    cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
    limit: u64,
) -> u64 {
    let mut total: u64 = 1;
    for &gidx in order {
        let c = {
            let cached =
                get_options_for_group(gidx, pre, flags, select_any_cap, exact_groups, cache, stats);
            (cached.options.len() as u64).max(1)
        };
        if total > limit / c {
            return limit + 1;
        }
        total *= c;
    }
    total
}

/// Run one systematic backtracking pass over `order` with branch-and-bound
/// pruning, stopping once the solve-wide node count reaches `node_limit`
/// (0 = unlimited).
///
/// `node_limit` is a cumulative ceiling, not a per-pass allowance:
/// `SolverSearchState::nodes_explored` counts every candidate evaluated since
/// the solve began and is never reset between phases. Every earlier phase spends
/// the same budget, so a late pass whose limit is already reached explores no
/// nodes and returns at once.
///
/// Returns at once when `order` is empty or an exact match already exists.
/// Incremental flag tracking is enabled only when `order` has the same length as
/// the full group list; a shorter order rebuilds the flag map per group instead.
/// The progress estimate is cleared on exit so it cannot leak into the next
/// phase.
#[allow(clippy::too_many_arguments)]
fn run_backtrack_pass(
    state: &mut SolverState,
    pre: &Precompute,
    order: &[i32],
    node_limit: i32,
    label: &str,
    select_any_cap: i32,
    exact_groups: Option<&HashSet<i32>>,
    cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) {
    if order.is_empty() || state.search.found_exact {
        return;
    }

    let mut plan = SearchPlan {
        order: order.to_vec(),
        order_pos: vec![-1; pre.groups.len()],
        node_limit,
        memo: HashMap::new(),
        incremental_flags: false,
    };
    for (i, &g) in order.iter().enumerate() {
        plan.order_pos[g as usize] = i as i32;
    }

    plan.incremental_flags = order.len() == pre.groups.len();
    if plan.incremental_flags && !order.is_empty() {
        let first_gref = pre.groups[order[0] as usize];
        state.search.flags = rebuild_flags(
            pre.installer,
            &state.search.selections,
            pre.overrides,
            first_gref.step_idx,
            first_gref.group_idx,
        );
    }

    // Priming the option cache here is load bearing: it keeps the backtrack
    // below from re-counting domain-reduction stats for the same groups. The
    // returned estimate also drives the progress bar's denominator.
    let space = estimate_search_space(
        pre,
        order,
        &state.search.flags,
        select_any_cap,
        exact_groups,
        cache,
        stats,
        CONFIG.greedy_space_cap,
    );

    state.progress.estimated_total = if node_limit > 0 {
        node_limit as i64
    } else {
        space as i64
    };
    state.progress.last_progress_nodes = state.search.nodes_explored;
    state.progress.pass_start_nodes = state.search.nodes_explored as i64;
    state.progress.pass_start_time = Instant::now();
    state.progress.last_progress_time = state.progress.pass_start_time;

    let cap = format_option_cap(select_any_cap);
    if node_limit == 0 {
        Logger::instance().log(&format!(
            "[solver] Phase: backtrack {label} ({} groups, space={}, select_any_cap={cap})",
            order.len(),
            format_count(space as i64)
        ));
    } else {
        Logger::instance().log(&format!(
            "[solver] Phase: backtrack {label} ({} groups, limit={}, select_any_cap={cap})",
            order.len(),
            format_count(node_limit as i64)
        ));
    }

    // A 0% bar up front, so even a fast solve shows a visible start.
    if state.progress.estimated_total > 1 {
        let bar = build_tqdm_bar(0, state.progress.estimated_total, 0);
        Logger::instance().log(&format!("[solver] {bar} | searching..."));
    }

    backtrack(
        state,
        pre,
        &mut plan,
        0,
        select_any_cap,
        exact_groups,
        cache,
        stats,
    );

    // A closing 100% bar whose denominator is the nodes actually explored rather
    // than the estimate, so a pass always ends at exactly 100%.
    let pass_nodes = state.search.nodes_explored as i64 - state.progress.pass_start_nodes;
    if pass_nodes > 0
        && state.progress.estimated_total > 1
        && state.search.nodes_explored > state.progress.last_progress_nodes
    {
        let pass_elapsed_s = state.progress.pass_start_time.elapsed().as_secs() as i64;
        let bar = build_tqdm_bar(pass_nodes, pass_nodes, pass_elapsed_s);
        if state.best.has_best {
            Logger::instance().log(&format!(
                "[solver] {bar} | best: m={} e={} (done)",
                state.best.best_metrics.missing, state.best.best_metrics.extra
            ));
        } else {
            Logger::instance().log(&format!("[solver] {bar} | no solution (done)"));
        }
    }

    // Clear the estimate so it cannot leak into the next phase.
    state.progress.estimated_total = 0;
}

/// Phase 1: greedy solve, iterative local search, then a first targeted repair
/// over the groups the remaining mismatches touch.
fn run_initial_phases(
    state: &mut SolverState,
    pre: &Precompute,
    select_any_cap: i32,
    options_cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) {
    Logger::instance().log(&format!(
        "[solver] Phase: greedy ({} groups)",
        pre.groups.len()
    ));
    greedy_solve(state, pre, select_any_cap, None, options_cache, stats);
    log_phase_metrics(state, "greedy");
    if state.search.found_exact {
        return;
    }

    let all_groups: Vec<i32> = (0..pre.groups.len() as i32).collect();
    Logger::instance().log(&format!(
        "[solver] Phase: local search ({} groups)",
        all_groups.len()
    ));
    local_search(
        state,
        pre,
        &all_groups,
        5,
        select_any_cap,
        None,
        options_cache,
        stats,
    );
    log_phase_metrics(state, "local search");

    if state.search.found_exact || !state.best.has_best {
        return;
    }

    let sim_best = simulate(
        pre.installer,
        pre.atoms,
        &state.best.best.selections,
        None,
        pre.overrides,
    );
    let mismatched = collect_mismatched_dests(&sim_best, pre.target, pre.excluded);
    let affected = groups_for_mismatches(pre, &mismatched);
    let affected_groups = join_group_names(pre, &affected);
    Logger::instance().log(&format!(
        "[solver] Remaining mismatches: {} dests, {} affected groups",
        mismatched.len(),
        affected.len()
    ));
    if !affected_groups.is_empty() {
        Logger::instance().log(&format!(
            "[solver] Mismatch-affecting groups: {affected_groups}"
        ));
    }

    if !state.search.found_exact {
        targeted_repair_search(state, pre, &affected, &mismatched);
        log_phase_metrics(state, "targeted repair");
    }
}

/// Phase 2: solve each independent component on its own, with a local search
/// followed by a backtrack pass. Returns without doing anything when the
/// installer decomposes into 1 component or fewer.
fn run_component_decomposition(
    state: &mut SolverState,
    pre: &Precompute,
    select_any_cap: i32,
    options_cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) {
    if pre.components.len() <= 1 {
        return;
    }

    Logger::instance().log(&format!(
        "[solver] Component decomposition: {} components",
        pre.components.len()
    ));

    // A 1-based counter over every component, empty ones included, so the number
    // in the log line is the component's position in `pre.components`.
    let mut comp_idx = 0;
    for comp in &pre.components {
        if state.search.found_exact || state.progress.deadline_exceeded {
            break;
        }
        if comp.is_empty() {
            comp_idx += 1;
            continue;
        }

        comp_idx += 1;
        Logger::instance().log(&format!(
            "[solver] Phase: backtrack component {comp_idx}/{} ({} groups)",
            pre.components.len(),
            comp.len()
        ));

        if state.best.has_best {
            state.search.selections = state.best.best.selections.clone();
        }
        state.search.flags = rebuild_flags(
            pre.installer,
            &state.search.selections,
            pre.overrides,
            -1,
            -1,
        );

        local_search(
            state,
            pre,
            comp,
            2,
            select_any_cap,
            None,
            options_cache,
            stats,
        );
        if state.search.found_exact {
            break;
        }

        let space = estimate_search_space(
            pre,
            comp,
            &state.search.flags,
            select_any_cap,
            None,
            options_cache,
            stats,
            CONFIG.component_space_cap,
        );
        let limit = if space <= CONFIG.component_node_limit as u64 {
            0
        } else {
            CONFIG.component_node_limit
        };
        run_backtrack_pass(
            state,
            pre,
            comp,
            limit,
            "component",
            select_any_cap,
            None,
            options_cache,
            stats,
        );
    }

    log_phase_metrics(state, "component solve");
}

/// Phase 3: residual repair, run only when the best solution is already
/// near-perfect: no missing files, no extra files, at most 1 size mismatch and
/// at most 2 hash mismatches. Returns without doing anything otherwise, and also
/// when the affected group set is empty or covers every group. This phase emits
/// no `After ...` metrics line.
fn run_residual_repair(
    state: &mut SolverState,
    pre: &Precompute,
    select_any_cap: i32,
    options_cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) {
    if !state.best.has_best {
        return;
    }

    let near_perfect = state.best.best.missing == 0
        && state.best.best.extra == 0
        && state.best.best.size_mismatch <= 1
        && state.best.best.hash_mismatch <= 2;
    if !near_perfect {
        return;
    }

    let sim_best = simulate(
        pre.installer,
        pre.atoms,
        &state.best.best.selections,
        None,
        pre.overrides,
    );
    let mismatched = collect_mismatched_dests(&sim_best, pre.target, pre.excluded);
    let repair_groups = groups_for_mismatches(pre, &mismatched);

    if repair_groups.is_empty() || repair_groups.len() >= pre.groups.len() {
        return;
    }

    Logger::instance().log(&format!(
        "[solver] Residual repair mode: {} mismatched dests, {} affected groups",
        mismatched.len(),
        repair_groups.len()
    ));
    // Unconditional, unlike the phase-1 equivalent: `repair_groups` is known
    // non-empty here, so this line needs no empty guard.
    Logger::instance().log(&format!(
        "[solver] Residual mismatch-affecting groups: {}",
        join_group_names(pre, &repair_groups)
    ));

    state.search.selections = state.best.best.selections.clone();
    state.search.flags = rebuild_flags(
        pre.installer,
        &state.search.selections,
        pre.overrides,
        -1,
        -1,
    );
    local_search(
        state,
        pre,
        &repair_groups,
        3,
        select_any_cap,
        None,
        options_cache,
        stats,
    );
    run_backtrack_pass(
        state,
        pre,
        &repair_groups,
        CONFIG.residual_node_limit,
        "residual",
        select_any_cap,
        None,
        options_cache,
        stats,
    );
}

/// Phase 4: local search and a backtrack pass over the groups the mismatches
/// touch, then an exact-mode fallback pass over the same groups. Returns without
/// doing anything when there is no best solution yet, or when the focus set is
/// empty or covers every group.
fn run_focused_search(
    state: &mut SolverState,
    pre: &Precompute,
    select_any_cap: i32,
    options_cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) {
    if !state.best.has_best {
        return;
    }

    let sim_best = simulate(
        pre.installer,
        pre.atoms,
        &state.best.best.selections,
        None,
        pre.overrides,
    );
    let mismatched = collect_mismatched_dests(&sim_best, pre.target, pre.excluded);
    let focus_groups = groups_for_mismatches(pre, &mismatched);

    if focus_groups.is_empty() || focus_groups.len() >= pre.groups.len() {
        return;
    }

    Logger::instance().log(&format!(
        "[solver] Focused search: {} mismatched dests, {} groups",
        mismatched.len(),
        focus_groups.len()
    ));
    state.search.selections = state.best.best.selections.clone();
    state.search.flags = rebuild_flags(
        pre.installer,
        &state.search.selections,
        pre.overrides,
        -1,
        -1,
    );
    local_search(
        state,
        pre,
        &focus_groups,
        2,
        select_any_cap,
        None,
        options_cache,
        stats,
    );

    if !state.search.found_exact {
        let space = estimate_search_space(
            pre,
            &focus_groups,
            &state.search.flags,
            select_any_cap,
            None,
            options_cache,
            stats,
            CONFIG.focused_space_cap,
        );
        let limit = if space <= CONFIG.focused_node_limit as u64 {
            0
        } else {
            CONFIG.focused_node_limit
        };
        run_backtrack_pass(
            state,
            pre,
            &focus_groups,
            limit,
            "focused",
            select_any_cap,
            None,
            options_cache,
            stats,
        );
    }

    if !state.search.found_exact {
        let exact_focus_groups: HashSet<i32> = focus_groups.iter().copied().collect();
        Logger::instance().log(&format!(
            "[solver] Focused exact fallback: {} groups",
            focus_groups.len()
        ));

        state.search.selections = state.best.best.selections.clone();
        state.search.flags = rebuild_flags(
            pre.installer,
            &state.search.selections,
            pre.overrides,
            -1,
            -1,
        );

        let exact_space = estimate_search_space(
            pre,
            &focus_groups,
            &state.search.flags,
            select_any_cap,
            Some(&exact_focus_groups),
            options_cache,
            stats,
            CONFIG.exact_focused_space_cap,
        );
        let exact_limit = if exact_space <= CONFIG.exact_focused_node_limit as u64 {
            0
        } else {
            CONFIG.exact_focused_node_limit
        };
        run_backtrack_pass(
            state,
            pre,
            &focus_groups,
            exact_limit,
            "focused-exact",
            select_any_cap,
            Some(&exact_focus_groups),
            options_cache,
            stats,
        );
    }

    log_phase_metrics(state, "focused search");
}

/// Outcome of one global fallback pass: whether the pass hit its node limit,
/// and how many SelectAny options its cap dropped. The cap ladder in
/// [`run_global_fallback`] branches on both.
struct GlobalPassOutcome {
    hit_limit: bool,
    capped_options: i32,
}

/// One global fallback backtrack pass over the canonical group order.
///
/// Restarts from the best selections, clears the option cache, and derives the
/// node limit from the estimated search space. At the full cap the limit is also
/// clamped to `CONFIG.full_pass_default_limit`, or to
/// `CONFIG.full_pass_imperfect_limit` when the best still has missing or extra
/// files. Returns a zeroed outcome without searching when the solve is already
/// exact or past the deadline.
#[allow(clippy::too_many_arguments)]
fn run_global_pass(
    state: &mut SolverState,
    pre: &Precompute,
    global_order: &[i32],
    cap: i32,
    label: &str,
    exact_groups: Option<&HashSet<i32>>,
    options_cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) -> GlobalPassOutcome {
    if state.search.found_exact || state.progress.deadline_exceeded {
        return GlobalPassOutcome {
            hit_limit: false,
            capped_options: 0,
        };
    }

    if state.best.has_best {
        state.search.selections = state.best.best.selections.clone();
    }
    state.search.flags = rebuild_flags(
        pre.installer,
        &state.search.selections,
        pre.overrides,
        -1,
        -1,
    );

    options_cache.clear();
    let search_space = estimate_search_space(
        pre,
        global_order,
        &state.search.flags,
        cap,
        exact_groups,
        options_cache,
        stats,
        CONFIG.global_space_cap,
    );
    let mut node_limit = if search_space <= CONFIG.global_node_limit as u64 {
        0
    } else {
        CONFIG.global_node_limit
    };
    if cap == SELECT_ANY_CAP_FULL {
        let mut full_limit = CONFIG.full_pass_default_limit;
        if state.best.has_best && (state.best.best.missing > 0 || state.best.best.extra > 0) {
            full_limit = CONFIG.full_pass_imperfect_limit;
        }
        if node_limit == 0 {
            node_limit = full_limit;
        } else {
            node_limit = node_limit.min(full_limit);
        }
    }

    let capped_before = stats.capped_select_any_options;
    let pruned_before = stats.pruned_node_limit;
    run_backtrack_pass(
        state,
        pre,
        global_order,
        node_limit,
        label,
        cap,
        exact_groups,
        options_cache,
        stats,
    );

    GlobalPassOutcome {
        hit_limit: stats.pruned_node_limit > pruned_before,
        capped_options: stats.capped_select_any_options - capped_before,
    }
}

/// Phase 5: global fallback over one canonical group order, widening the
/// SelectAny cap rung by rung.
///
/// The rungs are narrow (64), medium (256), a targeted medium pass that puts the
/// mismatch-affected groups in exact mode, and full (uncapped). Each rung runs
/// only while no exact match exists. The targeted and full rungs additionally
/// need the medium rung to have hit its node limit, capped an option, or left
/// mismatches behind.
fn run_global_fallback(
    state: &mut SolverState,
    pre: &Precompute,
    options_cache: &mut HashMap<OptionCacheKey, CachedOptions>,
    stats: &mut SolverStats,
) {
    if state.best.has_best {
        state.search.selections = state.best.best.selections.clone();
    }
    state.search.flags = rebuild_flags(
        pre.installer,
        &state.search.selections,
        pre.overrides,
        -1,
        -1,
    );

    let mut base_order: Vec<i32> = Vec::new();
    for comp in &pre.components {
        for &gidx in comp {
            base_order.push(gidx);
        }
    }
    if base_order.is_empty() {
        base_order = (0..pre.groups.len() as i32).collect();
    }
    base_order.sort_unstable();
    let global_order = base_order;

    let narrow = run_global_pass(
        state,
        pre,
        &global_order,
        SELECT_ANY_CAP_NARROW,
        "global",
        None,
        options_cache,
        stats,
    );
    let mut medium = GlobalPassOutcome {
        hit_limit: false,
        capped_options: 0,
    };
    if !state.search.found_exact {
        Logger::instance().log(&format!(
            "[solver] Option widening: SelectAny cap {} -> {}",
            format_option_cap(SELECT_ANY_CAP_NARROW),
            format_option_cap(SELECT_ANY_CAP_MEDIUM)
        ));
        medium = run_global_pass(
            state,
            pre,
            &global_order,
            SELECT_ANY_CAP_MEDIUM,
            "global-widened",
            None,
            options_cache,
            stats,
        );
    }

    let capped_any = narrow.capped_options > 0 || medium.capped_options > 0;
    let unresolved_after_medium = !state.search.found_exact
        && state.best.has_best
        && (state.best.best.missing > 0
            || state.best.best.extra > 0
            || state.best.best.size_mismatch > 0
            || state.best.best.hash_mismatch > 0);
    let need_full_fallback =
        !state.search.found_exact && (medium.hit_limit || capped_any || unresolved_after_medium);

    if need_full_fallback {
        if !state.search.found_exact && state.best.has_best {
            let sim_best = simulate(
                pre.installer,
                pre.atoms,
                &state.best.best.selections,
                None,
                pre.overrides,
            );
            let mismatched = collect_mismatched_dests(&sim_best, pre.target, pre.excluded);
            let affected = groups_for_mismatches(pre, &mismatched);
            if !affected.is_empty() && affected.len() < pre.groups.len() {
                Logger::instance().log(&format!(
                    "[solver] Global targeted fallback: {} mismatched dests, {} affected groups",
                    mismatched.len(),
                    affected.len()
                ));
                let exact_groups: HashSet<i32> = affected.iter().copied().collect();
                run_global_pass(
                    state,
                    pre,
                    &global_order,
                    SELECT_ANY_CAP_MEDIUM,
                    "global-targeted",
                    Some(&exact_groups),
                    options_cache,
                    stats,
                );
            }
        }

        let unresolved_after_targeted = !state.search.found_exact
            && state.best.has_best
            && (state.best.best.missing > 0
                || state.best.best.extra > 0
                || state.best.best.size_mismatch > 0
                || state.best.best.hash_mismatch > 0);
        if unresolved_after_targeted {
            Logger::instance().log(&format!(
                "[solver] Option widening: SelectAny cap {} -> {}",
                format_option_cap(SELECT_ANY_CAP_MEDIUM),
                format_option_cap(SELECT_ANY_CAP_FULL)
            ));
            run_global_pass(
                state,
                pre,
                &global_order,
                SELECT_ANY_CAP_FULL,
                "global-full",
                None,
                options_cache,
                stats,
            );
        }
    }
}

/// Group ordering priority for the per-step sort. Higher sorts first, so the
/// most constrained group types are assigned before SelectAny.
fn group_priority(t: FomodGroupType) -> i32 {
    match t {
        FomodGroupType::SelectAll => 4,
        FomodGroupType::SelectExactlyOne => 3,
        FomodGroupType::SelectAtMostOne => 2,
        FomodGroupType::SelectAtLeastOne => 1,
        FomodGroupType::SelectAny => 0,
    }
}

/// Infer the plugin selections that best reproduce a target file tree.
///
/// Builds the flat group list (document order, then a per-step priority sort
/// with a document-order tiebreak), precomputes the read-only solver data, seeds
/// an all-deselected state, then drives the five phases described in the module
/// doc. Returns the best [`SolverResult`] found. It never signals failure;
/// turning a poor result into `""` is the inference service's concern.
///
/// **`propagation` narrows and labels, nothing more.**
///
/// - `build_precompute` hands it to
///   [`crate::fomod_csp_options::get_options_for_group`], which drops any option
///   that would select a plugin `narrowed_domains` eliminated.
/// - `resolved_groups` decides which `phase_per_group` entries come out empty.
///
/// It does not skip a phase, shorten the search, or seed the selection grid. The
/// seed is unconditionally all-deselected, and `PropagationResult::fully_resolved`
/// is never read in this module. Passing `None` only means every group keeps its
/// full option domain.
///
/// **Time budget.** The solve is capped at `CONFIG.time_limit_seconds` (600 s)
/// from entry. Expiry is not an error: phases 2 through 5 are skipped, a running
/// backtrack unwinds, and the best result so far is returned. The returned
/// [`SolverResult`] carries no timed-out flag, so a caller cannot tell a
/// completed search from a truncated one; only the `[solver] Wall-clock time
/// limit ... exceeded` log line records it.
///
/// **Cost.** Runs on the calling thread and can block for the full time budget.
/// It performs no file I/O: it reads the already-built `atoms` and `target` and
/// writes only log lines.
pub fn solve_fomod_csp(
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    atom_index: &AtomIndex,
    target: &TargetTree,
    excluded_dests: &HashSet<String>,
    overrides: Option<&InferenceOverrides>,
    propagation: Option<&PropagationResult>,
) -> SolverResult {
    // (S1) Flat GroupRef list in document order (flat_start captured pre-sort).
    let mut groups: Vec<GroupRef> = Vec::new();
    let mut flat_plugins = 0i32;
    for (si, step) in installer.steps.iter().enumerate() {
        for (gi, group) in step.groups.iter().enumerate() {
            let pc = group.plugins.len() as i32;
            groups.push(GroupRef {
                step_idx: si as i32,
                group_idx: gi as i32,
                flat_start: flat_plugins,
                plugin_count: pc,
            });
            flat_plugins += pc;
        }
    }

    // (S2) Per-step sort: priority descending, then plugin_count ascending, then
    // document order. The third key keeps the order total, so two otherwise
    // equal groups cannot swap places between runs.
    for si in 0..installer.steps.len() as i32 {
        let Some(begin) = groups.iter().position(|g| g.step_idx == si) else {
            continue;
        };
        let end = groups.iter().rposition(|g| g.step_idx == si).unwrap() + 1;
        groups[begin..end].sort_by(|a, b| {
            let pa = group_priority(
                installer.steps[a.step_idx as usize].groups[a.group_idx as usize].r#type,
            );
            let pb = group_priority(
                installer.steps[b.step_idx as usize].groups[b.group_idx as usize].r#type,
            );
            pb.cmp(&pa)
                .then(a.plugin_count.cmp(&b.plugin_count))
                .then(a.group_idx.cmp(&b.group_idx))
        });
    }

    // (S3) Precompute + seed state (every plugin deselected).
    let evidence = compute_evidence(installer, atoms, atom_index, target, excluded_dests);
    let pre = build_precompute(
        installer,
        atoms,
        atom_index,
        target,
        excluded_dests,
        overrides,
        propagation,
        groups,
        evidence,
    );

    let mut state = SolverState::default();
    for step in &installer.steps {
        let mut step_sel: Vec<Vec<bool>> = Vec::with_capacity(step.groups.len());
        for group in &step.groups {
            step_sel.push(vec![false; group.plugins.len()]);
        }
        state.search.selections.push(step_sel);
    }

    // (S4) Deadline, stats, cache, initial cap.
    state.progress.deadline =
        Some(Instant::now() + Duration::from_secs(CONFIG.time_limit_seconds as u64));
    let mut stats = SolverStats {
        logged_group_options: vec![false; pre.groups.len()],
        ..SolverStats::default()
    };
    let mut options_cache: HashMap<OptionCacheKey, CachedOptions> = HashMap::new();
    let select_any_cap = SELECT_ANY_CAP_NARROW;

    // Total plugin count, for the log line only; nothing branches on it.
    let flat_plugins: i32 = pre.groups.iter().map(|g| g.plugin_count).sum();
    Logger::instance().log(&format!(
        "[solver] Starting CSP: {} groups, {flat_plugins} total plugins, {} components",
        pre.groups.len(),
        pre.components.len()
    ));

    // (S5) Phase sequence; phases 2-5 run only while unsolved and in budget.
    let mut ran_phase2 = false;
    let mut ran_phase3 = false;
    let mut ran_phase4 = false;
    let mut ran_phase5 = false;

    run_initial_phases(
        &mut state,
        &pre,
        select_any_cap,
        &mut options_cache,
        &mut stats,
    );

    if !state.search.found_exact && !state.progress.deadline_exceeded {
        ran_phase2 = true;
        run_component_decomposition(
            &mut state,
            &pre,
            select_any_cap,
            &mut options_cache,
            &mut stats,
        );
    }
    if !state.search.found_exact && !state.progress.deadline_exceeded {
        ran_phase3 = true;
        run_residual_repair(
            &mut state,
            &pre,
            select_any_cap,
            &mut options_cache,
            &mut stats,
        );
    }
    if !state.search.found_exact && !state.progress.deadline_exceeded {
        ran_phase4 = true;
        run_focused_search(
            &mut state,
            &pre,
            select_any_cap,
            &mut options_cache,
            &mut stats,
        );
    }
    if !state.search.found_exact && !state.progress.deadline_exceeded {
        ran_phase5 = true;
        run_global_fallback(&mut state, &pre, &mut options_cache, &mut stats);
    }

    // (S6) Final reporting, then assemble the result.
    if state.progress.deadline_exceeded {
        Logger::instance().log(&format!(
            "[solver] Wall-clock time limit ({}s) exceeded after {} nodes",
            CONFIG.time_limit_seconds, state.search.nodes_explored
        ));
    }

    if state.best.has_best {
        Logger::instance().log(&format!(
            "[solver] Done: {} nodes, exact={}, missing={}, extra={}, size_mm={}, hash_mm={}",
            state.search.nodes_explored,
            state.best.best.exact_match,
            state.best.best.missing,
            state.best.best.extra,
            state.best.best.size_mismatch,
            state.best.best.hash_mismatch
        ));
    } else {
        Logger::instance().log(&format!(
            "[solver] No solution found ({} nodes explored)",
            state.search.nodes_explored
        ));
    }

    Logger::instance().log(&format!(
        "[solver] Pruning summary: extra_only={}, lower_bound={}, memo={}, invisible_skip={}, node_limit={}, max_depth_aborts={}",
        stats.pruned_extra_only,
        stats.pruned_lower_bound,
        stats.pruned_memo,
        stats.skipped_invisible,
        stats.pruned_node_limit,
        stats.max_depth_aborts
    ));

    Logger::instance().log(&format!(
        "[solver] Domain reduction summary: dropped_extra_only_options={}, collapsed_equivalent={}, forced_unique={}, capped_select_any={}",
        stats.dropped_extra_only_options,
        stats.collapsed_equivalent_options,
        stats.forced_unique_options,
        stats.capped_select_any_options
    ));

    state.best.best.nodes_explored = state.search.nodes_explored;

    // `final_phase` is the highest phase entered, not the phase that produced
    // the result: each `ran_phaseN` is set before the call, so a phase that
    // returns on its own precondition still wins the label. The phase-2 label
    // reads "csp.local_search" although phase 2 is component decomposition.
    // These five strings reach schema-v2 JSON and `inference_diagnostics` maps
    // them to reason codes by exact match, falling back to no reason at all, so
    // renaming one changes the emitted output. See PARITY-NOTES.md.
    let final_phase = if ran_phase5 {
        "csp.fallback"
    } else if ran_phase4 {
        "csp.focused"
    } else if ran_phase3 {
        "csp.repair"
    } else if ran_phase2 {
        "csp.local_search"
    } else {
        "csp.greedy"
    };
    state.best.best.phase_reached = final_phase.to_string();

    // `phase_per_group` is not per-group provenance: every group the propagator
    // did not resolve gets the same `final_phase` string, whether or not any
    // phase touched it. The only per-group distinction is the empty string for a
    // propagation-resolved group. `alternatives_per_group` is filled with zeros
    // and nothing ever computes a real alternative count; the zeros feed the
    // ambiguity term of the confidence model, so leave them alone. See
    // PARITY-NOTES.md and the field docs on `SolverResult`.
    state.best.best.phase_per_group = Vec::with_capacity(installer.steps.len());
    state.best.best.alternatives_per_group = Vec::with_capacity(installer.steps.len());
    for (s, step) in installer.steps.iter().enumerate() {
        let mut pg_row: Vec<String> = Vec::with_capacity(step.groups.len());
        let alt_row: Vec<i32> = vec![0; step.groups.len()];
        for g in 0..step.groups.len() {
            let mut prop_resolved = false;
            if let Some(prop) = propagation {
                for &(ps, pg) in &prop.resolved_groups {
                    if ps == s as i32 && pg == g as i32 {
                        prop_resolved = true;
                        break;
                    }
                }
            }
            pg_row.push(if prop_resolved {
                String::new()
            } else {
                final_phase.to_string()
            });
        }
        state.best.best.phase_per_group.push(pg_row);
        state.best.best.alternatives_per_group.push(alt_row);
    }

    state.best.best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_atom::{AtomIndex, FomodAtom, Origin, TargetFile};
    use crate::fomod_csp_precompute::{build_precompute, compute_evidence};
    use crate::fomod_ir::{
        FomodCondition, FomodConditionType, FomodConditionalPattern, FomodGroup, FomodPlugin,
        FomodStep,
    };

    // --- builders ---------------------------------------------------------

    fn plugin(name: &str) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            ..FomodPlugin::default()
        }
    }

    fn plugin_flag(name: &str, fname: &str, fval: &str) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            condition_flags: vec![(fname.to_string(), fval.to_string())],
            ..FomodPlugin::default()
        }
    }

    fn patom(dest: &str, source: &str, plugin_index: i32) -> FomodAtom {
        FomodAtom {
            source_path: source.to_string(),
            dest_path: dest.to_string(),
            origin: Origin::Plugin,
            plugin_index,
            ..FomodAtom::default()
        }
    }

    fn grp(gt: FomodGroupType, plugins: Vec<FomodPlugin>) -> FomodGroup {
        FomodGroup {
            name: "g".to_string(),
            r#type: gt,
            plugins,
        }
    }

    fn one_step(groups: Vec<FomodGroup>) -> FomodInstaller {
        FomodInstaller {
            steps: vec![FomodStep {
                groups,
                ..FomodStep::default()
            }],
            ..FomodInstaller::default()
        }
    }

    fn target_of(entries: &[(&str, u64)]) -> TargetTree {
        entries
            .iter()
            .map(|(d, s)| (d.to_string(), TargetFile { size: *s, hash: 0 }))
            .collect()
    }

    fn build_index(atoms: &ExpandedAtoms) -> AtomIndex {
        let mut idx: AtomIndex = HashMap::new();
        for a in &atoms.required {
            idx.entry(a.dest_path.clone()).or_default().push(a.clone());
        }
        for v in &atoms.per_plugin {
            for a in v {
                idx.entry(a.dest_path.clone()).or_default().push(a.clone());
            }
        }
        for v in &atoms.per_conditional {
            for a in v {
                idx.entry(a.dest_path.clone()).or_default().push(a.clone());
            }
        }
        idx
    }

    /// Document-order GroupRefs (pre.groups position == document group index),
    /// deliberately bypassing the per-step priority sort so the tests pin a known
    /// order.
    fn doc_refs(installer: &FomodInstaller) -> Vec<GroupRef> {
        let mut groups = Vec::new();
        let mut flat = 0i32;
        for (si, step) in installer.steps.iter().enumerate() {
            for (gi, group) in step.groups.iter().enumerate() {
                let pc = group.plugins.len() as i32;
                groups.push(GroupRef {
                    step_idx: si as i32,
                    group_idx: gi as i32,
                    flat_start: flat,
                    plugin_count: pc,
                });
                flat += pc;
            }
        }
        groups
    }

    fn seed_state(installer: &FomodInstaller) -> SolverState {
        let mut state = SolverState::default();
        for step in &installer.steps {
            let mut ss = Vec::new();
            for g in &step.groups {
                ss.push(vec![false; g.plugins.len()]);
            }
            state.search.selections.push(ss);
        }
        state
    }

    fn fresh_stats(pre: &Precompute) -> SolverStats {
        SolverStats {
            logged_group_options: vec![false; pre.groups.len()],
            ..SolverStats::default()
        }
    }

    // --- evaluate_candidate: equal metrics do not replace ------------------

    #[test]
    fn evaluate_candidate_keeps_first_at_equal_metrics() {
        // Two plugins both produce target "t"; target "u" is producible by nobody
        // (always missing). Selecting p0 or p1 yields the same metric tuple, so
        // the first-scored candidate wins.
        let installer = one_step(vec![grp(
            FomodGroupType::SelectExactlyOne,
            vec![plugin("p0"), plugin("p1")],
        )]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![patom("t", "s0", 0)], vec![patom("t", "s1", 1)]],
            ..ExpandedAtoms::default()
        };
        let index = build_index(&atoms);
        let target = target_of(&[("t", 0), ("u", 0)]);
        let excluded = HashSet::new();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_refs(&installer),
            ev,
        );

        let mut state = seed_state(&installer);

        state.search.selections[0][0] = vec![true, false];
        let m0 = evaluate_candidate(
            &mut state,
            pre.installer,
            pre.atoms,
            pre.target,
            pre.excluded,
            pre.overrides,
        );
        assert_eq!((m0.missing, m0.reproduced), (1, 1));
        assert_eq!(state.best.best.selections[0][0], vec![true, false]);

        state.search.selections[0][0] = vec![false, true];
        let m1 = evaluate_candidate(
            &mut state,
            pre.installer,
            pre.atoms,
            pre.target,
            pre.excluded,
            pre.overrides,
        );
        assert_eq!(m1, m0, "metrics tie");
        assert_eq!(
            state.best.best.selections[0][0],
            vec![true, false],
            "equal metrics must not replace the first-found best"
        );
    }

    // --- apply_option -----------------------------------------------------

    #[test]
    fn apply_option_writes_option_masked_by_plugin_count() {
        let mut sel = vec![vec![vec![false, false, false]]];
        let gref = GroupRef {
            step_idx: 0,
            group_idx: 0,
            flat_start: 0,
            plugin_count: 3,
        };
        // Option shorter than plugin_count: the trailing plugin stays false.
        apply_option(&mut sel, &gref, &[true, false]);
        assert_eq!(sel[0][0], vec![true, false, false]);
        apply_option(&mut sel, &gref, &[false, true, true]);
        assert_eq!(sel[0][0], vec![false, true, true]);
    }

    // --- contested_signature byte-exactness -------------------------------

    #[test]
    fn contested_signature_folds_selected_assigned_contested_in_sorted_order() {
        // One SelectAny group, two plugins both producing target "d" -> both are
        // contested (target producers). plugin_to_group = [0, 0].
        let installer = one_step(vec![grp(
            FomodGroupType::SelectAny,
            vec![plugin("p0"), plugin("p1")],
        )]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![patom("d", "s0", 0)], vec![patom("d", "s1", 1)]],
            ..ExpandedAtoms::default()
        };
        let index = build_index(&atoms);
        let target = target_of(&[("d", 0)]);
        let excluded = HashSet::new();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_refs(&installer),
            ev,
        );
        assert_eq!(pre.contested_plugins, vec![0, 1]);

        let plan = SearchPlan {
            order: vec![0],
            order_pos: vec![0],
            node_limit: 0,
            memo: HashMap::new(),
            incremental_flags: true,
        };
        let mut state = seed_state(&installer);

        // Only p0 selected, group assigned (order_pos 0 < next_idx 1): fold (0+1).
        state.search.selections[0][0] = vec![true, false];
        let mut expect = 14695981039346656037u64;
        hash_combine(&mut expect, 1);
        assert_eq!(contested_signature(&state, &pre, &plan, 1), expect);

        // Both selected: fold (0+1) then (1+1) in sorted contested order.
        state.search.selections[0][0] = vec![true, true];
        let mut expect_both = 14695981039346656037u64;
        hash_combine(&mut expect_both, 1);
        hash_combine(&mut expect_both, 2);
        assert_eq!(contested_signature(&state, &pre, &plan, 1), expect_both);

        // next_idx 0: the group's order_pos (0) is not < 0, so nothing folds.
        assert_eq!(
            contested_signature(&state, &pre, &plan, 0),
            14695981039346656037u64
        );
    }

    // --- two flag-replay orders differ ------------------------------------

    #[test]
    fn rebuild_flags_document_order_vs_advance_group_order() {
        // Two SelectAll groups: g0 sets F=a, g1 sets F=b. Document order
        // (rebuild_flags) ends F=b (g1 last); advancing groups in reverse (g1
        // then g0) ends F=a (g0 last).
        let installer = one_step(vec![
            grp(FomodGroupType::SelectAll, vec![plugin_flag("pA", "F", "a")]),
            grp(FomodGroupType::SelectAll, vec![plugin_flag("pB", "F", "b")]),
        ]);
        let refs = doc_refs(&installer);
        let selections = vec![vec![vec![true], vec![true]]];

        let doc = rebuild_flags(&installer, &selections, None, -1, -1);
        assert_eq!(doc.get("F").map(String::as_str), Some("b"));

        let mut flags: HashMap<String, String> = HashMap::new();
        let mut undo = Vec::new();
        advance_flags_past_group(&mut flags, &installer, &selections, &refs[1], &mut undo);
        advance_flags_past_group(&mut flags, &installer, &selections, &refs[0], &mut undo);
        assert_eq!(flags.get("F").map(String::as_str), Some("a"));

        assert_ne!(doc.get("F"), flags.get("F"), "the two replay orders differ");
    }

    // --- lower_bound counts a mismatch only when unfixable ------------------

    #[test]
    fn lower_bound_skips_a_dest_a_later_group_can_still_produce() {
        // g0 produces extra "x"; g1 produces target "d". With nothing selected
        // "d" is missing; lower_bound must skip it while g1 is still ahead in the
        // order, and must count it once g1 is behind.
        let installer = one_step(vec![
            grp(FomodGroupType::SelectAny, vec![plugin("p0")]),
            grp(FomodGroupType::SelectAny, vec![plugin("p1")]),
        ]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![patom("x", "sx", 0)], vec![patom("d", "sd", 1)]],
            ..ExpandedAtoms::default()
        };
        let index = build_index(&atoms);
        let target = target_of(&[("d", 0)]);
        let excluded = HashSet::new();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_refs(&installer),
            ev,
        );
        assert_eq!(pre.dest_to_groups.get("d").cloned(), Some(vec![1]));

        let plan = SearchPlan {
            order: vec![0, 1],
            order_pos: vec![0, 1],
            node_limit: 0,
            memo: HashMap::new(),
            incremental_flags: true,
        };
        let state = seed_state(&installer);

        let lb0 = lower_bound(&state, &pre, &plan, 0);
        assert_eq!(lb0.missing, 0, "fixable-by-ahead-group dest must not count");

        let lb2 = lower_bound(&state, &pre, &plan, 2);
        assert_eq!(lb2.missing, 1, "unfixable missing dest must count");

        let sim = simulate(
            pre.installer,
            pre.atoms,
            &state.search.selections,
            None,
            pre.overrides,
        );
        assert_eq!(compare_trees(&sim, pre.target, pre.excluded).missing, 1);
    }

    // --- extra-only option prune ------------------------------------------

    #[test]
    fn backtrack_prunes_extra_only_options() {
        // A SelectAtLeastOne group whose plugins produce only extra dests and set
        // no needed flag: every option is extra-only, so all survive reduce
        // (keep-empty fallback) and the backtrack prunes each in the branch
        // state.
        let installer = one_step(vec![grp(
            FomodGroupType::SelectAtLeastOne,
            vec![plugin("p0"), plugin("p1")],
        )]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![patom("x0", "s0", 0)], vec![patom("x1", "s1", 1)]],
            ..ExpandedAtoms::default()
        };
        let index = build_index(&atoms);
        let target = target_of(&[("t", 0)]); // never produced
        let excluded = HashSet::new();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_refs(&installer),
            ev,
        );

        let mut state = seed_state(&installer);
        let mut cache = HashMap::new();
        let mut stats = fresh_stats(&pre);
        run_backtrack_pass(
            &mut state,
            &pre,
            &[0],
            0,
            "test",
            SELECT_ANY_CAP_NARROW,
            None,
            &mut cache,
            &mut stats,
        );
        assert!(
            stats.pruned_extra_only > 0,
            "expected extra-only options to be pruned"
        );
    }

    // --- node-limit >= boundary -------------------------------------------

    #[test]
    fn node_limit_boundary_uses_gte() {
        // greedy scores one node (nodes_explored == 1); a follow-up backtrack
        // with node_limit == 1 prunes immediately (>= boundary), keeping the
        // greedy best.
        let installer = one_step(vec![grp(
            FomodGroupType::SelectExactlyOne,
            vec![plugin("p0"), plugin("p1")],
        )]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![patom("a", "sa", 0)], vec![patom("b", "sb", 1)]],
            ..ExpandedAtoms::default()
        };
        let index = build_index(&atoms);
        let target = target_of(&[("a", 0), ("b", 0), ("c", 0)]); // "c" unreachable
        let excluded = HashSet::new();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_refs(&installer),
            ev,
        );

        let mut state = seed_state(&installer);
        let mut cache = HashMap::new();
        let mut stats = fresh_stats(&pre);

        greedy_solve(
            &mut state,
            &pre,
            SELECT_ANY_CAP_NARROW,
            None,
            &mut cache,
            &mut stats,
        );
        assert_eq!(state.search.nodes_explored, 1);
        assert!(state.best.has_best);
        assert!(!state.search.found_exact, "target 'c' is unreachable");

        run_backtrack_pass(
            &mut state,
            &pre,
            &[0],
            1,
            "test",
            SELECT_ANY_CAP_NARROW,
            None,
            &mut cache,
            &mut stats,
        );
        assert!(stats.pruned_node_limit > 0, "node_limit>=nodes must prune");
        assert!(
            state.best.has_best,
            "best-so-far survives the node-limit prune"
        );
    }

    // --- memo prune -------------------------------------------------------

    #[test]
    fn backtrack_memo_prunes_equivalent_subtree() {
        // Two branches of group 0 (both set F=v, neither is a target producer)
        // converge on an identical (flag, contested) state at the branching group
        // at order position 4, so the second visit re-hits the memo with a
        // not-better lower bound and is pruned.
        let g0 = grp(
            FomodGroupType::SelectExactlyOne,
            vec![plugin_flag("p0", "F", "v"), plugin_flag("p1", "F", "v")],
        );
        let g1 = grp(FomodGroupType::SelectExactlyOne, vec![plugin("q1")]);
        let g2 = grp(FomodGroupType::SelectExactlyOne, vec![plugin("q2")]);
        let g3 = grp(FomodGroupType::SelectExactlyOne, vec![plugin("q3")]);
        let g4 = grp(
            FomodGroupType::SelectExactlyOne,
            vec![plugin("r0"), plugin("r1")],
        );
        let mut installer = one_step(vec![g0, g1, g2, g3, g4]);
        // A conditional gated on F makes F a "needed" flag, so g0's flag-only
        // options are not dropped as extra-only. It produces nothing.
        installer.conditional_patterns = vec![FomodConditionalPattern {
            condition: FomodCondition {
                r#type: FomodConditionType::Flag,
                flag_name: "F".to_string(),
                flag_value: "v".to_string(),
                ..FomodCondition::default()
            },
            files: vec![],
        }];

        let atoms = ExpandedAtoms {
            per_plugin: vec![
                vec![patom("x0", "sx0", 0)],  // g0 p0 -> extra
                vec![patom("x1", "sx1", 1)],  // g0 p1 -> extra
                vec![patom("t1", "st1", 2)],  // g1
                vec![patom("t2", "st2", 3)],  // g2
                vec![patom("t3", "st3", 4)],  // g3
                vec![patom("t4a", "s4a", 5)], // g4 r0
                vec![patom("t4b", "s4b", 6)], // g4 r1
            ],
            per_conditional: vec![vec![]],
            ..ExpandedAtoms::default()
        };
        let index = build_index(&atoms);
        let target = target_of(&[("t1", 0), ("t2", 0), ("t3", 0), ("t4a", 0), ("t4b", 0)]);
        let excluded = HashSet::new();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_refs(&installer),
            ev,
        );

        let mut state = seed_state(&installer);
        let mut cache = HashMap::new();
        let mut stats = fresh_stats(&pre);

        // Establish a non-exact best (group 4 can only cover one of t4a/t4b).
        greedy_solve(
            &mut state,
            &pre,
            SELECT_ANY_CAP_NARROW,
            None,
            &mut cache,
            &mut stats,
        );
        assert!(state.best.has_best && !state.search.found_exact);

        run_backtrack_pass(
            &mut state,
            &pre,
            &[0, 1, 2, 3, 4],
            0,
            "test",
            SELECT_ANY_CAP_NARROW,
            None,
            &mut cache,
            &mut stats,
        );
        assert!(
            stats.pruned_memo > 0,
            "expected a memo re-hit prune (pruned_memo={})",
            stats.pruned_memo
        );
    }

    // --- deadline / budget smoke ------------------------------------------

    fn tiny_nonrepro() -> (
        FomodInstaller,
        ExpandedAtoms,
        AtomIndex,
        TargetTree,
        HashSet<String>,
    ) {
        let installer = one_step(vec![grp(
            FomodGroupType::SelectExactlyOne,
            vec![plugin("p0"), plugin("p1")],
        )]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![patom("a", "sa", 0)], vec![patom("b", "sb", 1)]],
            ..ExpandedAtoms::default()
        };
        let index = build_index(&atoms);
        let target = target_of(&[("a", 0), ("b", 0), ("c", 0)]); // "c" unreachable
        (installer, atoms, index, target, HashSet::new())
    }

    #[test]
    fn deadline_in_the_past_trips_and_none_never_does() {
        let (installer, atoms, index, target, excluded) = tiny_nonrepro();
        let ev = compute_evidence(&installer, &atoms, &index, &target, &excluded);
        let pre = build_precompute(
            &installer,
            &atoms,
            &index,
            &target,
            &excluded,
            None,
            None,
            doc_refs(&installer),
            ev,
        );

        // (A) deadline already in the past -> deadline_exceeded flips, no panic.
        let mut state = seed_state(&installer);
        state.progress.deadline = Some(Instant::now() - Duration::from_secs(1));
        let mut cache = HashMap::new();
        let mut stats = fresh_stats(&pre);
        run_backtrack_pass(
            &mut state,
            &pre,
            &[0],
            0,
            "test",
            SELECT_ANY_CAP_NARROW,
            None,
            &mut cache,
            &mut stats,
        );
        assert!(state.progress.deadline_exceeded, "past deadline must trip");

        // (B) deadline None -> the deadline path is never taken; the pass runs to
        // completion and records a best.
        let mut state2 = seed_state(&installer);
        assert!(state2.progress.deadline.is_none());
        let mut cache2 = HashMap::new();
        let mut stats2 = fresh_stats(&pre);
        run_backtrack_pass(
            &mut state2,
            &pre,
            &[0],
            0,
            "test",
            SELECT_ANY_CAP_NARROW,
            None,
            &mut cache2,
            &mut stats2,
        );
        assert!(
            !state2.progress.deadline_exceeded,
            "None deadline never trips"
        );
        assert!(state2.best.has_best, "the pass explored at least one leaf");
    }
}
