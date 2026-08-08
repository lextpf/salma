//! Forward simulation of a FOMOD install, and the metrics that score it.
//!
//! [`simulate`] replays a candidate `[step][group][plugin]` selection grid into
//! an in-memory [`SimulatedTree`] instead of writing files, and
//! [`compare_trees`] diffs that tree against the tree the mod actually has on
//! disk. Together they are the scoring oracle every CSP solver phase depends on:
//! the solver proposes a selection, this module says how closely it reproduces
//! the installed mod.
//!
//! The score is meaningful because the simulator resolves a destination conflict
//! the way [`crate::fomod_service::execute_file_operations`] does: the
//! highest-priority atom wins, and on equal priority the one applied last wins.
//! The two do not feed that tiebreak from the same sequence, so equal-priority
//! conflicts are where they can part over a shared destination. Gates one side
//! applies and the other does not part them over whether a file is installed at
//! all. The next section has both.
//!
//! [`compare_trees_impl`], [`compare_trees`] and [`collect_mismatched_dests`]
//! live here rather than with the solver because they read a [`SimulatedTree`];
//! [`crate::fomod_csp_solver`] imports them. The solver's `lower_bound` reuses
//! [`compare_trees_impl`] with its own three predicates. Never copy the
//! size/hash/reproduced else-chain into the solver: two copies of it would drift
//! apart without any test noticing.
//!
//! ## Where the simulator and the installer diverge
//!
//! This section is the authoritative account of the difference.
//! [`crate::fomod_service::execute_file_operations`] points here rather than
//! restating it, so correct it here and leave that side a pointer.
//!
//! Both sides are last-writer-wins on a tie, but they compare different keys.
//! The installer stamps each `FileOperation` with a `document_order` counter as
//! it enqueues the operation, then stable-sorts by `(priority, document_order)`
//! and copies in that order, so "last" there means "enqueued last". The
//! simulator carries no such stamp: [`should_overwrite`] compares `priority`
//! alone, so "last" here means "applied last" under its own phase order plus
//! step, group and plugin walk order. It never reads
//! `FomodAtom::document_order`, and that field is not the installer's counter
//! either: `fomod_service::enqueue_entry` numbers operations as the install
//! passes queue them, while
//! [`crate::fomod_inference_atoms::expand_all_atoms`] numbers entries in its own
//! four loops.
//!
//! Two kinds of divergence follow. The first is which atom wins a destination
//! both sides produce. Any conflict a priority difference settles comes out the
//! same on both sides. An equal-priority conflict comes out the same only where
//! enqueue order and application order agree. These cases are known to break
//! that:
//!
//! - A Required-typed plugin promoted by
//!   `fomod_service::process_optional_files` in its per-step auto-install loop.
//!   That loop runs only after every selected plugin of the step is enqueued, so
//!   the promoted plugin is enqueued last and wins. The simulator promotes it
//!   inline at its position in the phase-2 walk, so a selected plugin later in
//!   the same step is applied later and wins instead.
//! - The same promotion for a step the selections JSON does not name. That pass
//!   runs after every JSON-named step, so the promoted plugin outranks a
//!   selected plugin of any step, not only its own, while the simulator still
//!   promotes it inline at its IR position.
//! - Two entries of one selected plugin on one destination, where the XML lists
//!   an alwaysInstall / installIfUsable entry before a normal one. The installer
//!   enqueues a plugin's entries in XML order, so the normal entry is enqueued
//!   later and wins. `expand_all_atoms` fills that plugin's bucket with its
//!   normal atoms first and its auto atoms after, and phase 2 applies the whole
//!   bucket in that order, so the auto atom is applied later and wins.
//! - Selections JSON that lists steps or groups in an order the IR does not use.
//!   The installer enqueues in JSON order; the simulator always walks IR order.
//!
//! The second kind is a gate one side applies and the other does not. That
//! changes which files are installed at all, whatever the priorities:
//!
//! - A plugin whose type turns `Required` only after a later plugin of the same
//!   step sets a flag. `fomod_service::process_optional_files` evaluates the
//!   type in its per-step auto-install loop, after every selected plugin of the
//!   step has run, so it promotes the plugin, applies its `condition_flags` and
//!   enqueues all of its files. The simulator evaluates the type inline in phase
//!   2, before that flag exists, so it does not promote; phase 3 then applies
//!   only the plugin's `always_install` and `install_if_usable` atoms and sets
//!   none of its flags. The predicted tree is missing that plugin's normal
//!   files, and a conditional pattern gated on one of its flags fires only on
//!   the install side.
//! - A `Required` plugin in a step the selections JSON names that is hidden
//!   while its own step is processed and visible by the end.
//!   `fomod_service::process_optional_files` skips that step on visibility in
//!   pass 1, its per-step auto-install loop with it; skips it again in pass 2,
//!   because the covered-step set holds step names; then reaches it visible in
//!   pass 3 and drops the plugin on `eff_type == PluginType::Required`, so the
//!   install writes none of its files. Phase 3 has no counterpart to that skip:
//!   it finds the step visible and applies the plugin's `always_install` and
//!   `install_if_usable` atoms. Here the predicted tree carries files the
//!   install does not, the reverse of the case above.
//! - A selected plugin whose `<dependencies>` do not hold.
//!   `fomod_service::process_optional_files` skips it before
//!   `apply_condition_flags`, so the install gets none of its files and none of
//!   its flags. The simulator does not read `plugin.dependencies` at all and
//!   applies both. Nothing upstream stops the solver proposing such a
//!   selection: `fomod_csp_precompute` reads the field only to collect flag
//!   names.
//!
//! Treat both lists as the known set, not a proof of completeness. The general
//! statement is the one to reason from: equal priority is where the two can
//! differ over a shared destination, and any further gap between enqueue order
//! and application order adds a case; a gate present on one side only is where
//! they differ over whether a file is installed at all.
//!
//! Do not reorder the phases to close the gap. The application-order tiebreak is
//! deliberate, and changing it changes the predicted winner of every
//! equal-priority collision the solver scores against. See `PARITY-NOTES.md`.
//!
//! ## Phases
//!
//! [`simulate_into`] runs four passes, in this order.
//!
//! 1. **Required.** Every `atoms.required` atom, applied unconditionally in
//!    vector order.
//! 2. **Selected and Required-typed plugins**, walked per step, then group, then
//!    plugin. `selected` comes from the bounds-checked grid; an unselected
//!    plugin whose `evaluate_plugin_type` against the flag state so far is
//!    `Required` is promoted. The grid and that promotion are the only
//!    plugin-level gates: `plugin.dependencies` is never evaluated here. A
//!    selected plugin first accumulates its `condition_flags` (last write wins
//!    per name), then applies every one of its atoms unconditionally, with no
//!    always_install / install_if_usable filter. `flat_idx` advances once per
//!    plugin whatever happens: an invisible step advances it past every group's
//!    plugins but applies nothing and sets no flags.
//! 3. **Auto atoms of the plugins phase 2 did not handle**, scored against the
//!    final flag state. `flat_idx` restarts at 0 and step visibility is
//!    re-evaluated, so a step invisible in phase 2 can become visible here and
//!    the reverse. `eff_type` is evaluated against the final flags:
//!    `always_install` atoms apply unconditionally, `install_if_usable` atoms
//!    apply only when `eff_type != NotUsable`.
//! 4. **Conditional installs.** Each conditional pattern, active via
//!    `evaluate_condition_inferred` when an override entry exists and
//!    `evaluate_condition` otherwise, applied in order when active.
//!
//! Phase 3 re-evaluates visibility rather than caching phase 2's answer because
//! the flag map grows as phase 2 runs. Phase 2 asks whether a step is visible at
//! that plugin's position; phase 3 asks whether it is visible at the end.
//!
//! ## Application order is not `document_order`
//!
//! ```text
//!   apply order                    atom source                          doc_order range
//!   -----------------------------------------------------------------------------------
//!   phase 1                        atoms.required                       [0 .. R)
//!
//!   phase 2  flat_idx 0,1,2,...    per_plugin[i], for a selected or
//!                                  Required-promoted plugin i
//!              normal atoms        of plugin i                          [R .. R+N)
//!              auto atoms          of plugin i                          [R+N .. R+N+A)
//!              then plugin i+1, whose normal atoms are in               [R .. R+N)
//!
//!   phase 3  flat_idx back to 0    per_plugin[i], for a plugin phase 2
//!                                  did not handle; auto atoms only      [R+N .. R+N+A)
//!
//!   phase 4                        per_conditional[ci]                  [R+N+A .. end)
//! ```
//!
//! [`crate::fomod_inference_atoms::expand_all_atoms`] numbers every normal
//! plugin entry below every always-install / installIfUsable entry, in a
//! separate global pass. Plugin 0's auto atom therefore carries a higher
//! `document_order` than plugin 1's normal atom, yet phase 2 applies plugin 0's
//! auto atom first. Phase 3 then restarts `flat_idx` at 0, so it can apply an
//! atom whose `document_order` is lower than one phase 2 already applied. The
//! applied sequence is therefore not monotonic in `document_order`, neither
//! within phase 2 nor across the phase-2 to phase-3 boundary.
//!
//! [`should_overwrite`] compares `priority` alone, and neither it nor
//! [`apply_atom`] reads `document_order`. Never sort the atoms by
//! `document_order` before applying them: that changes the winner of every
//! equal-priority collision.

use std::collections::{HashMap, HashSet};

use crate::fomod_atom::{ExpandedAtoms, FomodAtom, TargetTree};
use crate::fomod_csp_types::{InferenceOverrides, ReproMetrics};
use crate::fomod_dependency_evaluator::{
    evaluate_condition, evaluate_condition_inferred, evaluate_plugin_type,
};
use crate::fomod_ir::{FomodInstaller, FomodStep};
use crate::types::{FomodDependencyContext, PluginType};

/// The file tree produced by a simulated FOMOD installation.
///
/// Maps a destination path to the single winning [`FomodAtom`] after priority
/// and application-order conflict resolution, so the solver can score a
/// candidate selection without extracting anything.
///
/// The key is `FomodAtom::dest_path` stored verbatim; the simulator normalizes
/// nothing itself. In the normal pipeline the key already arrives in
/// `crate::utils::normalize_path` form (lowercase, forward slashes, no leading
/// or trailing `/`), because `crate::fomod_ir_parser::parse_module_config`
/// normalizes every parsed destination and
/// [`crate::fomod_inference_atoms::expand_entry`]'s folder branch normalizes the
/// concatenated one; the single-file branch passes the parser's value straight
/// through.
///
/// A caller that builds [`ExpandedAtoms`] by hand must pre-normalize every
/// `dest_path`. Unnormalized keys never match a [`TargetTree`], whose keys come
/// from the normalized installed-file scan, and [`compare_trees`] then reports
/// each destination as both missing and extra.
#[derive(Debug, Clone, Default)]
pub struct SimulatedTree {
    /// dest -> winning atom.
    pub files: HashMap<String, FomodAtom>,
}

/// An atom overwrites the incumbent at its destination when its priority is
/// `>=` the incumbent's.
///
/// `>=` rather than `>` means the last atom applied wins a tie, the same shape
/// as the real installer's stable sort, where the later-enqueued operation wins.
/// What counts as "last" is [`simulate_into`]'s fixed application order, which is
/// neither the XML order, nor `FomodAtom::document_order` (never read here), nor
/// the installer's enqueue order:
///
/// 1. `atoms.required`, in vector order.
/// 2. Per step, group and plugin in IR order, every atom of each selected or
///    Required-promoted plugin, in vector order.
/// 3. A second walk from `flat_idx = 0` over the plugins phase 2 did not handle,
///    applying their `always_install` atoms, and their `install_if_usable` atoms
///    when the effective type is not `NotUsable`.
/// 4. Conditional patterns, in order.
///
/// `FomodAtom::document_order` is a different sequence.
/// [`crate::fomod_inference_atoms::expand_all_atoms`] numbers every normal
/// plugin entry below every always-install / installIfUsable entry, in a
/// separate global pass, so plugin 0's auto atom outranks plugin 1's normal atom
/// yet is applied first. Re-sorting the phases by `document_order` would change
/// the winner of every equal-priority collision. The module doc has the ranges
/// it carries, and the known cases where this tiebreak and the installer's
/// disagree.
fn should_overwrite(existing: &FomodAtom, new_atom: &FomodAtom) -> bool {
    new_atom.priority >= existing.priority
}

/// Insert `atom` at its destination when the slot is empty, or when the
/// incumbent loses to it under [`should_overwrite`].
fn apply_atom(tree: &mut SimulatedTree, atom: &FomodAtom) {
    let overwrite = match tree.files.get(&atom.dest_path) {
        Some(existing) => should_overwrite(existing, atom),
        None => true,
    };
    if overwrite {
        tree.files.insert(atom.dest_path.clone(), atom.clone());
    }
}

/// Step-visibility decision shared by phases 2 and 3, evaluated against the flag
/// map as it stands at the call:
///
/// - no visibility condition -> `true`;
/// - overrides present and `si` in range -> `evaluate_condition_inferred` with
///   `overrides.step_visible[si]`;
/// - otherwise, including overrides present but the vector too short ->
///   `evaluate_condition` in normal mode.
fn compute_step_visibility(
    si: usize,
    step: &FomodStep,
    flags: &HashMap<String, String>,
    context: Option<&FomodDependencyContext>,
    overrides: Option<&InferenceOverrides>,
) -> bool {
    let Some(visible) = &step.visible else {
        return true;
    };
    match overrides {
        Some(ov) if si < ov.step_visible.len() => {
            evaluate_condition_inferred(visible, flags, ov.step_visible[si], context)
        }
        _ => evaluate_condition(visible, flags, context),
    }
}

/// Run a forward simulation for one selection, producing a fresh tree.
///
/// `selections` is a boolean grid indexed `selections[step][group][plugin]`. Its
/// dimensions need not match the installer's counts: a missing or short axis
/// reads as `false` (deselected). An empty outer slice is valid and means no
/// plugin is explicitly selected; Required promotion and auto atoms still run.
pub fn simulate(
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    selections: &[Vec<Vec<bool>>],
    context: Option<&FomodDependencyContext>,
    overrides: Option<&InferenceOverrides>,
) -> SimulatedTree {
    let mut tree = SimulatedTree::default();
    simulate_into(&mut tree, installer, atoms, selections, context, overrides);
    tree
}

/// In-place [`simulate`] that reuses an existing tree's allocation.
///
/// `tree.files` is cleared before the simulation begins, keeping its capacity,
/// then repopulated with the result.
pub fn simulate_into(
    tree: &mut SimulatedTree,
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    selections: &[Vec<Vec<bool>>],
    context: Option<&FomodDependencyContext>,
    overrides: Option<&InferenceOverrides>,
) {
    tree.files.clear();
    let mut flags: HashMap<String, String> = HashMap::new();

    // Phase 1: Required files.
    for atom in &atoms.required {
        apply_atom(tree, atom);
    }

    // Phase 2: chronological pass for selected / Required-typed plugins.
    let mut processed_in_phase2 = vec![false; atoms.per_plugin.len()];
    let mut flat_idx: usize = 0;
    for (si, step) in installer.steps.iter().enumerate() {
        if !compute_step_visibility(si, step, &flags, context, overrides) {
            // Skip all groups in this invisible step, but still advance flat_idx.
            for group in &step.groups {
                flat_idx += group.plugins.len();
            }
            continue;
        }

        for (gi, group) in step.groups.iter().enumerate() {
            for (pi, plugin) in group.plugins.iter().enumerate() {
                let mut selected = selections
                    .get(si)
                    .and_then(|g| g.get(gi))
                    .and_then(|p| p.get(pi))
                    .copied()
                    .unwrap_or(false);

                if !selected
                    && evaluate_plugin_type(plugin, &flags, context) == PluginType::Required
                {
                    selected = true;
                }

                if selected && flat_idx < atoms.per_plugin.len() {
                    for (name, value) in &plugin.condition_flags {
                        flags.insert(name.clone(), value.clone());
                    }
                    for atom in &atoms.per_plugin[flat_idx] {
                        apply_atom(tree, atom);
                    }
                    processed_in_phase2[flat_idx] = true;
                }
                flat_idx += 1;
            }
        }
    }

    // Phase 3: final-flag-state pass for unselected non-Required plugins.
    flat_idx = 0;
    for (si, step) in installer.steps.iter().enumerate() {
        if !compute_step_visibility(si, step, &flags, context, overrides) {
            for group in &step.groups {
                flat_idx += group.plugins.len();
            }
            continue;
        }

        for group in &step.groups {
            for plugin in &group.plugins {
                if flat_idx >= atoms.per_plugin.len() || processed_in_phase2[flat_idx] {
                    flat_idx += 1;
                    continue;
                }
                let eff_type = evaluate_plugin_type(plugin, &flags, context);
                for atom in &atoms.per_plugin[flat_idx] {
                    // always_install applies unconditionally; install_if_usable
                    // applies only when the effective type is not NotUsable.
                    // Both outcomes are the same apply_atom call, so the two
                    // branches collapse into one condition.
                    if atom.always_install
                        || (atom.install_if_usable && eff_type != PluginType::NotUsable)
                    {
                        apply_atom(tree, atom);
                    }
                }
                flat_idx += 1;
            }
        }
    }

    // Phase 4: conditional file installs.
    for (ci, pattern) in installer.conditional_patterns.iter().enumerate() {
        let active = match overrides {
            Some(ov) if ci < ov.conditional_active.len() => evaluate_condition_inferred(
                &pattern.condition,
                &flags,
                ov.conditional_active[ci],
                context,
            ),
            _ => evaluate_condition(&pattern.condition, &flags, context),
        };
        if active && ci < atoms.per_conditional.len() {
            for atom in &atoms.per_conditional[ci] {
                apply_atom(tree, atom);
            }
        }
    }
}

/// Generic tree comparison: walk `sim` against `target` and call the three
/// predicates to decide whether each divergence is counted.
///
/// Each predicate receives the destination path and returns true to count that
/// divergence. [`compare_trees`] passes always-true predicates.
/// `fomod_csp_solver::lower_bound` passes `can_fix_missing` / `can_fix_size` /
/// `can_fix_hash`, each true only when no still-unassigned group and no
/// remaining conditional pattern can produce or repair that destination, which
/// is what keeps its bound admissible. Only `missing`, `size_mismatch` and
/// `hash_mismatch` are gated; `reproduced` and `extra` take no predicate and are
/// always counted.
///
/// **Decision table.** A zero on either side of a comparison skips that branch
/// and falls through toward reproduced:
///
/// ```text
///   in target?  in sim?  sizes differ?  hashes differ?   result
///   ------------------------------------------------------------------
///   yes         no       -              -                missing
///   yes         yes      yes            not consulted    size_mismatch
///   yes         yes      no             yes              hash_mismatch
///   yes         yes      no             no               reproduced
///   no          yes      -              -                extra
/// ```
///
/// A size mismatch suppresses the hash check, so one destination is counted at
/// most once. [`collect_mismatched_dests`] and [`classify_dests`] repeat this
/// else-chain and must stay identical to it.
///
/// Destinations in `excluded` are skipped in both walks and counted nowhere.
pub fn compare_trees_impl(
    sim: &SimulatedTree,
    target: &TargetTree,
    excluded: &HashSet<String>,
    mut on_missing: impl FnMut(&str) -> bool,
    mut on_size_mismatch: impl FnMut(&str) -> bool,
    mut on_hash_mismatch: impl FnMut(&str) -> bool,
) -> ReproMetrics {
    let mut m = ReproMetrics::default();

    for (dest, tf) in target {
        if excluded.contains(dest) {
            continue;
        }
        match sim.files.get(dest) {
            None => {
                if on_missing(dest) {
                    m.missing += 1;
                }
            }
            Some(atom) => {
                if tf.size != 0 && atom.file_size != 0 && tf.size != atom.file_size {
                    if on_size_mismatch(dest) {
                        m.size_mismatch += 1;
                    }
                } else if tf.hash != 0 && atom.content_hash != 0 && tf.hash != atom.content_hash {
                    if on_hash_mismatch(dest) {
                        m.hash_mismatch += 1;
                    }
                } else {
                    m.reproduced += 1;
                }
            }
        }
    }

    for dest in sim.files.keys() {
        if excluded.contains(dest) {
            continue;
        }
        if !target.contains_key(dest) {
            m.extra += 1;
        }
    }
    m
}

/// Compare a simulated tree against the target, counting every divergence: the
/// always-true-predicate wrapper over [`compare_trees_impl`].
pub fn compare_trees(
    sim: &SimulatedTree,
    target: &TargetTree,
    excluded: &HashSet<String>,
) -> ReproMetrics {
    compare_trees_impl(sim, target, excluded, |_| true, |_| true, |_| true)
}

/// Every destination where the simulation diverges from the target, as a flat
/// union with the category discarded. Use [`classify_dests`] when the category
/// is needed.
///
/// Categories follow the same else-chain and the same decision table as
/// [`compare_trees_impl`]: a dest in target but not in sim is missing; a dest in
/// both with differing nonzero sizes is a size mismatch; else with differing
/// nonzero hashes a hash mismatch; a dest in sim but not in target is extra.
/// Reproduced dests are not collected.
///
/// The result is sorted byte-wise ascending, which is the only ordering
/// guarantee a caller may rely on. The sole consumer,
/// `fomod_csp_solver::groups_for_mismatches`, seeds from each dest's direct
/// producer groups, adds every needed-flag setter group for a dest in
/// `conditional_dests`, BFS-expands through the flag dependency chain and sorts
/// its own output, so it needs determinism rather than a particular order.
pub fn collect_mismatched_dests(
    sim: &SimulatedTree,
    target: &TargetTree,
    excluded: &HashSet<String>,
) -> Vec<String> {
    let mut out: HashSet<String> = HashSet::new();

    for (dest, tf) in target {
        if excluded.contains(dest) {
            continue;
        }
        match sim.files.get(dest) {
            None => {
                out.insert(dest.clone()); // missing
            }
            Some(atom) => {
                if tf.size != 0 && atom.file_size != 0 && tf.size != atom.file_size {
                    out.insert(dest.clone()); // size mismatch
                } else if tf.hash != 0 && atom.content_hash != 0 && tf.hash != atom.content_hash {
                    out.insert(dest.clone()); // hash mismatch
                }
            }
        }
    }

    for dest in sim.files.keys() {
        if excluded.contains(dest) {
            continue;
        }
        if !target.contains_key(dest) {
            out.insert(dest.clone()); // extra
        }
    }

    let mut mismatched: Vec<String> = out.into_iter().collect();
    mismatched.sort();
    mismatched
}

/// How one destination diverged from the target tree.
///
/// Each variant names the `diagnostics.repro` counter of the same name in
/// [`ReproMetrics`], so the per-file marks and the tally use one vocabulary.
///
/// Reproduced dests have no variant: they are the overwhelming majority, and
/// every consumer reads "this file is fine" from absence rather than paying for
/// a row per good file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DestStatus {
    /// In the target tree but not produced by the simulation.
    Missing,
    /// Produced and in the target, but the two nonzero sizes differ. This check
    /// suppresses the hash check.
    SizeMismatch,
    /// Produced and in the target with compatible sizes, but the two nonzero
    /// content hashes differ.
    HashMismatch,
    /// Produced by the simulation but absent from the target tree.
    Extra,
}

impl DestStatus {
    /// The wire name, matching the `diagnostics.repro` counter it belongs to so
    /// the payload uses one vocabulary for the tally and the paths.
    pub fn as_str(self) -> &'static str {
        match self {
            DestStatus::Missing => "missing",
            DestStatus::SizeMismatch => "size_mismatch",
            DestStatus::HashMismatch => "hash_mismatch",
            DestStatus::Extra => "extra",
        }
    }
}

/// Every diverging destination tagged with how it diverged, sorted by path.
///
/// This sits beside [`collect_mismatched_dests`] rather than replacing it
/// because the two answer different questions. The solver asks which dests are
/// wrong so it knows which groups to retarget, and its four call sites depend on
/// the flat-union semantics. This function answers what is wrong with one named
/// file, which is what a reader of the output tree needs, so it keeps the
/// category.
///
/// The else-chain below must stay identical to [`compare_trees_impl`]'s and to
/// [`collect_mismatched_dests`]': a size mismatch suppresses the hash check, and
/// a zero size or zero hash on either side falls through to reproduced. If the
/// copies drift, the per-file marks contradict the `repro` counts shipped in the
/// same payload. `classify_dests_agrees_with_compare_trees` pins that invariant.
pub fn classify_dests(
    sim: &SimulatedTree,
    target: &TargetTree,
    excluded: &HashSet<String>,
) -> Vec<(String, DestStatus)> {
    let mut out: Vec<(String, DestStatus)> = Vec::new();

    for (dest, tf) in target {
        if excluded.contains(dest) {
            continue;
        }
        match sim.files.get(dest) {
            None => out.push((dest.clone(), DestStatus::Missing)),
            Some(atom) => {
                if tf.size != 0 && atom.file_size != 0 && tf.size != atom.file_size {
                    out.push((dest.clone(), DestStatus::SizeMismatch));
                } else if tf.hash != 0 && atom.content_hash != 0 && tf.hash != atom.content_hash {
                    out.push((dest.clone(), DestStatus::HashMismatch));
                }
            }
        }
    }

    for dest in sim.files.keys() {
        if excluded.contains(dest) {
            continue;
        }
        if !target.contains_key(dest) {
            out.push((dest.clone(), DestStatus::Extra));
        }
    }

    // Both maps are hashed, so the walk order is arbitrary; sort for a stable
    // payload. A dest reaches this vector at most once - the second loop only
    // sees dests absent from the target - so path order is a total order.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_atom::{Origin, TargetFile};
    use crate::fomod_ir::{
        FomodCondition, FomodConditionOp, FomodConditionType, FomodConditionalPattern, FomodGroup,
        FomodPlugin, FomodTypePattern,
    };

    // ------------------------------------------------------------------
    // Builders for synthetic IR + atoms.
    // ------------------------------------------------------------------

    fn atom(dest: &str, source: &str, priority: i32, doc: i32) -> FomodAtom {
        FomodAtom {
            source_path: source.to_string(),
            dest_path: dest.to_string(),
            priority,
            document_order: doc,
            ..FomodAtom::default()
        }
    }

    fn plugin(name: &str, ptype: PluginType) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            r#type: ptype,
            ..FomodPlugin::default()
        }
    }

    fn flag_leaf(name: &str, value: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Flag,
            flag_name: name.to_string(),
            flag_value: value.to_string(),
            ..FomodCondition::default()
        }
    }

    fn file_leaf(path: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::File,
            file_path: path.to_string(),
            file_state: "Active".to_string(),
            ..FomodCondition::default()
        }
    }

    /// An always-true condition: an empty `And` composite (start true, no
    /// children). Used to force a type_pattern to always match.
    fn always_true() -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Composite,
            op: FomodConditionOp::And,
            ..FomodCondition::default()
        }
    }

    fn single_step(groups: Vec<FomodGroup>) -> FomodStep {
        FomodStep {
            groups,
            ..FomodStep::default()
        }
    }

    fn group(plugins: Vec<FomodPlugin>) -> FomodGroup {
        FomodGroup {
            plugins,
            ..FomodGroup::default()
        }
    }

    /// Build an installer from a list of steps, plus optional conditional
    /// patterns. `atoms.per_plugin` must be sized to the flat plugin count by
    /// the caller.
    fn installer(
        steps: Vec<FomodStep>,
        conditionals: Vec<FomodConditionalPattern>,
    ) -> FomodInstaller {
        FomodInstaller {
            steps,
            conditional_patterns: conditionals,
            ..FomodInstaller::default()
        }
    }

    fn dests(tree: &SimulatedTree) -> Vec<String> {
        let mut v: Vec<String> = tree.files.keys().cloned().collect();
        v.sort();
        v
    }

    fn winner_source<'a>(tree: &'a SimulatedTree, dest: &str) -> &'a str {
        &tree.files.get(dest).expect("dest present").source_path
    }

    #[test]
    fn higher_priority_wins_regardless_of_application_order() {
        // Two required atoms on the same dest, different priorities. Phase 1
        // applies them in vector order, so we can flip the order to prove that
        // the higher priority wins either way (a lower-priority late atom fails
        // the >= test; a higher-priority late atom passes it).
        let inst = installer(vec![], vec![]);

        let atoms_hi_first = ExpandedAtoms {
            required: vec![atom("d", "hi", 5, 0), atom("d", "lo", 3, 1)],
            ..ExpandedAtoms::default()
        };
        let t = simulate(&inst, &atoms_hi_first, &[], None, None);
        assert_eq!(winner_source(&t, "d"), "hi");

        let atoms_lo_first = ExpandedAtoms {
            required: vec![atom("d", "lo", 3, 0), atom("d", "hi", 5, 1)],
            ..ExpandedAtoms::default()
        };
        let t = simulate(&inst, &atoms_lo_first, &[], None, None);
        assert_eq!(winner_source(&t, "d"), "hi");
    }

    #[test]
    fn equal_priority_last_applied_wins_proving_gte_not_gt() {
        let inst = installer(vec![], vec![]);

        let a_then_b = ExpandedAtoms {
            required: vec![atom("d", "A", 2, 0), atom("d", "B", 2, 1)],
            ..ExpandedAtoms::default()
        };
        assert_eq!(
            winner_source(&simulate(&inst, &a_then_b, &[], None, None), "d"),
            "B"
        );

        let b_then_a = ExpandedAtoms {
            required: vec![atom("d", "B", 2, 0), atom("d", "A", 2, 1)],
            ..ExpandedAtoms::default()
        };
        assert_eq!(
            winner_source(&simulate(&inst, &b_then_a, &[], None, None), "d"),
            "A"
        );
    }

    #[test]
    fn selected_plugin_overwrites_equal_priority_required_because_phase2_is_later() {
        // Phase 1 applies the required atom; phase 2 applies the selected
        // plugin's atom later, so on equal priority the plugin wins (>=).
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let atoms = ExpandedAtoms {
            required: vec![atom("d", "req", 0, 0)],
            per_plugin: vec![vec![atom("d", "plug", 0, 1)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        assert_eq!(
            winner_source(&simulate(&inst, &atoms, &sel, None, None), "d"),
            "plug"
        );
    }

    #[test]
    fn higher_priority_required_beats_selected_plugin() {
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let atoms = ExpandedAtoms {
            required: vec![atom("d", "req", 9, 0)],
            per_plugin: vec![vec![atom("d", "plug", 1, 1)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        assert_eq!(
            winner_source(&simulate(&inst, &atoms, &sel, None, None), "d"),
            "req"
        );
    }

    #[test]
    fn condition_flags_last_write_wins_within_a_plugin() {
        // A selected plugin sets f=a then f=b; only the conditional gated on
        // f==b must fire.
        let mut p = plugin("P", PluginType::Optional);
        p.condition_flags = vec![("f".into(), "a".into()), ("f".into(), "b".into())];
        let inst = installer(
            vec![single_step(vec![group(vec![p])])],
            vec![
                FomodConditionalPattern {
                    condition: flag_leaf("f", "b"),
                    ..FomodConditionalPattern::default()
                },
                FomodConditionalPattern {
                    condition: flag_leaf("f", "a"),
                    ..FomodConditionalPattern::default()
                },
            ],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![]],
            per_conditional: vec![
                vec![atom("cond_b", "cb", 0, 10)],
                vec![atom("cond_a", "ca", 0, 11)],
            ],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        let t = simulate(&inst, &atoms, &sel, None, None);
        assert_eq!(dests(&t), vec!["cond_b".to_string()]);
    }

    #[test]
    fn condition_flags_accumulate_across_selected_plugins() {
        // plugin0 (step0) sets f=x; plugin1 (step1) sets f=y; final f=y.
        let mut p0 = plugin("P0", PluginType::Optional);
        p0.condition_flags = vec![("f".into(), "x".into())];
        let mut p1 = plugin("P1", PluginType::Optional);
        p1.condition_flags = vec![("f".into(), "y".into())];
        let inst = installer(
            vec![
                single_step(vec![group(vec![p0])]),
                single_step(vec![group(vec![p1])]),
            ],
            vec![FomodConditionalPattern {
                condition: flag_leaf("f", "y"),
                ..FomodConditionalPattern::default()
            }],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![], vec![]],
            per_conditional: vec![vec![atom("done", "d", 0, 20)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]], vec![vec![true]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["done".to_string()]
        );
    }

    #[test]
    fn type_pattern_promotes_a_later_plugin_to_required_from_earlier_flag() {
        // plugin0 selected, sets want=yes. plugin1 (later, same step) is
        // Optional but a type_pattern flips it to Required when want==yes. It is
        // not selected in the grid, yet its atom must install via promotion.
        let mut p0 = plugin("P0", PluginType::Optional);
        p0.condition_flags = vec![("want".into(), "yes".into())];
        let mut p1 = plugin("P1", PluginType::Optional);
        p1.type_patterns = vec![FomodTypePattern {
            condition: flag_leaf("want", "yes"),
            result_type: PluginType::Required,
        }];
        let inst = installer(vec![single_step(vec![group(vec![p0, p1])])], vec![]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![], vec![atom("promoted", "pr", 0, 5)]],
            ..ExpandedAtoms::default()
        };

        // p0 selected, p1 deselected -> p1 promoted, its atom installs.
        let sel = vec![vec![vec![true, false]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["promoted".to_string()]
        );

        // p0 not selected -> want unset -> p1 stays Optional -> nothing installs
        // (its lone atom is a normal, non-auto atom that never applies unselected).
        let sel_none = vec![vec![vec![false, false]]];
        assert!(
            simulate(&inst, &atoms, &sel_none, None, None)
                .files
                .is_empty()
        );
    }

    #[test]
    fn invisible_step_advances_flat_idx_and_leaks_nothing() {
        // step0 invisible (gated on show=1, never set). Its plugin is marked
        // selected and would set leak=1 and install "leaked". step1 visible
        // with a selected plugin installing "kept". The invisible step must
        // install nothing, set no flag, yet advance flat_idx so step1's plugin
        // maps to per_plugin[1] (not [0]).
        let mut hidden = plugin("Hidden", PluginType::Optional);
        hidden.condition_flags = vec![("leak".into(), "1".into())];
        let step0 = FomodStep {
            visible: Some(flag_leaf("show", "1")),
            groups: vec![group(vec![hidden])],
            ..FomodStep::default()
        };
        let step1 = single_step(vec![group(vec![plugin("Kept", PluginType::Optional)])]);
        let inst = installer(
            vec![step0, step1],
            vec![FomodConditionalPattern {
                condition: flag_leaf("leak", "1"),
                ..FomodConditionalPattern::default()
            }],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![
                vec![atom("leaked", "lk", 0, 0)],
                vec![atom("kept", "kp", 0, 1)],
            ],
            per_conditional: vec![vec![atom("leak_cond", "lc", 0, 2)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]], vec![vec![true]]];
        let t = simulate(&inst, &atoms, &sel, None, None);
        // Only "kept" installs: no leaked atom, no leak_cond (flag never set).
        assert_eq!(dests(&t), vec!["kept".to_string()]);
    }

    #[test]
    fn phase3_step_becomes_visible_with_final_flags_and_auto_atoms_apply() {
        // step0 gated on f=1 (unset during phase 2 -> invisible there). step1's
        // selected plugin sets f=1. In phase 3, step0 is now visible, so its
        // unselected plugin's always_install atom applies.
        let step0 = FomodStep {
            visible: Some(flag_leaf("f", "1")),
            groups: vec![group(vec![plugin("Auto", PluginType::Optional)])],
            ..FomodStep::default()
        };
        let mut setter = plugin("Setter", PluginType::Optional);
        setter.condition_flags = vec![("f".into(), "1".into())];
        let step1 = single_step(vec![group(vec![setter])]);
        let inst = installer(vec![step0, step1], vec![]);

        let mut auto_atom = atom("auto_out", "ao", 0, 0);
        auto_atom.always_install = true;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![auto_atom], vec![]],
            ..ExpandedAtoms::default()
        };
        // step0 plugin unselected; step1 setter selected.
        let sel = vec![vec![vec![false]], vec![vec![true]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["auto_out".to_string()]
        );
    }

    #[test]
    fn phase3_step_becomes_invisible_with_final_flags_and_auto_atoms_do_not_apply() {
        // step0 gated on hide being empty/absent (true during phase 2 ->
        // visible). step1's selected plugin sets hide=1. In phase 3 step0 turns
        // invisible, so its unselected plugin's always_install atom must not
        // apply.
        let step0 = FomodStep {
            visible: Some(flag_leaf("hide", "")),
            groups: vec![group(vec![plugin("Auto", PluginType::Optional)])],
            ..FomodStep::default()
        };
        let mut setter = plugin("Setter", PluginType::Optional);
        setter.condition_flags = vec![("hide".into(), "1".into())];
        let step1 = single_step(vec![group(vec![setter])]);
        let inst = installer(vec![step0, step1], vec![]);

        let mut auto_atom = atom("auto_out", "ao", 0, 0);
        auto_atom.always_install = true;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![auto_atom], vec![]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![false]], vec![vec![true]]];
        assert!(simulate(&inst, &atoms, &sel, None, None).files.is_empty());
    }

    #[test]
    fn selected_plugin_applies_install_if_usable_even_when_effective_type_is_not_usable() {
        // A selected plugin whose eff_type would be NotUsable still installs its
        // install_if_usable and always_install atoms in phase 2 (no filter for
        // selected plugins).
        let mut p = plugin("P", PluginType::Optional);
        p.type_patterns = vec![FomodTypePattern {
            condition: always_true(),
            result_type: PluginType::NotUsable,
        }];
        let inst = installer(vec![single_step(vec![group(vec![p])])], vec![]);

        let mut iiu = atom("iiu_out", "iu", 0, 0);
        iiu.install_if_usable = true;
        let mut always = atom("always_out", "aw", 0, 1);
        always.always_install = true;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![iiu, always]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["always_out".to_string(), "iiu_out".to_string()]
        );
    }

    #[test]
    fn unselected_not_usable_plugin_installs_always_but_not_install_if_usable() {
        let mut p = plugin("P", PluginType::Optional);
        p.type_patterns = vec![FomodTypePattern {
            condition: always_true(),
            result_type: PluginType::NotUsable,
        }];
        let inst = installer(vec![single_step(vec![group(vec![p])])], vec![]);

        let mut iiu = atom("iiu_out", "iu", 0, 0);
        iiu.install_if_usable = true;
        let mut always = atom("always_out", "aw", 0, 1);
        always.always_install = true;
        let mut normal = atom("normal_out", "nm", 0, 2); // neither flag -> never applies unselected
        normal.always_install = false;
        normal.install_if_usable = false;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![iiu, always, normal]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![false]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["always_out".to_string()]
        );
    }

    #[test]
    fn unselected_usable_plugin_installs_both_auto_atoms() {
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let mut iiu = atom("iiu_out", "iu", 0, 0);
        iiu.install_if_usable = true;
        let mut always = atom("always_out", "aw", 0, 1);
        always.always_install = true;
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![iiu, always]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![false]]];
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, None)),
            vec!["always_out".to_string(), "iiu_out".to_string()]
        );
    }

    #[test]
    fn conditional_pattern_flag_driven_activation() {
        let mut setter = plugin("Setter", PluginType::Optional);
        setter.condition_flags = vec![("c".into(), "1".into())];
        let inst = installer(
            vec![single_step(vec![group(vec![setter])])],
            vec![FomodConditionalPattern {
                condition: flag_leaf("c", "1"),
                ..FomodConditionalPattern::default()
            }],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![]],
            per_conditional: vec![vec![atom("cond_out", "co", 0, 3)]],
            ..ExpandedAtoms::default()
        };
        // selected -> flag set -> pattern fires.
        assert_eq!(
            dests(&simulate(&inst, &atoms, &[vec![vec![true]]], None, None)),
            vec!["cond_out".to_string()]
        );
        // deselected -> flag unset -> pattern does not fire.
        assert!(
            simulate(&inst, &atoms, &[vec![vec![false]]], None, None)
                .files
                .is_empty()
        );
    }

    #[test]
    fn conditional_pattern_external_override_matrix() {
        // A single File-dependency conditional; only ForceTrue activates it.
        let inst = installer(
            vec![],
            vec![FomodConditionalPattern {
                condition: file_leaf("some/dep.esp"),
                ..FomodConditionalPattern::default()
            }],
        );
        let atoms = ExpandedAtoms {
            per_conditional: vec![vec![atom("ext_out", "eo", 0, 0)]],
            ..ExpandedAtoms::default()
        };
        let sel: Vec<Vec<Vec<bool>>> = vec![];

        use crate::fomod_dependency_evaluator::ExternalConditionOverride as X;
        let cases = [
            (X::ForceTrue, true),
            (X::ForceFalse, false),
            (X::Unknown, false),
        ];
        for (mode, expected_present) in cases {
            let ov = InferenceOverrides {
                conditional_active: vec![mode],
                step_visible: vec![],
            };
            let present = !simulate(&inst, &atoms, &sel, None, Some(&ov))
                .files
                .is_empty();
            assert_eq!(present, expected_present, "override {mode:?}");
        }
        // No override at all -> normal eval, File dep with no context -> false.
        assert!(simulate(&inst, &atoms, &sel, None, None).files.is_empty());
    }

    #[test]
    fn conditional_override_vector_shorter_than_pattern_count_falls_back_to_normal() {
        // Two File-dep conditionals; overrides supplies only index 0 (ForceTrue).
        // Pattern 1 is out of override range -> normal eval -> false (no ctx).
        let inst = installer(
            vec![],
            vec![
                FomodConditionalPattern {
                    condition: file_leaf("a.esp"),
                    ..FomodConditionalPattern::default()
                },
                FomodConditionalPattern {
                    condition: file_leaf("b.esp"),
                    ..FomodConditionalPattern::default()
                },
            ],
        );
        let atoms = ExpandedAtoms {
            per_conditional: vec![
                vec![atom("a_out", "a", 0, 0)],
                vec![atom("b_out", "b", 0, 1)],
            ],
            ..ExpandedAtoms::default()
        };
        use crate::fomod_dependency_evaluator::ExternalConditionOverride as X;
        let ov = InferenceOverrides {
            conditional_active: vec![X::ForceTrue],
            step_visible: vec![],
        };
        let t = simulate(&inst, &atoms, &[], None, Some(&ov));
        assert_eq!(dests(&t), vec!["a_out".to_string()]);
    }

    #[test]
    fn step_visibility_override_forces_flag_independent_visibility() {
        // A File-dep visibility condition. ForceFalse hides the step (its
        // selected plugin does not install); ForceTrue shows it.
        let step0 = FomodStep {
            visible: Some(file_leaf("gate.esp")),
            groups: vec![group(vec![plugin("P", PluginType::Optional)])],
            ..FomodStep::default()
        };
        let inst = installer(vec![step0], vec![]);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![atom("out", "o", 0, 0)]],
            ..ExpandedAtoms::default()
        };
        let sel = vec![vec![vec![true]]];
        use crate::fomod_dependency_evaluator::ExternalConditionOverride as X;

        let ov_true = InferenceOverrides {
            conditional_active: vec![],
            step_visible: vec![X::ForceTrue],
        };
        assert_eq!(
            dests(&simulate(&inst, &atoms, &sel, None, Some(&ov_true))),
            vec!["out".to_string()]
        );

        let ov_false = InferenceOverrides {
            conditional_active: vec![],
            step_visible: vec![X::ForceFalse],
        };
        assert!(
            simulate(&inst, &atoms, &sel, None, Some(&ov_false))
                .files
                .is_empty()
        );
    }

    #[test]
    fn empty_and_short_selection_grids_deselect_everything() {
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![atom("out", "o", 0, 0)]],
            ..ExpandedAtoms::default()
        };
        // Fully empty grid.
        assert!(simulate(&inst, &atoms, &[], None, None).files.is_empty());
        // Outer present but group axis short.
        assert!(
            simulate(&inst, &atoms, &[vec![]], None, None)
                .files
                .is_empty()
        );
        // Group present but plugin axis short.
        assert!(
            simulate(&inst, &atoms, &[vec![vec![]]], None, None)
                .files
                .is_empty()
        );
    }

    #[test]
    fn simulate_into_clears_prior_contents_on_reuse() {
        let inst = installer(
            vec![single_step(vec![group(vec![plugin(
                "P",
                PluginType::Optional,
            )])])],
            vec![],
        );
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![atom("x", "x", 0, 0)]],
            ..ExpandedAtoms::default()
        };
        let mut tree = SimulatedTree::default();
        // First run selects the plugin -> "x" present.
        simulate_into(&mut tree, &inst, &atoms, &[vec![vec![true]]], None, None);
        assert_eq!(dests(&tree), vec!["x".to_string()]);
        // Second run selects nothing -> tree must be cleared, "x" gone.
        simulate_into(&mut tree, &inst, &atoms, &[], None, None);
        assert!(tree.files.is_empty());
    }

    fn sim_with(entries: &[(&str, u64, u64)]) -> SimulatedTree {
        // (dest, file_size, content_hash)
        let mut t = SimulatedTree::default();
        for (dest, size, hash) in entries {
            let mut a = atom(dest, dest, 0, 0);
            a.file_size = *size;
            a.content_hash = *hash;
            a.origin = Origin::Plugin;
            t.files.insert(dest.to_string(), a);
        }
        t
    }

    fn target_with(entries: &[(&str, u64, u64)]) -> TargetTree {
        // (dest, size, hash)
        entries
            .iter()
            .map(|(d, s, h)| (d.to_string(), TargetFile { size: *s, hash: *h }))
            .collect()
    }

    fn excluded(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn compare_trees_size_mismatch_suppresses_hash_check() {
        // Both sizes nonzero and differ -> size_mismatch, hash never consulted
        // even though hashes also differ.
        let sim = sim_with(&[("d", 10, 99)]);
        let target = target_with(&[("d", 20, 88)]);
        let m = compare_trees(&sim, &target, &excluded(&[]));
        assert_eq!(
            m,
            ReproMetrics {
                size_mismatch: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_equal_size_differing_hash_is_hash_mismatch() {
        let sim = sim_with(&[("d", 10, 99)]);
        let target = target_with(&[("d", 10, 88)]);
        let m = compare_trees(&sim, &target, &excluded(&[]));
        assert_eq!(
            m,
            ReproMetrics {
                hash_mismatch: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_zero_target_size_still_runs_hash_check() {
        // target size 0 skips the size branch; nonzero differing hashes on both
        // sides still reach the hash branch.
        let sim = sim_with(&[("d", 10, 99)]);
        let target = target_with(&[("d", 0, 88)]);
        let m = compare_trees(&sim, &target, &excluded(&[]));
        assert_eq!(
            m,
            ReproMetrics {
                hash_mismatch: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_zero_hash_either_side_falls_through_to_reproduced() {
        // sim hash 0 -> hash branch skipped -> reproduced.
        let m = compare_trees(
            &sim_with(&[("d", 10, 0)]),
            &target_with(&[("d", 10, 88)]),
            &excluded(&[]),
        );
        assert_eq!(
            m,
            ReproMetrics {
                reproduced: 1,
                ..Default::default()
            }
        );
        // target hash 0 -> reproduced.
        let m = compare_trees(
            &sim_with(&[("d", 10, 99)]),
            &target_with(&[("d", 10, 0)]),
            &excluded(&[]),
        );
        assert_eq!(
            m,
            ReproMetrics {
                reproduced: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_zero_size_either_side_falls_through_when_hashes_agree() {
        // sim size 0 -> size branch skipped; hashes equal -> reproduced.
        let m = compare_trees(
            &sim_with(&[("d", 0, 77)]),
            &target_with(&[("d", 20, 77)]),
            &excluded(&[]),
        );
        assert_eq!(
            m,
            ReproMetrics {
                reproduced: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_counts_missing_and_extra() {
        let sim = sim_with(&[("only_sim", 5, 0)]);
        let target = target_with(&[("only_target", 5, 0)]);
        let m = compare_trees(&sim, &target, &excluded(&[]));
        assert_eq!(
            m,
            ReproMetrics {
                missing: 1,
                extra: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compare_trees_excluded_skipped_in_both_loops() {
        // "gone" would be missing (target-only), "surplus" would be extra
        // (sim-only); both excluded -> neither counted. "keep" reproduced.
        let sim = sim_with(&[("keep", 5, 0), ("surplus", 9, 0)]);
        let target = target_with(&[("keep", 5, 0), ("gone", 3, 0)]);
        let m = compare_trees(&sim, &target, &excluded(&["gone", "surplus"]));
        assert_eq!(
            m,
            ReproMetrics {
                reproduced: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn collect_mismatched_dests_categories_match_compare_trees() {
        // One of each: missing, size mismatch, hash mismatch, extra, plus a
        // reproduced and an excluded dest that must not appear.
        let sim = sim_with(&[
            ("size_bad", 10, 0),  // vs target 20 -> size mismatch
            ("hash_bad", 10, 99), // vs target hash 88 -> hash mismatch
            ("repro_ok", 10, 0),  // vs target 10 -> reproduced
            ("extra_one", 7, 0),  // not in target -> extra
            ("skip_extra", 7, 0), // extra but excluded
        ]);
        let target = target_with(&[
            ("size_bad", 20, 0),
            ("hash_bad", 10, 88),
            ("repro_ok", 10, 0),
            ("missing_one", 3, 0),  // not in sim -> missing
            ("skip_missing", 3, 0), // missing but excluded
        ]);
        let excl = excluded(&["skip_extra", "skip_missing"]);

        let mut got = collect_mismatched_dests(&sim, &target, &excl);
        got.sort();
        assert_eq!(
            got,
            vec![
                "extra_one".to_string(),
                "hash_bad".to_string(),
                "missing_one".to_string(),
                "size_bad".to_string(),
            ]
        );

        // Cross-check: the collected count equals the sum of the four error
        // counters compare_trees reports on the same tree.
        let m = compare_trees(&sim, &target, &excl);
        let error_sum = (m.missing + m.extra + m.size_mismatch + m.hash_mismatch) as usize;
        assert_eq!(got.len(), error_sum);
        assert_eq!(m.reproduced, 1);
    }

    /// The invariant that keeps the per-file marks honest: every bucket of
    /// `classify_dests` must be exactly as large as the `compare_trees` counter
    /// of the same name. If the two else-chains ever drift, the output tree
    /// would contradict the `repro` tally shipped in the same JSON.
    #[test]
    fn classify_dests_agrees_with_compare_trees() {
        // Same fixture as the collect_mismatched_dests parity test: one of
        // each category, plus a reproduced dest and two excluded ones.
        let sim = sim_with(&[
            ("size_bad", 10, 0),
            ("hash_bad", 10, 99),
            ("repro_ok", 10, 0),
            ("extra_one", 7, 0),
            ("skip_extra", 7, 0),
        ]);
        let target = target_with(&[
            ("size_bad", 20, 0),
            ("hash_bad", 10, 88),
            ("repro_ok", 10, 0),
            ("missing_one", 3, 0),
            ("skip_missing", 3, 0),
        ]);
        let excl = excluded(&["skip_extra", "skip_missing"]);

        let got = classify_dests(&sim, &target, &excl);

        // Sorted by path, category preserved, reproduced and excluded absent.
        assert_eq!(
            got,
            vec![
                ("extra_one".to_string(), DestStatus::Extra),
                ("hash_bad".to_string(), DestStatus::HashMismatch),
                ("missing_one".to_string(), DestStatus::Missing),
                ("size_bad".to_string(), DestStatus::SizeMismatch),
            ]
        );

        let m = compare_trees(&sim, &target, &excl);
        let count = |s: DestStatus| got.iter().filter(|(_, st)| *st == s).count() as i32;
        assert_eq!(count(DestStatus::Missing), m.missing);
        assert_eq!(count(DestStatus::Extra), m.extra);
        assert_eq!(count(DestStatus::SizeMismatch), m.size_mismatch);
        assert_eq!(count(DestStatus::HashMismatch), m.hash_mismatch);
    }

    /// A size mismatch suppresses the hash check in `compare_trees_impl`, so it
    /// must suppress it here too - otherwise one file would be marked twice.
    #[test]
    fn classify_dests_size_mismatch_suppresses_hash_check() {
        let sim = sim_with(&[("both_wrong", 10, 99)]);
        let target = target_with(&[("both_wrong", 20, 88)]);
        let got = classify_dests(&sim, &target, &excluded(&[]));
        assert_eq!(
            got,
            vec![("both_wrong".to_string(), DestStatus::SizeMismatch)]
        );
    }

    /// A zero size or zero hash on either side falls through to reproduced, so
    /// such a file must not be marked at all.
    #[test]
    fn classify_dests_zero_size_or_hash_falls_through_to_reproduced() {
        let sim = sim_with(&[("zero_size", 0, 5), ("zero_hash", 10, 0)]);
        let target = target_with(&[("zero_size", 40, 5), ("zero_hash", 10, 7)]);
        assert!(classify_dests(&sim, &target, &excluded(&[])).is_empty());
    }

    #[test]
    fn compare_trees_impl_predicate_gate_can_suppress_counting() {
        // Prove the generic predicate variant is honored: gating on_missing to
        // false suppresses the missing count that compare_trees would report.
        let sim = sim_with(&[]);
        let target = target_with(&[("a", 1, 0), ("b", 1, 0)]);
        // Count only dest "a" as missing.
        let m = compare_trees_impl(
            &sim,
            &target,
            &excluded(&[]),
            |d| d == "a",
            |_| true,
            |_| true,
        );
        assert_eq!(m.missing, 1);
        // The unfiltered wrapper counts both.
        assert_eq!(compare_trees(&sim, &target, &excluded(&[])).missing, 2);
    }
}
