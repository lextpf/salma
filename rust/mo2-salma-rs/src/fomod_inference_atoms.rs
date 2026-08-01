//! Atom expansion for inference - Rust port of `src/FomodInferenceAtoms.hpp`
//! / `.cpp`, plus the schema-v2 [`assemble_json`] and its anonymous-namespace
//! helpers (`build_plugin_object`, `lookup_*_diag`) added in Task 10, and
//! [`add_output_tree`] (whose C++ home is `FomodInferenceService.cpp`; it is
//! ported here so this module owns the full byte oracle - see PARITY-NOTES
//! "Task 10").
//!
//! Resolves IR file entries into concrete [`FomodAtom`]s by matching against
//! the archive entry list, builds the destination index and exclusion set the
//! solver scores against, builds the [`TargetTree`] from the installed files,
//! and assembles the schema-v2 inference response. The C++ `log_warning` call
//! sites (unsafe destinations, out-of-bounds selection indices, output-tree
//! cap) are reproduced verbatim.

use std::collections::{HashMap, HashSet};

use crate::fomod_atom::{AtomIndex, ExpandedAtoms, FomodAtom, Origin, TargetFile, TargetTree};
use crate::fomod_csp_types::SolverResult;
use crate::fomod_forward_simulator::SimulatedTree;
use crate::fomod_ir::{FomodFileEntry, FomodInstaller, total_flat_plugins};
use crate::inference_diagnostics::{
    GroupDiagnostics, InferenceDiagnostics, PluginDiagnostics, StepDiagnostics,
    serialize_confidence, serialize_reason, serialize_run_diagnostics,
};
use crate::json::Value;
use crate::logger::Logger;
use crate::utils::{is_safe_destination, normalize_path};

/// Inference-side wrapper for path-traversal validation. Thin forwarder to
/// [`is_safe_destination`] so call sites in the inference pipeline read
/// symmetrically with the FomodService pipeline (mirror of
/// `mo2core::is_safe_dest`).
pub fn is_safe_dest(dest: &str) -> bool {
    is_safe_destination(dest)
}

/// Expand a single [`FomodFileEntry`] into concrete [`FomodAtom`]s by
/// matching against archive entries. Mirror of `mo2core::expand_entry`.
///
/// For folder entries, performs a prefix search over `sorted_entries` (which
/// must be lexicographically sorted) to find all archive members under the
/// source directory, producing one atom per match. For single-file entries,
/// produces exactly one atom. Unsafe destinations (path traversal) are
/// skipped with a warning. A top-level
/// `meta.ini` destination is skipped as well, mirroring
/// [`build_target_tree`]'s exclusion of the installed-side MO2 metadata file.
///
/// All atoms produced by ONE call share the same `doc_order` value; the
/// caller increments it per file-entry expansion, not per atom.
#[allow(clippy::too_many_arguments)] // one-to-one with the C++ signature
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
        // An empty source means "root of archive" - match every entry.
        //
        // The combination below relies on two empty-string identities to fall
        // out without a special-case branch in the loop:
        //   * partition_point over "" returns 0 because "" is
        //     lexicographically less-than-or-equal to any other string.
        //   * str::starts_with("") is unconditionally true.
        // So the loop iterates every entry exactly once.
        //
        // For a non-root folder the prefix is anchored with a trailing slash
        // so that a folder entry "foo" does NOT spuriously match a sibling
        // file "foobar.esp" - the slash forces a path-boundary match.
        let mut prefix = entry.source.clone();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }

        // C++ std::lower_bound: first element >= prefix.
        let start = sorted_entries.partition_point(|e| e.as_str() < prefix.as_str());
        for entry_path in &sorted_entries[start..] {
            if !entry_path.starts_with(&prefix) {
                break;
            }
            let rel = &entry_path[prefix.len()..];
            // Raw concatenation BEFORE normalization, exactly as in C++.
            let dest = if entry.destination.is_empty() {
                rel.to_string()
            } else {
                format!("{}/{}", entry.destination, rel)
            };

            let norm_dest = normalize_path(&dest);
            // Skip archive-shipped top-level meta.ini: build_target_tree
            // excludes the installed one as MO2 metadata, so producing it
            // here would make exact_match permanently unreachable.
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
        // normalizes destinations already; normalize again for direct
        // callers - but note the atom's dest_path stays the RAW
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

/// Expand all FOMOD file entries into atoms using three ordered passes.
/// Mirror of `mo2core::expand_all_atoms`.
///
/// Three separate passes are intentional: they encode different
/// document_order ranges to ensure correct priority semantics in the FOMOD
/// spec:
///
/// - Pass 1: required files (lowest document_order range)
/// - Pass 2: normal plugin files, then always-install/installIfUsable plugin
///   files (middle range, auto entries after normal ones)
/// - Pass 3: conditional install patterns (highest range)
///
/// `doc_order` is a single counter incremented per file-entry expansion call
/// (all atoms from one folder entry share one document_order). Merging the
/// passes into a single loop would break the document_order invariant.
pub fn expand_all_atoms(
    installer: &FomodInstaller,
    sorted_entries: &[String],
    entry_sizes: &HashMap<String, u64>,
) -> ExpandedAtoms {
    let mut result = ExpandedAtoms::default();
    let mut doc_order = 0i32;

    // Required files.
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

    // Count total plugins; per_plugin is sized BEFORE the walk.
    result
        .per_plugin
        .resize_with(total_flat_plugins(installer) as usize, Vec::new);

    // Pass 1: normal (non-always) plugin file entries.
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

    // Pass 2: always-install and installIfUsable entries (higher doc_order),
    // SAME traversal with flat_idx recomputed from 0.
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

    // Conditional patterns.
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

/// Build an index that groups all atoms by their destination path. Mirror of
/// `mo2core::build_atom_index`.
///
/// Iterates every atom via [`ExpandedAtoms::for_each`] (required, then
/// per_plugin in flat order, then per_conditional), so the per-destination
/// `Vec` order matches the C++ exactly - downstream conflict resolution
/// depends on it.
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

/// Identify destinations that are only targeted by auto-install atoms and
/// should be excluded from solver scoring. Mirror of
/// `mo2core::compute_excluded_dests`.
///
/// A destination is excluded when every atom targeting it is an
/// always_install/install_if_usable plugin atom AND all atoms originate from
/// the same source path. Required-origin atoms keep the destination in play
/// (they let the solver detect incomplete installations where expected
/// Required files are missing from the target). Destinations with any
/// conditional-origin atom are never excluded, because which conditionals
/// fire depends on flags, which depend on plugin selections - they carry
/// solver signal.
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
                    // Required files are always installed, but keeping them
                    // in the comparison lets the solver detect incomplete
                    // installations (intentional, see the C++ comment).
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
        // Exclude only if all atoms are auto-installed AND all from the same
        // source.
        if all_auto && sources.len() <= 1 {
            excluded.insert(dest.clone());
        }
    }
    excluded
}

/// Build a [`TargetTree`] from the files already installed in the mod
/// directory. Mirror of `mo2core::build_target_tree`.
///
/// Creates a [`TargetFile`] entry for each installed file (keyed by relative
/// path), recording its size for later comparison against candidate atoms.
/// The MO2 metadata file `meta.ini` (top-level, exact key) is skipped since
/// it is never part of FOMOD installations.
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
// Assemble the schema-v2 JSON from a solver result + diagnostics
// (mirror of `assemble_json` and its anonymous-namespace helpers in
// `src/FomodInferenceAtoms.cpp:306-467`).
// ---------------------------------------------------------------------------

/// Maximum number of `outputTree` entries emitted before truncation. Mirror of
/// the C++ `kMaxOutputTreeEntries` (`src/FomodInferenceService.cpp:470`).
const MAX_OUTPUT_TREE_ENTRIES: usize = 5000;

/// Build a plugin JSON object from name + diagnostic fields. Mirror of the C++
/// `build_plugin_object`. Always carries `name` and `selected`; `confidence` and
/// `reasons` ride along when the diagnostic record exists for this position.
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

/// Look up the per-plugin diagnostic record, or `None` if any index is out of
/// range. Mirror of the C++ `lookup_plugin_diag`.
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

/// Look up the per-group diagnostic record. Mirror of `lookup_group_diag`.
fn lookup_group_diag(diag: &InferenceDiagnostics, s: usize, g: usize) -> Option<&GroupDiagnostics> {
    diag.steps.get(s).and_then(|step| step.groups.get(g))
}

/// Look up the per-step diagnostic record. Mirror of `lookup_step_diag`.
fn lookup_step_diag(diag: &InferenceDiagnostics, s: usize) -> Option<&StepDiagnostics> {
    diag.steps.get(s)
}

/// Convert a [`SolverResult`] + [`InferenceDiagnostics`] into the schema-v2 JSON
/// response object. Mirror of `mo2core::assemble_json`.
///
/// Walks the installer's step/group/plugin hierarchy and cross-references the
/// solver's 3-D boolean selection grid to classify each plugin as selected or
/// deselected. Out-of-bounds selection indices log a warning and default to
/// `false` (deselected).
///
/// The returned object carries `schema_version`, `steps`, and `diagnostics`;
/// [`add_output_tree`] adds the `outputTree` sibling afterward (its C++ home is
/// `FomodInferenceService.cpp`, but it is ported here so this module owns the
/// full byte oracle - see PARITY-NOTES "Task 10").
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
                // The C++ warns only on the out-of-bounds branch, so the lookup
                // keeps the Option instead of collapsing to `unwrap_or(false)`.
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

/// Attach the inferred install's virtual output tree to `out` as a flat,
/// path-sorted `outputTree` array of `{path, size, source}`. Mirror of the C++
/// `add_output_tree` (`src/FomodInferenceService.cpp:468-503`).
///
/// The simulation's file map is unordered, so entries are sorted by destination
/// path (byte order) for a stable diff. Large trees are capped at
/// [`MAX_OUTPUT_TREE_ENTRIES`]; when capped, the `outputTreeTruncated` /
/// `outputTreeTotal` siblings record the full count. `out` must be a
/// [`Value::Object`] (panics otherwise, mirroring the C++ `json&` contract).
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
        // "foo" must NOT match "foobar.esp"; the trailing slash anchors it.
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
        // Uppercase destination is lowercased by normalize_path; the
        // source_path stays the sorted entry string AS-IS.
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
        // The file branch does NOT re-normalize dest_path (the parser already
        // normalized it); is_safe_dest runs on the raw destination.
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
        // "../evil" normalizes to "evil" (normalize_path drops ".."), which
        // IS safe post-normalization - but the file branch checks the RAW
        // destination, and is_safe_destination("../evil") normalizes
        // internally too, so it is accepted. Pin the raw/normalized split:
        // the atom keeps the raw destination string.
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

        // per_plugin sized to total_flat_plugins BEFORE the walk.
        assert_eq!(atoms.per_plugin.len(), 3);
        assert_eq!(compute_flat_starts(&installer), vec![vec![0], vec![2]]);

        // Pass 1 (normal entries): plugin0 doc 1, plugin1 doc 2.
        // Pass 2 (auto entries):   plugin1 doc 3, plugin2 doc 4.
        // Pass 3 (conditionals):   pattern0 entries doc 5 and 6.
        let p0 = &atoms.per_plugin[0];
        assert_eq!(p0.len(), 1);
        assert_eq!(p0[0].document_order, 1);
        assert_eq!(p0[0].plugin_index, 0);
        assert_eq!(p0[0].origin, Origin::Plugin);

        let p1 = &atoms.per_plugin[1];
        assert_eq!(p1.len(), 2);
        // Normal entry first (pass 1), auto entry second (pass 2).
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
}
