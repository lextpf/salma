/*!
 * @brief defines file atoms and target-tree data for inference.
 * @author Alex (https://github.com/lextpf)
 *
 * an atom records one possible write and its FOMOD origin. atom indices use normalized
 * destination paths.
 */

use std::collections::HashMap;

/**
 * @enum Origin
 * @brief which FOMOD section an atom came from, and therefore what decides whether it installs.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Origin {
    #[default]
    Required,
    Plugin,
    Conditional,
}

/**
 * @struct FomodAtom
 * @brief one file-install operation produced by evaluating the FOMOD XML.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FomodAtom {
    pub source_path: String,
    /**
     * @brief mod-relative destination path.
     * @author Alex (https://github.com/lextpf)
     */
    pub dest_path: String,
    /**
     * @brief overwrite priority; higher values win conflicts.
     * @author Alex (https://github.com/lextpf)
     *
     * comes from the XML `priority` attribute; 0 when the attribute is absent.
     */
    pub priority: i32,
    /**
     * @brief position of the source entry in the XML, ascending.
     * @author Alex (https://github.com/lextpf)
     *
     * no production code reads it: the installer's conflict sort uses
     * `FileOperation::document_order`, a separate counter stamped at enqueue time, and the
     * simulator compares `priority` only.
     */
    pub document_order: i32,
    /**
     * @brief store the FNV-1a content hash; zero means unavailable.
     * @author Alex (https://github.com/lextpf)
     */
    pub content_hash: u64,
    /**
     * @brief uncompressed size in bytes; 0 means unknown.
     * @author Alex (https://github.com/lextpf)
     *
     * a 0 on either side makes every size comparison pass rather than fail.
     */
    pub file_size: u64,
    pub origin: Origin,
    pub plugin_index: i32,
    pub conditional_index: i32,
    pub always_install: bool,
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

/**
 * @struct TargetFile
 * @brief metadata for a file already present in the installed mod directory.
 * @author Alex (https://github.com/lextpf)
 *
 * conflict resolution compares atoms against each other and never reads the target tree.
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetFile {
    /**
     * @brief size in bytes of the installed file; 0 means not yet known.
     * @author Alex (https://github.com/lextpf)
     */
    pub size: u64,
    pub hash: u64,
}

/**
 * @brief maps a destination path to every atom that targets it.
 * @author Alex (https://github.com/lextpf)
 *
 * each `Vec` holds its atoms in [`ExpandedAtoms::for_each`] order.
 */
pub type AtomIndex = HashMap<String, Vec<FomodAtom>>;

/**
 * @brief maps a destination path to the metadata of the installed file there.
 * @author Alex (https://github.com/lextpf)
 *
 */
pub type TargetTree = HashMap<String, TargetFile>;

/**
 * @struct ExpandedAtoms
 * @brief atoms grouped by origin, ready for selection-based filtering.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExpandedAtoms {
    pub required: Vec<FomodAtom>,
    pub per_plugin: Vec<Vec<FomodAtom>>,
    pub per_conditional: Vec<Vec<FomodAtom>>,
}

impl ExpandedAtoms {
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
