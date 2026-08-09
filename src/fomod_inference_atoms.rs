//! Atom expansion and response assembly for inference.
//!
//! Two halves, both owned here so the whole inference output is produced in one
//! place:
//!
//! - **Expansion.** Resolve the FOMOD IR's file entries into concrete
//!   [`FomodAtom`]s against the archive's entry list, index those atoms by
//!   destination, work out which destinations carry no solver signal, and build
//!   the [`TargetTree`] from the files the installed mod actually contains.
//! - **Assembly.** Turn a solver result plus its diagnostics into the schema-v2
//!   JSON response, then attach the `outputTree` and `reproDetail` siblings.
//!
//! Four warnings reach `logs/salma.log` from here, all tagged `[infer]`: a
//! skipped unsafe destination during expansion, an out-of-bounds selection index
//! during assembly, and one per truncated payload for `outputTree` and
//! `reproDetail`.

use std::collections::{HashMap, HashSet};

use crate::fomod_atom::{AtomIndex, ExpandedAtoms, FomodAtom, Origin, TargetFile, TargetTree};
use crate::fomod_csp_types::SolverResult;
use crate::fomod_forward_simulator::{DestStatus, SimulatedTree};
use crate::fomod_ir::{FomodFileEntry, FomodInstaller, total_flat_plugins};
use crate::inference_diagnostics::{
    GroupDiagnostics, InferenceDiagnostics, PluginDiagnostics, StepDiagnostics,
    serialize_confidence, serialize_reason, serialize_run_diagnostics,
};
use crate::json::Value;
use crate::logger::Logger;
use crate::utils::{is_safe_destination, normalize_path};

/// Destination validation for the inference pipeline. Forwards to
/// [`is_safe_destination`].
///
/// Read that function's contract before relying on this as a traversal guard. It
/// normalizes first, and normalization deletes `..` segments, so
/// `is_safe_dest("..")` is true. The one live rejection is a normalized form
/// whose second byte is `:`, a Windows drive letter such as `c:/windows`. A
/// rooted destination is not rejected: normalization strips leading slashes
/// before the leading-slash branch is reached, so `is_safe_dest("/etc/passwd")`
/// is true as well.
pub fn is_safe_dest(dest: &str) -> bool {
    is_safe_destination(dest)
}

/// Expand one [`FomodFileEntry`] into atoms and append them to `out`.
///
/// The two branches differ in ways that matter:
///
/// - **`entry.is_folder == true`**: a prefix search over `sorted_entries`, which
///   the caller must have put through [`normalize_path`] (lowercase, forward
///   slashes, no leading or trailing slash) and sorted in byte order, finds
///   every archive member under the source directory and emits one atom per
///   match. The search compares those strings directly against
///   `FomodFileEntry::source`, which the parser normalizes the same way, so a
///   `sorted_entries` string not already in that form (mixed case, backslashes)
///   matches nothing, silently, and yields no atom. A folder whose
///   source matches nothing emits no atoms. Each atom's `dest_path` is freshly
///   normalized; `source_path` is the matched `sorted_entries` string stored
///   verbatim.
/// - **`entry.is_folder == false`**: exactly one atom is emitted and
///   `sorted_entries` is never consulted. The source is not checked for
///   existence in the archive, so an entry naming a missing source still
///   produces an atom, with `file_size` 0. The atom's `dest_path` is
///   `entry.destination` verbatim; only the `meta.ini` test below normalizes it
///   first. `source_path` is `entry.source` verbatim.
///
/// The normalized form [`FomodAtom`] documents for `source_path` is the caller's
/// guarantee, not this function's: nothing here lowercases or re-separates
/// either string.
///
/// Both branches skip two kinds of destination, and nothing else suppresses an
/// atom:
///
/// - A destination that normalizes to the top-level `meta.ini` is skipped
///   silently, matching [`build_target_tree`]'s exclusion of the installed-side
///   MO2 metadata file. Keeping it would put a file in every simulated tree that
///   the target tree can never hold, making an exact match unreachable.
/// - A destination [`is_safe_dest`] rejects is skipped and logged. That check
///   rejects a drive-qualified destination such as `c:/evil` and nothing else.
///   It rejects neither `..` traversal nor a rooted path: normalization deletes
///   `..` segments and leading slashes before the check, and the file branch
///   then stores the raw destination, so an atom whose `dest_path` still reads
///   `../evil` or `/etc/passwd` can be produced.
///
/// All atoms from one call share `doc_order`. The caller increments it per file
/// entry, not per atom.
///
/// `entry_sizes` maps an archive entry path, in that same normalized form, to
/// its uncompressed size in bytes. A path missing from the map gives `file_size`
/// 0, which every later size comparison treats as unknown and lets pass.
#[allow(clippy::too_many_arguments)] // flat expansion inputs, no useful grouping
pub fn expand_entry(
    entry: &FomodFileEntry,
    sorted_entries: &[String],
    entry_sizes: &HashMap<String, u64>,
    doc_order: i32,
    origin: Origin,
    plugin_idx: i32,
    cond_idx: i32,
    out: &mut Vec<FomodAtom>,
) {
    if entry.is_folder {
        // An empty source means "root of archive": match every entry. Two
        // empty-string identities make that fall out with no special case:
        //   * partition_point over "" returns 0, because "" sorts at or before
        //     every other string.
        //   * str::starts_with("") is unconditionally true.
        // So the loop visits every entry exactly once.
        //
        // For a non-root folder the prefix carries a trailing slash, which
        // forces a path-boundary match: without it, folder entry "foo" would
        // also match the sibling file "foobar.esp".
        let mut prefix = entry.source.clone();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }

        // First entry at or after the prefix.
        let start = sorted_entries.partition_point(|e| e.as_str() < prefix.as_str());
        for entry_path in &sorted_entries[start..] {
            if !entry_path.starts_with(&prefix) {
                break;
            }
            let rel = &entry_path[prefix.len()..];
            // Concatenate raw, normalize once afterwards.
            let dest = if entry.destination.is_empty() {
                rel.to_string()
            } else {
                format!("{}/{}", entry.destination, rel)
            };

            let norm_dest = normalize_path(&dest);
            // Skip an archive-shipped top-level meta.ini: build_target_tree
            // excludes the installed one as MO2 metadata, so emitting it here
            // would make exact_match unreachable.
            if norm_dest == "meta.ini" {
                continue;
            }
            if !is_safe_dest(&norm_dest) {
                Logger::instance().log_warning(&format!(
                    "[infer] Skipping atom with unsafe destination: {norm_dest}"
                ));
                continue;
            }

            out.push(FomodAtom {
                source_path: entry_path.clone(),
                dest_path: norm_dest,
                priority: entry.priority,
                document_order: doc_order,
                origin,
                plugin_index: plugin_idx,
                conditional_index: cond_idx,
                always_install: entry.always_install,
                install_if_usable: entry.install_if_usable,
                file_size: entry_sizes.get(entry_path).copied().unwrap_or(0),
                content_hash: 0,
            });
        }
    } else {
        // Same top-level meta.ini exclusion as the folder branch. The parser
        // normalizes destinations already; normalizing again covers a direct
        // caller. It normalizes for the test only: the atom keeps the raw
        // entry.destination, and is_safe_dest also runs on the raw value.
        if normalize_path(&entry.destination) == "meta.ini" {
            return;
        }
        if !is_safe_dest(&entry.destination) {
            Logger::instance().log_warning(&format!(
                "[infer] Skipping atom with unsafe destination: {}",
                entry.destination
            ));
            return;
        }

        out.push(FomodAtom {
            source_path: entry.source.clone(),
            dest_path: entry.destination.clone(),
            priority: entry.priority,
            document_order: doc_order,
            origin,
            plugin_index: plugin_idx,
            conditional_index: cond_idx,
            always_install: entry.always_install,
            install_if_usable: entry.install_if_usable,
            file_size: entry_sizes.get(&entry.source).copied().unwrap_or(0),
            content_hash: 0,
        });
    }
}

/// Expand every FOMOD file entry into atoms, grouped by origin.
///
/// Four separate traversals, numbered here the way the in-function comments
/// number them. Splitting them is deliberate, but `document_order` is not the
/// reason: no production code reads [`FomodAtom`]'s `document_order`, so the
/// ranges below label the traversal sequence rather than driving it. What the
/// split fixes is the order inside each `per_plugin` bucket.
/// [`crate::fomod_forward_simulator`] applies a selected plugin's whole bucket
/// at once, so loops 2 and 3 put a plugin's normal atoms ahead of its auto
/// atoms, and merging them flips the winner of an equal-priority conflict inside
/// one plugin. `PARITY-NOTES.md`, section "The `>=` overwrite rule vs
/// `execute_file_operations` stable-sort", argues from those ranges that the
/// simulator and `execute_file_operations` pick the same winners, and names the
/// phase-2/phase-3 split as the exception.
///
/// ```text
/// doc_order counter, one increment per file entry, never per atom:
///
///   loop 1  required files          [0 .. R)
///   loop 2  plugin entries, normal  [R .. R+N)      flat_idx walks 0..P
///   loop 3  plugin entries, auto    [R+N .. R+N+A)  flat_idx restarts at 0
///   loop 4  conditional patterns    [R+N+A .. end)
///
///   one <folder> entry -> many atoms, all sharing that entry's doc_order
/// ```
///
/// A "normal" entry sets neither `always_install` nor `install_if_usable`; an
/// "auto" entry sets either. Loops 2 and 3 walk the same step/group/plugin
/// structure and write into the same `per_plugin[flat_idx]` buckets, so one
/// plugin's bucket holds its normal atoms first and its auto atoms after, even
/// though a later plugin's normal atoms carry a lower `document_order` than this
/// plugin's auto atoms.
///
/// These four loops are not the install replay's three passes in
/// [`crate::fomod_service`], nor the simulator's four phases in
/// [`crate::fomod_forward_simulator`]. Different numberings of different work.
pub fn expand_all_atoms(
    installer: &FomodInstaller,
    sorted_entries: &[String],
    entry_sizes: &HashMap<String, u64>,
) -> ExpandedAtoms {
    let mut result = ExpandedAtoms::default();
    let mut doc_order = 0i32;

    // Loop 1: required files.
    for entry in &installer.required_files {
        expand_entry(
            entry,
            sorted_entries,
            entry_sizes,
            doc_order,
            Origin::Required,
            -1,
            -1,
            &mut result.required,
        );
        doc_order += 1;
    }

    // per_plugin is sized before the walk, so loops 2 and 3 can index into it.
    result
        .per_plugin
        .resize_with(total_flat_plugins(installer) as usize, Vec::new);

    // Loop 2: normal plugin file entries (neither alwaysInstall nor
    // installIfUsable).
    let mut flat_idx = 0usize;
    for step in &installer.steps {
        for group in &step.groups {
            for plugin in &group.plugins {
                for entry in &plugin.files {
                    if !entry.always_install && !entry.install_if_usable {
                        expand_entry(
                            entry,
                            sorted_entries,
                            entry_sizes,
                            doc_order,
                            Origin::Plugin,
                            flat_idx as i32,
                            -1,
                            &mut result.per_plugin[flat_idx],
                        );
                        doc_order += 1;
                    }
                }
                flat_idx += 1;
            }
        }
    }

    // Loop 3: auto plugin file entries (alwaysInstall or installIfUsable), at a
    // higher doc_order. Same traversal as loop 2, with flat_idx restarted at 0
    // so the atoms land in the same per_plugin buckets.
    let mut flat_idx = 0usize;
    for step in &installer.steps {
        for group in &step.groups {
            for plugin in &group.plugins {
                for entry in &plugin.files {
                    if entry.always_install || entry.install_if_usable {
                        expand_entry(
                            entry,
                            sorted_entries,
                            entry_sizes,
                            doc_order,
                            Origin::Plugin,
                            flat_idx as i32,
                            -1,
                            &mut result.per_plugin[flat_idx],
                        );
                        doc_order += 1;
                    }
                }
                flat_idx += 1;
            }
        }
    }

    // Loop 4: conditional install patterns.
    result
        .per_conditional
        .resize_with(installer.conditional_patterns.len(), Vec::new);
    for (ci, pattern) in installer.conditional_patterns.iter().enumerate() {
        for entry in &pattern.files {
            expand_entry(
                entry,
                sorted_entries,
                entry_sizes,
                doc_order,
                Origin::Conditional,
                -1,
                ci as i32,
                &mut result.per_conditional[ci],
            );
            doc_order += 1;
        }
    }

    result
}

/// Group every atom by its destination path.
///
/// Iterates via [`ExpandedAtoms::for_each`] (required, then per_plugin in flat
/// order, then per_conditional), so each destination's `Vec` is in a
/// deterministic order.
///
/// No current consumer depends on that order. Conflict resolution does not read
/// this index: the simulator resolves over [`ExpandedAtoms`], and the real
/// installer resolves over its sorted `FileOperation` queue. The CSP precompute
/// sums per-plugin evidence and breaks on the first hash hit; the remaining
/// consumers insert into hash sets. Keep the order anyway, so an order-sensitive
/// consumer added later starts from a defined sequence rather than an arbitrary
/// one.
///
/// Map keys are the atoms' `dest_path` values, cloned as-is.
pub fn build_atom_index(atoms: &ExpandedAtoms) -> AtomIndex {
    let mut index = AtomIndex::new();
    atoms.for_each(|a| {
        index
            .entry(a.dest_path.clone())
            .or_default()
            .push(a.clone());
    });
    index
}

/// Find the destinations that carry no solver signal, so scoring can ignore
/// them.
///
/// A destination is excluded when every atom targeting it is an `always_install`
/// or `install_if_usable` plugin atom and all of them share one source path.
/// Such a destination looks the same under every selection, so it can neither
/// confirm nor rule anything out.
///
/// Two cases stay in play on purpose:
///
/// - A Required-origin atom keeps its destination in, which is how the solver
///   can still see an incomplete installation whose expected required files are
///   missing from the target.
/// - Any conditional-origin atom keeps its destination in, because which
///   conditionals fire depends on flags, and flags depend on plugin selections.
pub fn compute_excluded_dests(atom_index: &AtomIndex) -> HashSet<String> {
    let mut excluded = HashSet::new();
    for (dest, atoms) in atom_index {
        let mut all_auto = true;
        let mut has_conditional = false;
        let mut sources: HashSet<&str> = HashSet::new();
        for a in atoms {
            sources.insert(a.source_path.as_str());
            match a.origin {
                Origin::Conditional => {
                    has_conditional = true;
                }
                Origin::Required => {
                    // Required files always install, but keeping them in the
                    // comparison is what lets the solver detect an incomplete
                    // installation. Deliberate; do not fold into the auto case.
                    all_auto = false;
                }
                Origin::Plugin => {
                    if !a.always_install && !a.install_if_usable {
                        all_auto = false;
                    }
                }
            }
        }
        if has_conditional {
            continue;
        }
        // Exclude only when every atom is auto-installed and all share a source.
        if all_auto && sources.len() <= 1 {
            excluded.insert(dest.clone());
        }
    }
    excluded
}

/// Build a [`TargetTree`] from the files already installed in the mod directory.
///
/// One [`TargetFile`] per installed file, keyed by relative path and carrying
/// the size that later comparisons check candidate atoms against. Hashes stay 0
/// here and are filled in lazily, only for contested destinations.
///
/// A top-level `meta.ini` is skipped on an exact key match. It is MO2 metadata
/// and never part of a FOMOD installation; [`expand_entry`] drops the same
/// destination on the atom side so the two trees stay comparable.
pub fn build_target_tree(installed_files: &HashMap<String, u64>) -> TargetTree {
    let mut target = TargetTree::new();
    for (rel_path, &file_size) in installed_files {
        if rel_path == "meta.ini" {
            continue;
        }
        target.insert(
            rel_path.clone(),
            TargetFile {
                size: file_size,
                hash: 0,
            },
        );
    }
    target
}

// ---------------------------------------------------------------------------
// Assemble the schema-v2 JSON from a solver result plus diagnostics.
// ---------------------------------------------------------------------------

/// Maximum number of `outputTree` entries emitted before truncation.
const MAX_OUTPUT_TREE_ENTRIES: usize = 5000;

/// Build one plugin object. Always carries `name` and `selected`; `confidence`
/// and `reasons` ride along when a diagnostic record exists for this position.
fn build_plugin_object(name: &str, selected: bool, diag: Option<&PluginDiagnostics>) -> Value {
    let mut j = Value::object();
    j.insert("name", Value::string(name));
    j.insert("selected", Value::Bool(selected));
    if let Some(diag) = diag {
        j.insert("confidence", serialize_confidence(&diag.confidence));
        let mut reasons = Value::array();
        for r in &diag.reasons {
            reasons.push(serialize_reason(r));
        }
        j.insert("reasons", reasons);
    }
    j
}

/// Per-plugin diagnostic record, or `None` when any of the three indices is out
/// of range.
fn lookup_plugin_diag(
    diag: &InferenceDiagnostics,
    s: usize,
    g: usize,
    p: usize,
) -> Option<&PluginDiagnostics> {
    diag.steps
        .get(s)
        .and_then(|step| step.groups.get(g))
        .and_then(|group| group.plugins.get(p))
}

/// Per-group diagnostic record, or `None` when either index is out of range.
fn lookup_group_diag(diag: &InferenceDiagnostics, s: usize, g: usize) -> Option<&GroupDiagnostics> {
    diag.steps.get(s).and_then(|step| step.groups.get(g))
}

/// Per-step diagnostic record, or `None` when the index is out of range.
fn lookup_step_diag(diag: &InferenceDiagnostics, s: usize) -> Option<&StepDiagnostics> {
    diag.steps.get(s)
}

/// Turn a [`SolverResult`] plus its [`InferenceDiagnostics`] into the schema-v2
/// response object.
///
/// Walks the installer's step/group/plugin hierarchy and reads the solver's
/// `[step][group][plugin]` boolean grid to sort each plugin into `plugins`
/// (selected) or `deselected`. An out-of-bounds selection index logs a warning
/// and counts as deselected, so a short grid degrades instead of failing.
///
/// The returned object carries exactly three keys: `schema_version`, `steps` and
/// `diagnostics`. That is not the complete response. The pipeline then calls
/// [`add_output_tree`], which adds `outputTree` and, when capped,
/// `outputTreeTruncated` and `outputTreeTotal`; and [`add_repro_detail`], which
/// adds `reproDetail`. Both run on the normal path and on the Tier-1 `meta.ini`
/// cache path, so a consumer can rely on those keys being present.
pub fn assemble_json(
    installer: &FomodInstaller,
    result: &SolverResult,
    diagnostics: &InferenceDiagnostics,
) -> Value {
    let mut j_steps = Value::array();
    for (si, step) in installer.steps.iter().enumerate() {
        let mut j_step = Value::object();
        j_step.insert("name", Value::string(&step.name));

        if let Some(step_diag) = lookup_step_diag(diagnostics, si) {
            j_step.insert("confidence", serialize_confidence(&step_diag.confidence));
            j_step.insert("visible", Value::Bool(step_diag.visible));
            let mut reasons = Value::array();
            for r in &step_diag.reasons {
                reasons.push(serialize_reason(r));
            }
            j_step.insert("reasons", reasons);
        }

        let mut j_groups = Value::array();
        for (gi, group) in step.groups.iter().enumerate() {
            let mut j_group = Value::object();
            j_group.insert("name", Value::string(&group.name));

            if let Some(group_diag) = lookup_group_diag(diagnostics, si, gi) {
                j_group.insert("confidence", serialize_confidence(&group_diag.confidence));
                j_group.insert("resolved_by", Value::string(&group_diag.resolved_by));
                let mut reasons = Value::array();
                for r in &group_diag.reasons {
                    reasons.push(serialize_reason(r));
                }
                j_group.insert("reasons", reasons);
            }

            let mut j_selected = Value::array();
            let mut j_deselected = Value::array();
            for (pi, plugin) in group.plugins.iter().enumerate() {
                // The warning fires only on the out-of-bounds branch, so keep
                // the Option instead of collapsing to `unwrap_or(false)`.
                let sel = match result
                    .selections
                    .get(si)
                    .and_then(|g| g.get(gi))
                    .and_then(|p| p.get(pi))
                    .copied()
                {
                    Some(value) => value,
                    None => {
                        Logger::instance().log_warning(&format!(
                            "[infer] assemble_json: selection index out of bounds \
                             (step={si}, group={gi}, plugin={pi}), defaulting to false"
                        ));
                        false
                    }
                };
                let plugin_diag = lookup_plugin_diag(diagnostics, si, gi, pi);
                let j_plugin = build_plugin_object(&plugin.name, sel, plugin_diag);
                if sel {
                    j_selected.push(j_plugin);
                } else {
                    j_deselected.push(j_plugin);
                }
            }
            j_group.insert("plugins", j_selected);
            j_group.insert("deselected", j_deselected);
            j_groups.push(j_group);
        }
        j_step.insert("groups", j_groups);
        j_steps.push(j_step);
    }

    let mut out = Value::object();
    out.insert(
        "schema_version",
        Value::Int(diagnostics.schema_version as i64),
    );
    out.insert("steps", j_steps);
    out.insert("diagnostics", serialize_run_diagnostics(&diagnostics.run));
    out
}

/// Attach the inferred install's virtual output tree to `out` as a flat
/// `outputTree` array of `{path, size, source}`. `path` is the atom's
/// `dest_path`, `source` its archive entry path, and `size` its `file_size`,
/// where 0 means unknown rather than empty. The payload carries no separate
/// unknown marker, so a consumer that sums `size` under-counts.
///
/// The simulation's file map is unordered, so entries are sorted by destination
/// path in byte order, which is what makes two runs diffable. The array is
/// capped at [`MAX_OUTPUT_TREE_ENTRIES`]; when it is, the `outputTreeTruncated`
/// and `outputTreeTotal` siblings carry the flag and the full count, and a
/// warning goes to `logs/salma.log`.
///
/// `out` must be a [`Value::Object`]. Anything else panics.
pub fn add_output_tree(out: &mut Value, sim: &SimulatedTree) {
    let mut entries: Vec<&FomodAtom> = sim.files.values().collect();
    entries.sort_by(|a, b| a.dest_path.cmp(&b.dest_path));

    let total = entries.len();
    let truncated = total > MAX_OUTPUT_TREE_ENTRIES;
    if truncated {
        entries.truncate(MAX_OUTPUT_TREE_ENTRIES);
        Logger::instance().log_warning(&format!(
            "[infer] Output tree capped at {MAX_OUTPUT_TREE_ENTRIES} of {total} entries"
        ));
    }

    let mut tree = Value::array();
    for atom in entries {
        let mut entry = Value::object();
        entry.insert("path", Value::string(&atom.dest_path));
        entry.insert("size", Value::Int(atom.file_size as i64));
        entry.insert("source", Value::string(&atom.source_path));
        tree.push(entry);
    }
    out.insert("outputTree", tree);
    if truncated {
        out.insert("outputTreeTruncated", Value::Bool(true));
        out.insert("outputTreeTotal", Value::Int(total as i64));
    }
}

/// Maximum number of paths carried in `reproDetail` before truncation. Set to
/// [`MAX_OUTPUT_TREE_ENTRIES`] so one number bounds both payloads.
///
/// Sharing the number does not make the two payloads line up, and that is a
/// known limitation. `outputTree` truncates the simulated files sorted by path.
/// `reproDetail` truncates the union of diverging target destinations and extra
/// simulated destinations, also sorted by path, which is a different sequence.
/// So a path past this cap can still belong to a row that survived into the
/// capped `outputTree`, and that row then shows unmarked.
///
/// Worked example: 4000 missing destinations under `a/` plus 6000 extra
/// destinations under `z/` give a 10000-path classification whose 5000-path
/// prefix is 4000 `a/` entries and only 1000 `z/` entries, while `outputTree`
/// shows 5000 `z/` rows, 4000 of them visible and unmarked.
///
/// Treat the marks as a stable prefix, never as a complete set. Capping missing
/// and extra separately would remove the asymmetry, at the cost of two budgets.
const MAX_REPRO_DETAIL_PATHS: usize = MAX_OUTPUT_TREE_ENTRIES;

/// Add the `reproDetail` sibling: which destinations diverged, by category.
///
/// `diagnostics.repro` counts how many files were missing, extra or mismatched.
/// This names them, so a reader of the output tree can mark individual rows
/// instead of being handed a number. The bucket keys are the same four the
/// counter object uses, so the tally and the paths read as one vocabulary.
///
/// Emitted unconditionally, with empty buckets on a clean run, so a consumer can
/// read "key present, all four buckets empty" as "this run reproduced
/// everything" without checking a second signal.
///
/// The `reproDetail` object holds four bucket keys, one per [`DestStatus`]
/// value, and up to two more when the list is capped:
///
/// | key              | value  | present when                     |
/// |------------------|--------|----------------------------------|
/// | `missing`        | array  | always                           |
/// | `extra`          | array  | always                           |
/// | `size_mismatch`  | array  | always                           |
/// | `hash_mismatch`  | array  | always                           |
/// | `truncated`      | `true` | `classified` is longer than [`MAX_REPRO_DETAIL_PATHS`] |
/// | `total`          | number | same condition as `truncated`    |
///
/// The keys serialize in sorted order, because [`Value::Object`] is a sorted
/// map. Inside each bucket the paths keep the order of `classified`.
///
/// The two truncation markers sit inside `reproDetail`, while
/// `outputTreeTruncated` and `outputTreeTotal` are siblings of `outputTree`. A
/// consumer reading the markers has to look in a different place for each
/// payload.
///
/// `classified` is expected sorted by path, as
/// [`classify_dests`](crate::fomod_forward_simulator::classify_dests) returns it,
/// so truncation keeps a stable prefix rather than an arbitrary sample.
///
/// `out` must be a [`Value::Object`]. Anything else panics, the same contract
/// [`add_output_tree`] carries. Truncating writes a warning line to
/// `logs/salma.log`.
pub fn add_repro_detail(out: &mut Value, classified: &[(String, DestStatus)]) {
    let total = classified.len();
    let truncated = total > MAX_REPRO_DETAIL_PATHS;
    if truncated {
        Logger::instance().log_warning(&format!(
            "[infer] Repro detail capped at {MAX_REPRO_DETAIL_PATHS} of {total} paths"
        ));
    }

    let mut buckets = Value::object();
    for status in [
        DestStatus::Missing,
        DestStatus::Extra,
        DestStatus::SizeMismatch,
        DestStatus::HashMismatch,
    ] {
        let mut arr = Value::array();
        for (dest, st) in classified.iter().take(MAX_REPRO_DETAIL_PATHS) {
            if *st == status {
                arr.push(Value::string(dest));
            }
        }
        buckets.insert(status.as_str(), arr);
    }
    if truncated {
        buckets.insert("truncated", Value::Bool(true));
        buckets.insert("total", Value::Int(total as i64));
    }
    out.insert("reproDetail", buckets);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_ir::{
        FomodConditionalPattern, FomodGroup, FomodPlugin, FomodStep, compute_flat_starts,
    };

    // --- helpers -----------------------------------------------------------

    fn entries(paths: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    fn sizes(pairs: &[(&str, u64)]) -> HashMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn folder(source: &str, destination: &str) -> FomodFileEntry {
        FomodFileEntry {
            source: source.to_string(),
            destination: destination.to_string(),
            is_folder: true,
            ..FomodFileEntry::default()
        }
    }

    fn file(source: &str, destination: &str) -> FomodFileEntry {
        FomodFileEntry {
            source: source.to_string(),
            destination: destination.to_string(),
            ..FomodFileEntry::default()
        }
    }

    fn expand_one(entry: &FomodFileEntry, sorted: &[String]) -> Vec<FomodAtom> {
        let mut out = Vec::new();
        expand_entry(
            entry,
            sorted,
            &HashMap::new(),
            7,
            Origin::Plugin,
            3,
            -1,
            &mut out,
        );
        out
    }

    fn dests(atoms: &[FomodAtom]) -> Vec<&str> {
        atoms.iter().map(|a| a.dest_path.as_str()).collect()
    }

    // --- expand_entry: folder branch ----------------------------------------

    #[test]
    fn folder_prefix_requires_path_boundary() {
        // "foo" must not match "foobar.esp"; the trailing slash anchors it.
        let sorted = entries(&["foo/inside.txt", "foobar.esp", "foo.txt"]);
        let atoms = expand_one(&folder("foo", "out"), &sorted);
        assert_eq!(dests(&atoms), ["out/inside.txt"]);
        assert_eq!(atoms[0].source_path, "foo/inside.txt");
    }

    #[test]
    fn folder_source_with_trailing_slash_is_not_doubled() {
        let sorted = entries(&["foo/inside.txt"]);
        let atoms = expand_one(&folder("foo/", "out"), &sorted);
        assert_eq!(dests(&atoms), ["out/inside.txt"]);
    }

    #[test]
    fn empty_folder_source_matches_every_entry() {
        let sorted = entries(&["a.txt", "b/c.txt", "z.esp"]);
        let atoms = expand_one(&folder("", "out"), &sorted);
        assert_eq!(dests(&atoms), ["out/a.txt", "out/b/c.txt", "out/z.esp"]);
    }

    #[test]
    fn empty_destination_places_rel_at_root() {
        let sorted = entries(&["src/a.txt", "src/sub/b.txt"]);
        let atoms = expand_one(&folder("src", ""), &sorted);
        assert_eq!(dests(&atoms), ["a.txt", "sub/b.txt"]);
    }

    #[test]
    fn destination_joins_with_slash_then_normalizes() {
        // Destination with trailing slash: raw concat "dest//sub/a.txt"
        // normalizes to "dest/sub/a.txt".
        let sorted = entries(&["src/sub/a.txt"]);
        let atoms = expand_one(&folder("src", "dest/"), &sorted);
        assert_eq!(dests(&atoms), ["dest/sub/a.txt"]);
        // normalize_path lowercases the destination; source_path keeps the
        // sorted entry string unchanged.
        let sorted2 = entries(&["src/A.txt"]);
        let atoms2 = expand_one(&folder("src", "Dest"), &sorted2);
        assert_eq!(dests(&atoms2), ["dest/a.txt"]);
        assert_eq!(atoms2[0].source_path, "src/A.txt");
    }

    #[test]
    fn folder_branch_skips_top_level_meta_ini_only() {
        let sorted = entries(&["src/meta.ini", "src/sub/meta.ini", "src/a.txt"]);
        let atoms = expand_one(&folder("src", ""), &sorted);
        // Top-level meta.ini skipped; nested sub/meta.ini kept.
        assert_eq!(dests(&atoms), ["a.txt", "sub/meta.ini"]);
    }

    #[test]
    fn folder_branch_skips_unsafe_destinations() {
        // A drive-letter destination survives normalize_path and fails
        // is_safe_dest ("c:/evil/a.txt").
        let sorted = entries(&["src/a.txt"]);
        let atoms = expand_one(&folder("src", "C:/evil"), &sorted);
        assert!(atoms.is_empty());
    }

    #[test]
    fn folder_atom_fields_are_populated_from_entry_and_args() {
        let sorted = entries(&["src/a.txt"]);
        let entry = FomodFileEntry {
            source: "src".to_string(),
            destination: "d".to_string(),
            priority: 5,
            is_folder: true,
            always_install: true,
            install_if_usable: false,
        };
        let mut out = Vec::new();
        expand_entry(
            &entry,
            &sorted,
            &sizes(&[("src/a.txt", 1234)]),
            9,
            Origin::Conditional,
            -1,
            2,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        let a = &out[0];
        assert_eq!(a.source_path, "src/a.txt");
        assert_eq!(a.dest_path, "d/a.txt");
        assert_eq!(a.priority, 5);
        assert_eq!(a.document_order, 9);
        assert_eq!(a.origin, Origin::Conditional);
        assert_eq!(a.plugin_index, -1);
        assert_eq!(a.conditional_index, 2);
        assert!(a.always_install);
        assert!(!a.install_if_usable);
        assert_eq!(a.file_size, 1234);
        assert_eq!(a.content_hash, 0);
    }

    #[test]
    fn file_size_defaults_to_zero_when_missing_from_map() {
        let sorted = entries(&["src/a.txt"]);
        let atoms = expand_one(&folder("src", ""), &sorted);
        assert_eq!(atoms[0].file_size, 0);
    }

    // --- expand_entry: file branch -------------------------------------------

    #[test]
    fn file_branch_keeps_destination_unchanged() {
        // The file branch does not re-normalize dest_path, because the parser
        // already normalized it; is_safe_dest runs on the raw destination.
        let entry = file("src/a.esp", "sub/a.esp");
        let mut out = Vec::new();
        expand_entry(
            &entry,
            &entries(&[]),
            &sizes(&[("src/a.esp", 77)]),
            0,
            Origin::Required,
            -1,
            -1,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source_path, "src/a.esp");
        assert_eq!(out[0].dest_path, "sub/a.esp");
        assert_eq!(out[0].file_size, 77);
    }

    #[test]
    fn file_branch_skips_meta_ini_and_unsafe_destinations() {
        let mut out = Vec::new();
        // meta.ini (post-normalization match: "Meta.INI" normalizes to it).
        expand_entry(
            &file("src/meta.ini", "Meta.INI"),
            &entries(&[]),
            &HashMap::new(),
            0,
            Origin::Required,
            -1,
            -1,
            &mut out,
        );
        assert!(out.is_empty());
        // Unsafe destination (drive letter).
        expand_entry(
            &file("src/evil.txt", "C:/evil.txt"),
            &entries(&[]),
            &HashMap::new(),
            0,
            Origin::Required,
            -1,
            -1,
            &mut out,
        );
        assert!(out.is_empty());
        // normalize_path drops "..", so "../evil" normalizes to "evil" and
        // passes the safety check. The file branch checks the raw destination,
        // and is_safe_destination normalizes internally too, so it is accepted.
        // Pins the raw/normalized split: the atom keeps the raw string.
        expand_entry(
            &file("src/x.txt", "../evil"),
            &entries(&[]),
            &HashMap::new(),
            0,
            Origin::Required,
            -1,
            -1,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].dest_path, "../evil");
    }

    // --- expand_all_atoms: pass order and doc_order ranges --------------------

    /// Installer shape:
    ///   required_files: 1 folder entry (2 matched files)
    ///   step0/group0: plugin0 (1 normal entry), plugin1 (1 normal + 1 always)
    ///   step1/group0: plugin2 (1 installIfUsable entry)
    ///   conditional_patterns: 1 pattern with 2 entries
    fn sample_installer() -> FomodInstaller {
        let plugin0 = FomodPlugin {
            name: "P0".to_string(),
            files: vec![file("arc/p0.esp", "p0.esp")],
            ..FomodPlugin::default()
        };
        let plugin1 = FomodPlugin {
            name: "P1".to_string(),
            files: vec![
                file("arc/p1.esp", "p1.esp"),
                FomodFileEntry {
                    always_install: true,
                    ..file("arc/p1_auto.txt", "p1_auto.txt")
                },
            ],
            ..FomodPlugin::default()
        };
        let plugin2 = FomodPlugin {
            name: "P2".to_string(),
            files: vec![FomodFileEntry {
                install_if_usable: true,
                ..file("arc/p2_usable.txt", "p2_usable.txt")
            }],
            ..FomodPlugin::default()
        };
        FomodInstaller {
            required_files: vec![folder("req", "")],
            steps: vec![
                FomodStep {
                    groups: vec![FomodGroup {
                        plugins: vec![plugin0, plugin1],
                        ..FomodGroup::default()
                    }],
                    ..FomodStep::default()
                },
                FomodStep {
                    groups: vec![FomodGroup {
                        plugins: vec![plugin2],
                        ..FomodGroup::default()
                    }],
                    ..FomodStep::default()
                },
            ],
            conditional_patterns: vec![FomodConditionalPattern {
                files: vec![file("arc/c0.txt", "c0.txt"), folder("cond", "cd")],
                ..FomodConditionalPattern::default()
            }],
            ..FomodInstaller::default()
        }
    }

    fn sample_entries() -> Vec<String> {
        entries(&[
            "req/a.txt",
            "req/b.txt",
            "arc/p0.esp",
            "arc/p1.esp",
            "arc/p1_auto.txt",
            "arc/p2_usable.txt",
            "arc/c0.txt",
            "cond/x.txt",
            "cond/y.txt",
        ])
    }

    #[test]
    fn expand_all_atoms_three_pass_doc_order_and_flat_indices() {
        let installer = sample_installer();
        let atoms = expand_all_atoms(&installer, &sample_entries(), &HashMap::new());

        // Required: one folder entry -> doc_order 0 shared by both atoms.
        assert_eq!(atoms.required.len(), 2);
        assert!(atoms.required.iter().all(|a| a.document_order == 0));
        assert!(atoms.required.iter().all(|a| a.origin == Origin::Required));
        assert!(atoms.required.iter().all(|a| a.plugin_index == -1));
        assert!(atoms.required.iter().all(|a| a.conditional_index == -1));
        assert_eq!(dests(&atoms.required), ["a.txt", "b.txt"]);

        // per_plugin is sized to total_flat_plugins before the walk.
        assert_eq!(atoms.per_plugin.len(), 3);
        assert_eq!(compute_flat_starts(&installer), vec![vec![0], vec![2]]);

        // Loop 2 (normal plugin entries): plugin0 doc 1, plugin1 doc 2.
        // Loop 3 (auto plugin entries):   plugin1 doc 3, plugin2 doc 4.
        // Loop 4 (conditionals):          pattern0 entries doc 5 and 6.
        let p0 = &atoms.per_plugin[0];
        assert_eq!(p0.len(), 1);
        assert_eq!(p0[0].document_order, 1);
        assert_eq!(p0[0].plugin_index, 0);
        assert_eq!(p0[0].origin, Origin::Plugin);

        let p1 = &atoms.per_plugin[1];
        assert_eq!(p1.len(), 2);
        // Normal entry first (loop 2), auto entry second (loop 3).
        assert_eq!(p1[0].dest_path, "p1.esp");
        assert_eq!(p1[0].document_order, 2);
        assert!(!p1[0].always_install);
        assert_eq!(p1[1].dest_path, "p1_auto.txt");
        assert_eq!(p1[1].document_order, 3);
        assert!(p1[1].always_install);
        assert!(p1.iter().all(|a| a.plugin_index == 1));

        let p2 = &atoms.per_plugin[2];
        assert_eq!(p2.len(), 1);
        assert_eq!(p2[0].document_order, 4);
        assert_eq!(p2[0].plugin_index, 2);
        assert!(p2[0].install_if_usable);

        // Conditionals: entry-level doc_order 5 (file) and 6 (folder, shared
        // by both folder atoms).
        assert_eq!(atoms.per_conditional.len(), 1);
        let c0 = &atoms.per_conditional[0];
        assert_eq!(c0.len(), 3);
        assert_eq!(c0[0].dest_path, "c0.txt");
        assert_eq!(c0[0].document_order, 5);
        assert_eq!(dests(&c0[1..]), ["cd/x.txt", "cd/y.txt"]);
        assert!(c0[1..].iter().all(|a| a.document_order == 6));
        assert!(c0.iter().all(|a| a.origin == Origin::Conditional));
        assert!(c0.iter().all(|a| a.plugin_index == -1));
        assert!(c0.iter().all(|a| a.conditional_index == 0));

        // Range ordering: required < normal plugin < auto plugin <
        // conditional.
        let max_required = atoms
            .required
            .iter()
            .map(|a| a.document_order)
            .max()
            .unwrap();
        assert!(max_required < p0[0].document_order);
        assert!(p1[0].document_order < p1[1].document_order);
        assert!(p2[0].document_order < c0[0].document_order);
    }

    #[test]
    fn expand_all_atoms_flat_idx_spans_steps_and_groups() {
        // 2 steps x 2 groups x varying plugins: flat_idx must walk
        // step-major, group-next, plugin-innermost.
        let mk_plugin = |name: &str, src: &str| FomodPlugin {
            name: name.to_string(),
            files: vec![file(src, &format!("{name}.out"))],
            ..FomodPlugin::default()
        };
        let installer = FomodInstaller {
            steps: vec![
                FomodStep {
                    groups: vec![
                        FomodGroup {
                            plugins: vec![mk_plugin("a", "s/a"), mk_plugin("b", "s/b")],
                            ..FomodGroup::default()
                        },
                        FomodGroup {
                            plugins: vec![mk_plugin("c", "s/c")],
                            ..FomodGroup::default()
                        },
                    ],
                    ..FomodStep::default()
                },
                FomodStep {
                    groups: vec![FomodGroup {
                        plugins: vec![mk_plugin("d", "s/d")],
                        ..FomodGroup::default()
                    }],
                    ..FomodStep::default()
                },
            ],
            ..FomodInstaller::default()
        };
        let atoms = expand_all_atoms(&installer, &entries(&[]), &HashMap::new());
        assert_eq!(atoms.per_plugin.len(), 4);
        for (flat, bucket) in atoms.per_plugin.iter().enumerate() {
            assert_eq!(bucket.len(), 1, "plugin {flat}");
            assert_eq!(bucket[0].plugin_index, flat as i32);
        }
        // Document order follows the same traversal (all normal entries).
        let orders: Vec<i32> = atoms
            .per_plugin
            .iter()
            .map(|b| b[0].document_order)
            .collect();
        assert_eq!(orders, [0, 1, 2, 3]);
    }

    // --- build_atom_index ------------------------------------------------------

    #[test]
    fn build_atom_index_groups_by_dest_preserving_for_each_order() {
        let installer = sample_installer();
        let atoms = expand_all_atoms(&installer, &sample_entries(), &HashMap::new());
        let index = build_atom_index(&atoms);
        // 2 required + p0.esp + p1.esp + p1_auto + p2_usable + c0 + 2 cond
        // folder atoms = 9 distinct destinations here.
        assert_eq!(index.len(), 9);
        assert_eq!(index["a.txt"].len(), 1);
        assert_eq!(index["p1.esp"][0].plugin_index, 1);

        // Order preservation within one dest: craft a conflict.
        let conflicted = ExpandedAtoms {
            required: vec![FomodAtom {
                dest_path: "same.txt".to_string(),
                source_path: "r".to_string(),
                ..FomodAtom::default()
            }],
            per_plugin: vec![vec![FomodAtom {
                dest_path: "same.txt".to_string(),
                source_path: "p".to_string(),
                origin: Origin::Plugin,
                ..FomodAtom::default()
            }]],
            per_conditional: vec![vec![FomodAtom {
                dest_path: "same.txt".to_string(),
                source_path: "c".to_string(),
                origin: Origin::Conditional,
                ..FomodAtom::default()
            }]],
        };
        let idx = build_atom_index(&conflicted);
        assert_eq!(idx.len(), 1);
        let sources: Vec<&str> = idx["same.txt"]
            .iter()
            .map(|a| a.source_path.as_str())
            .collect();
        assert_eq!(sources, ["r", "p", "c"]);
    }

    // --- compute_excluded_dests -------------------------------------------------

    fn atom_with(
        dest: &str,
        source: &str,
        origin: Origin,
        always: bool,
        usable: bool,
    ) -> FomodAtom {
        FomodAtom {
            dest_path: dest.to_string(),
            source_path: source.to_string(),
            origin,
            always_install: always,
            install_if_usable: usable,
            ..FomodAtom::default()
        }
    }

    #[test]
    fn compute_excluded_dests_truth_table() {
        let mut index = AtomIndex::new();
        // All-auto, single source -> excluded.
        index.insert(
            "auto_single".to_string(),
            vec![
                atom_with("auto_single", "s1", Origin::Plugin, true, false),
                atom_with("auto_single", "s1", Origin::Plugin, false, true),
            ],
        );
        // All-auto but two distinct sources -> not excluded.
        index.insert(
            "auto_two_sources".to_string(),
            vec![
                atom_with("auto_two_sources", "s1", Origin::Plugin, true, false),
                atom_with("auto_two_sources", "s2", Origin::Plugin, true, false),
            ],
        );
        // Required atom present -> not excluded (all_auto forced false).
        index.insert(
            "required_present".to_string(),
            vec![
                atom_with("required_present", "s1", Origin::Required, false, false),
                atom_with("required_present", "s1", Origin::Plugin, true, false),
            ],
        );
        // Normal (non-auto) plugin atom -> not excluded.
        index.insert(
            "normal_plugin".to_string(),
            vec![atom_with(
                "normal_plugin",
                "s1",
                Origin::Plugin,
                false,
                false,
            )],
        );
        // Conditional atom present -> never excluded, even when everything
        // else is auto and single-source.
        index.insert(
            "conditional_present".to_string(),
            vec![
                atom_with("conditional_present", "s1", Origin::Plugin, true, false),
                atom_with(
                    "conditional_present",
                    "s1",
                    Origin::Conditional,
                    false,
                    false,
                ),
            ],
        );
        // Conditional-only dest -> also never excluded (has_conditional
        // short-circuits before the all_auto check; conditional atoms do not
        // touch all_auto).
        index.insert(
            "conditional_only".to_string(),
            vec![atom_with(
                "conditional_only",
                "s1",
                Origin::Conditional,
                false,
                false,
            )],
        );

        let excluded = compute_excluded_dests(&index);
        assert!(excluded.contains("auto_single"));
        assert!(!excluded.contains("auto_two_sources"));
        assert!(!excluded.contains("required_present"));
        assert!(!excluded.contains("normal_plugin"));
        assert!(!excluded.contains("conditional_present"));
        assert!(!excluded.contains("conditional_only"));
        assert_eq!(excluded.len(), 1);
    }

    // --- build_target_tree --------------------------------------------------------

    #[test]
    fn build_target_tree_skips_top_level_meta_ini_exactly() {
        let installed = sizes(&[
            ("meta.ini", 100),
            ("sub/meta.ini", 200),
            ("textures/a.dds", 300),
        ]);
        let tree = build_target_tree(&installed);
        assert_eq!(tree.len(), 2);
        assert!(!tree.contains_key("meta.ini"));
        assert_eq!(tree["sub/meta.ini"], TargetFile { size: 200, hash: 0 });
        assert_eq!(tree["textures/a.dds"], TargetFile { size: 300, hash: 0 });
    }

    // --- is_safe_dest delegate -----------------------------------------------------

    #[test]
    fn is_safe_dest_delegates_to_is_safe_destination() {
        assert!(is_safe_dest("textures/file.dds"));
        assert!(is_safe_dest(""));
        assert!(!is_safe_dest("C:/evil"));
        // ".." normalizes to empty -> safe, same as utils::is_safe_destination.
        assert!(is_safe_dest(".."));
    }

    // --- reproDetail ---------------------------------------------------------------

    /// The wire contract the UI reads: four buckets, keyed by the same names
    /// `diagnostics.repro` counts under, each dest in exactly one bucket.
    #[test]
    fn add_repro_detail_buckets_by_category() {
        let classified = vec![
            ("a/extra.dds".to_string(), DestStatus::Extra),
            ("b/hash.dds".to_string(), DestStatus::HashMismatch),
            ("c/gone.dds".to_string(), DestStatus::Missing),
            ("d/size.dds".to_string(), DestStatus::SizeMismatch),
        ];
        let mut out = Value::object();
        add_repro_detail(&mut out, &classified);

        let d = out.get("reproDetail").expect("reproDetail emitted");
        let bucket = |k: &str| match d.get(k) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>(),
            other => panic!("{k} should be an array, got {other:?}"),
        };
        assert_eq!(bucket("missing"), vec!["c/gone.dds".to_string()]);
        assert_eq!(bucket("extra"), vec!["a/extra.dds".to_string()]);
        assert_eq!(bucket("size_mismatch"), vec!["d/size.dds".to_string()]);
        assert_eq!(bucket("hash_mismatch"), vec!["b/hash.dds".to_string()]);
        // Not truncated, so neither marker rides along.
        assert!(d.get("truncated").is_none());
        assert!(d.get("total").is_none());
    }

    /// A clean run still emits the key with empty buckets, so a consumer can
    /// read "present and empty" as "reproduced everything" without a second
    /// signal to check.
    #[test]
    fn add_repro_detail_emits_empty_buckets_on_a_clean_run() {
        let mut out = Value::object();
        add_repro_detail(&mut out, &[]);
        let d = out.get("reproDetail").expect("reproDetail emitted");
        for k in ["missing", "extra", "size_mismatch", "hash_mismatch"] {
            match d.get(k) {
                Some(Value::Array(items)) => assert!(items.is_empty(), "{k} should be empty"),
                other => panic!("{k} should be an array, got {other:?}"),
            }
        }
    }
}
