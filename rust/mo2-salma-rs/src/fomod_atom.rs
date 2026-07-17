//! Atom datatypes - Rust port of `src/FomodAtom.hpp`.
//!
//! An atom is the smallest unit of work in a FOMOD installation: one source
//! file mapped to one destination path. The full install plan is assembled by
//! collecting atoms from required files, selected plugins, and conditional
//! install patterns, then resolving conflicts by priority and document order.
//!
//! When multiple atoms target the same destination path, the winner is the
//! atom that maximizes `(priority, document_order)` in lexicographic order.
//! Ties on both keys are not expected because `document_order` is a monotonic
//! enqueue counter, but if they occur the implementation keeps the first-seen
//! atom. `content_hash` and `file_size` allow skipping redundant extractions
//! when the winning atom is byte-identical to an already-installed file.

use std::collections::HashMap;

/// Where an atom originated in the FOMOD XML. Mirror of `FomodAtom::Origin`.
///
/// | Origin      | Source                        | Lifetime                      |
/// |-------------|-------------------------------|-------------------------------|
/// | Required    | `<requiredInstallFiles>`      | Always included               |
/// | Plugin      | `<files>` inside a `<plugin>` | Included when plugin selected |
/// | Conditional | `<conditionalFileInstalls>`   | Included when pattern matches |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Origin {
    /// From `<requiredInstallFiles>` - always included.
    #[default]
    Required,
    /// From a `<plugin>/<files>` block - included when the plugin is selected.
    Plugin,
    /// From `<conditionalFileInstalls>` - included when the pattern condition
    /// is met.
    Conditional,
}

/// Single file-install operation produced by FOMOD XML evaluation. Mirror of
/// `mo2core::FomodAtom` with identical field defaults (note `plugin_index` /
/// `conditional_index` default to -1, so `Default` is implemented by hand).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FomodAtom {
    /// Normalized archive-relative source entry path.
    pub source_path: String,
    /// Normalized mod-relative destination path.
    pub dest_path: String,
    /// Overwrite priority; higher values win conflicts.
    pub priority: i32,
    /// XML document order tiebreaker (higher wins among equal priority).
    pub document_order: i32,
    /// FNV-1a hash of archive bytes (0 = not yet computed).
    pub content_hash: u64,
    /// File size in bytes (0 = unknown).
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

/// Metadata for a file already present in the target directory. Mirror of
/// `mo2core::TargetFile`. Used during conflict resolution to detect when the
/// winning atom is byte-identical to an existing file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetFile {
    /// Uncompressed size in bytes (0 = not yet known).
    pub size: u64,
    /// FNV-1a content hash (0 = not computed).
    pub hash: u64,
}

/// Maps a lowercased destination path to every atom that targets it. Mirror
/// of `mo2core::AtomIndex`.
pub type AtomIndex = HashMap<String, Vec<FomodAtom>>;

/// Maps a lowercased destination path to metadata of the installed file.
/// Mirror of `mo2core::TargetTree`.
pub type TargetTree = HashMap<String, TargetFile>;

/// Atoms grouped by origin, ready for selection-based filtering. Mirror of
/// `mo2core::ExpandedAtoms`.
///
/// `required` atoms are always installed. `per_plugin` and `per_conditional`
/// vectors are indexed by the corresponding flat plugin or conditional index,
/// so the caller can include or exclude each group based on user selections
/// and dependency evaluation results.
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
    /// Apply a callable to every atom across all origin containers, in the
    /// C++ `for_each` iteration order: required, then per_plugin in flat
    /// order, then per_conditional in pattern order.
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

    /// Mutable counterpart of [`ExpandedAtoms::for_each`] (the C++ non-const
    /// overload), preserving the same iteration order.
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

    // --- defaults (must match the C++ in-struct initializers) ---

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
