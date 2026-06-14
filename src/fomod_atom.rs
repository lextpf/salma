//! Atom datatypes.
//!
//! An atom is one file-install operation: a single archive entry copied to a
//! single destination path. It is the vocabulary the whole engine shares, so
//! "atom" always means this and never an archive entry or a file entry.
//!
//! An install plan, real or simulated, is built by collecting atoms from the
//! required files, the plugins and the conditional install patterns, then
//! resolving the atoms that target the same destination down to one winner.
//!
//! ## Two resolvers, two rules
//!
//! There is no single conflict rule. The real installer and the inference-side
//! simulator resolve the same conflict differently, and a reader has to know
//! which one they are reasoning about:
//!
//! ```text
//! conflict on one dest_path -> who wins?
//!
//!   execute_file_operations    key: (priority, FileOperation::document_order)
//!   (real install)             ascending, stable sort, then copied in order
//!                              -> the last copy overwrites and wins
//!
//!   apply_atom                 key: priority only, `new >= existing`,
//!   (simulator, scoring)       applied in phase order, not doc_order order
//!                              -> the last atom applied wins
//!
//!   phase order in the simulator:
//!     1 required
//!     2 selected plugins (and Required-typed ones), in step/group/plugin
//!       order; each selected plugin's whole bucket is applied at once
//!     3 always-install / install-if-usable atoms of unselected plugins
//!     4 conditional patterns
//! ```
//!
//! Both rules are last-writer-wins on a tie, matching MO2, but they compare
//! different keys, and the two fields named `document_order` are not the same
//! sequence. [`crate::fomod_service::execute_file_operations`] sorts on
//! `FileOperation::document_order`, stamped as each operation is enqueued. The
//! simulator's `apply_atom` compares `priority` alone and breaks ties by its own
//! application order; it never reads `FomodAtom::document_order`.
//!
//! The clearest disagreement is inside one selected plugin. The installer
//! enqueues that plugin's entries in XML order, while
//! [`crate::fomod_inference_atoms::expand_all_atoms`] fills the plugin's bucket
//! with all of its normal entries before any of its `alwaysInstall` or
//! `installIfUsable` entries, and phase 2 applies the whole bucket at once. For
//! a plugin whose XML lists an auto entry ahead of a normal entry on the same
//! destination at equal priority, the installer keeps the normal entry and the
//! simulator keeps the auto one. Equal priority is where the two can differ;
//! [`crate::fomod_forward_simulator`] carries the list of known cases and is
//! the place to update when a new one turns up.
//!
//! `document_order` counts file entries, not atoms. It increments once per
//! expanded entry, so every atom produced from one `<folder>` entry carries the
//! same value (see [`crate::fomod_inference_atoms::expand_all_atoms`]). Equal
//! `(priority, document_order)` pairs are therefore normal, not a bug.
//!
//! `content_hash` and `file_size` are evidence for inference scoring only. They
//! are compared against the [`TargetTree`] built from the installed mod. No code
//! path skips an extraction or a copy because of them.

use std::collections::HashMap;

/// Which FOMOD section an atom came from, and therefore what decides whether it
/// installs.
///
/// | Origin      | Source                        | When it is included                                             |
/// |-------------|-------------------------------|-----------------------------------------------------------------|
/// | Required    | `<requiredInstallFiles>`      | Always                                                          |
/// | Plugin      | `<files>` inside a `<plugin>` | When the plugin is selected; or the atom sets `always_install`; |
/// |             |                               | or it sets `install_if_usable` and the plugin's effective       |
/// |             |                               | type is not `NotUsable`                                         |
/// | Conditional | `<conditionalFileInstalls>`   | When the pattern condition is met                               |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Origin {
    /// From `<requiredInstallFiles>`. Always included.
    #[default]
    Required,
    /// From a `<plugin>/<files>` block. Included when the plugin is selected,
    /// and also when it is not selected but the atom sets `always_install`, or
    /// sets `install_if_usable` and the plugin's effective type is not
    /// `NotUsable`. Breaking that link between selection and installation is
    /// what those two flags exist for.
    Plugin,
    /// From `<conditionalFileInstalls>`. Included when the pattern condition is
    /// met.
    Conditional,
}

/// One file-install operation produced by evaluating the FOMOD XML.
///
/// `plugin_index` and `conditional_index` default to -1, not 0, which is why
/// `Default` is written out by hand instead of derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FomodAtom {
    /// Archive-relative source entry path, normalized by
    /// [`crate::utils::normalize_path`]: lowercase, forward slashes, no
    /// leading or trailing slash.
    pub source_path: String,
    /// Mod-relative destination path. Atoms expanded from a `<folder>` entry
    /// carry a freshly normalized path. Atoms expanded from a `<file>` entry
    /// carry the owning [`crate::fomod_ir::FomodFileEntry`]'s `destination`
    /// verbatim, which the parser already normalized, so a hand-built entry
    /// can put a non-normalized string here.
    pub dest_path: String,
    /// Overwrite priority; higher values win conflicts. Comes from the XML
    /// `priority` attribute; 0 when the attribute is absent.
    pub priority: i32,
    /// Position of the source entry in the XML, ascending. It counts file
    /// entries, not atoms, so every atom expanded from one `<folder>` entry
    /// shares a value. No production code reads it: the installer's conflict
    /// sort uses `FileOperation::document_order`, a separate counter stamped at
    /// enqueue time, and the simulator compares `priority` only. It is kept so
    /// an atom can be traced back to the entry that produced it.
    pub document_order: i32,
    /// FNV-1a 64-bit hash of the archive entry's uncompressed bytes; 0 means
    /// not yet computed. Filled in lazily and only for contested destinations.
    pub content_hash: u64,
    /// Uncompressed size in bytes; 0 means unknown. A 0 on either side makes
    /// every size comparison pass rather than fail.
    pub file_size: u64,
    /// Which FOMOD section produced this atom.
    pub origin: Origin,
    /// Flat plugin index across all steps/groups (-1 if not from a plugin).
    pub plugin_index: i32,
    /// Index into `FomodInstaller::conditional_patterns` (-1 if not
    /// conditional).
    pub conditional_index: i32,
    /// Inherited from `FomodFileEntry::always_install`.
    pub always_install: bool,
    /// Inherited from `FomodFileEntry::install_if_usable`.
    pub install_if_usable: bool,
}

impl Default for FomodAtom {
    fn default() -> Self {
        FomodAtom {
            source_path: String::new(),
            dest_path: String::new(),
            priority: 0,
            document_order: 0,
            content_hash: 0,
            file_size: 0,
            origin: Origin::Required,
            plugin_index: -1,
            conditional_index: -1,
            always_install: false,
            install_if_usable: false,
        }
    }
}

/// Metadata for a file already present in the installed mod directory.
///
/// Read during tree comparison, after conflict resolution has already picked one
/// winning atom per destination. Conflict resolution compares atoms against each
/// other and never reads the target tree. The comparison sites are
/// `compare_trees`, `collect_mismatched_dests` and `classify_dests` in
/// [`crate::fomod_forward_simulator`], plus the CSP precompute evidence walk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetFile {
    /// Size in bytes of the installed file; 0 means not yet known. A 0 on
    /// either side of a comparison makes the size check pass.
    pub size: u64,
    /// FNV-1a 64-bit content hash; 0 means not computed. A 0 on either side of
    /// a comparison makes the hash check pass. Hashing is lazy and happens
    /// only for contested destinations.
    pub hash: u64,
}

/// Maps a destination path to every atom that targets it.
///
/// Keys are normalized destination paths, so lowercase with forward slashes.
/// Each `Vec` holds its atoms in [`ExpandedAtoms::for_each`] order. Conflict
/// resolution does not read this index; see the module doc for what does.
pub type AtomIndex = HashMap<String, Vec<FomodAtom>>;

/// Maps a destination path to the metadata of the installed file there.
///
/// Keys are mod-relative paths in the same normalized form as [`AtomIndex`]'s,
/// so one string looks a destination up in both.
pub type TargetTree = HashMap<String, TargetFile>;

/// Atoms grouped by origin, ready for selection-based filtering.
///
/// `required` atoms always install. `per_plugin` and `per_conditional` are
/// indexed by flat plugin index and by conditional pattern index, so a caller
/// includes or excludes a whole bucket per selection or per dependency result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExpandedAtoms {
    /// Atoms from `<requiredInstallFiles>`.
    pub required: Vec<FomodAtom>,
    /// Atoms per flat plugin index across all steps/groups.
    pub per_plugin: Vec<Vec<FomodAtom>>,
    /// Atoms per conditional pattern index.
    pub per_conditional: Vec<Vec<FomodAtom>>,
}

impl ExpandedAtoms {
    /// Visit every atom once, in a fixed order: `required`, then `per_plugin` in
    /// flat plugin order, then `per_conditional` in pattern order.
    ///
    /// [`crate::fomod_inference_atoms::build_atom_index`] inherits this order,
    /// so changing it changes that index's per-destination sequence.
    pub fn for_each(&self, mut f: impl FnMut(&FomodAtom)) {
        for a in &self.required {
            f(a);
        }
        for v in &self.per_plugin {
            for a in v {
                f(a);
            }
        }
        for v in &self.per_conditional {
            for a in v {
                f(a);
            }
        }
    }

    /// Mutable counterpart of [`ExpandedAtoms::for_each`], same visit order.
    pub fn for_each_mut(&mut self, mut f: impl FnMut(&mut FomodAtom)) {
        for a in &mut self.required {
            f(a);
        }
        for v in &mut self.per_plugin {
            for a in v {
                f(a);
            }
        }
        for v in &mut self.per_conditional {
            for a in v {
                f(a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- field defaults ---

    #[test]
    fn fomod_atom_defaults_match_cpp() {
        let a = FomodAtom::default();
        assert_eq!(a.source_path, "");
        assert_eq!(a.dest_path, "");
        assert_eq!(a.priority, 0);
        assert_eq!(a.document_order, 0);
        assert_eq!(a.content_hash, 0);
        assert_eq!(a.file_size, 0);
        assert_eq!(a.origin, Origin::Required);
        assert_eq!(a.plugin_index, -1);
        assert_eq!(a.conditional_index, -1);
        assert!(!a.always_install);
        assert!(!a.install_if_usable);
    }

    #[test]
    fn target_file_defaults_match_cpp() {
        let t = TargetFile::default();
        assert_eq!(t.size, 0);
        assert_eq!(t.hash, 0);
    }

    // --- for_each iteration order ---

    fn atom(dest: &str) -> FomodAtom {
        FomodAtom {
            dest_path: dest.to_string(),
            ..FomodAtom::default()
        }
    }

    fn sample() -> ExpandedAtoms {
        ExpandedAtoms {
            required: vec![atom("r0"), atom("r1")],
            per_plugin: vec![vec![atom("p0a")], vec![], vec![atom("p2a"), atom("p2b")]],
            per_conditional: vec![vec![atom("c0a")], vec![atom("c1a")]],
        }
    }

    #[test]
    fn for_each_visits_required_then_plugins_then_conditionals() {
        let atoms = sample();
        let mut seen = Vec::new();
        atoms.for_each(|a| seen.push(a.dest_path.clone()));
        assert_eq!(seen, ["r0", "r1", "p0a", "p2a", "p2b", "c0a", "c1a"]);
    }

    #[test]
    fn for_each_mut_visits_same_order_and_mutates() {
        let mut atoms = sample();
        let mut counter = 0;
        atoms.for_each_mut(|a| {
            a.document_order = counter;
            counter += 1;
        });
        let mut orders = Vec::new();
        atoms.for_each(|a| orders.push(a.document_order));
        assert_eq!(orders, [0, 1, 2, 3, 4, 5, 6]);
    }
}
