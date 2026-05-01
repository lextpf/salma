//! Content-root detection for archives that carry no FOMOD installer.
//!
//! [`crate::installation_service`] uses this to find which subdirectory of an
//! extracted archive holds the mod content, so a wrapper folder
//! (`ModName-v1.2/meshes/...` instead of `meshes/...`) is not installed one
//! level too deep.
//!
//! Detection is a name probe, not a content inspection: a directory counts as a
//! mod root when it holds an entry named in [`MOD_FOLDERS`].

use std::fs;
use std::path::{Path, PathBuf};

use crate::logger::Logger;

/// The game-data folder names that mark a directory as a mod root.
///
/// Declaration order is observable only through short-circuiting in
/// [`has_mod_structure`], which cannot change the boolean result.
const MOD_FOLDERS: [&str; 11] = [
    "SKSE",
    "meshes",
    "textures",
    "interface",
    "sound",
    "scripts",
    "seq",
    "F4SE",
    "SFSE",
    "OBSE",
    "materials",
];

/// Whether `dir` looks like a mod root, that is, holds any of [`MOD_FOLDERS`].
///
/// Two properties of the probe are load-bearing:
///
/// - Matching is case-insensitive, with no explicit `to_lower`, because
///   [`Path::exists`] resolves the name through Win32 on NTFS. `meshes`,
///   `Meshes` and `MESHES` all match. On a case-sensitive filesystem only the
///   listed spellings would match.
/// - The probe is `exists`, not `is_dir`, so a plain file named `textures`
///   makes the directory a mod root. This is deliberate, not an oversight;
///   tightening it changes which folder gets installed for archives that ship
///   such a file. See PARITY-NOTES.md.
///
/// Every probe error reads as `false`, so a permission fault on one candidate
/// leaves the scan running. The input is always a freshly extracted temp tree
/// owned by this process, so no probe is expected to fail.
pub fn has_mod_structure(dir: &Path) -> bool {
    MOD_FOLDERS.iter().any(|folder| dir.join(folder).exists())
}

/// Every immediate subdirectory of `archive_root` that [`has_mod_structure`].
///
/// One level deep only:
///
/// ```text
///   archive root/
///     ModA/meshes/          -> candidate
///     ModB/textures/        -> candidate
///     Docs/readme/          -> not a candidate, no game-data folder
///     Outer/Inner/meshes/   -> not a candidate, the marker is two levels down
///     meshes/               -> not a candidate, only children are reported
/// ```
///
/// A read failure logs a warning and returns the candidates collected so far,
/// so an unreadable `archive_root` yields an empty vector and a fault partway
/// through yields a partial one.
///
/// Result order follows [`fs::read_dir`] (Win32
/// `FindFirstFileW`/`FindNextFileW`) and is not sorted further. Order does not
/// reach install behavior: the caller uses index 0 only when the vector holds
/// exactly one entry, and otherwise selects by name match, treating any other
/// count as fatal.
pub fn find_main_mod_folders(archive_root: &Path) -> Vec<PathBuf> {
    let logger = Logger::instance();
    let mut results = Vec::new();

    let entries = match fs::read_dir(archive_root) {
        Ok(entries) => entries,
        Err(err) => {
            logger.log_warning(&format!(
                "[install] Cannot scan \"{}\": {err}",
                archive_root.display()
            ));
            return results;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                // A mid-iteration fault stops the scan and keeps the partial
                // results, rather than skipping just this entry.
                logger.log_warning(&format!(
                    "[install] Cannot scan \"{}\": {err}",
                    archive_root.display()
                ));
                break;
            }
        };
        let path = entry.path();
        // `Path::is_dir` resolves through `fs::metadata`, so it follows
        // symlinks and Windows junctions: a junction pointing at a mod folder
        // counts as a candidate.
        //
        // Do not swap in `entry.file_type()`. That call does not traverse a
        // link, which is why `FileOperations::copy_folder` uses it to skip
        // symlinked entries, so the switch would silently stop counting
        // junctions and change which folder gets installed.
        if !path.is_dir() {
            continue;
        }
        if has_mod_structure(&path) {
            logger.log(&format!(
                "[install]    candidate mod folder: \"{}\"",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            results.push(path);
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::random_hex_string;
    use std::fs as stdfs;

    /// Unique scratch directory under the OS temp dir.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("salma-msd-{tag}-{}", random_hex_string(8)));
        stdfs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    #[test]
    fn empty_directory_has_no_mod_structure() {
        let root = scratch("empty");
        assert!(!has_mod_structure(&root));
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn each_recognized_folder_marks_a_mod_root() {
        for folder in MOD_FOLDERS {
            let root = scratch("one");
            stdfs::create_dir_all(root.join(folder)).expect("create marker");
            assert!(has_mod_structure(&root), "{folder} should mark a mod root");
            stdfs::remove_dir_all(&root).ok();
        }
    }

    #[test]
    fn unrecognized_folder_does_not_mark_a_mod_root() {
        let root = scratch("other");
        stdfs::create_dir_all(root.join("docs")).expect("create dir");
        stdfs::create_dir_all(root.join("source")).expect("create dir");
        assert!(!has_mod_structure(&root));
        stdfs::remove_dir_all(&root).ok();
    }

    /// `Path::exists` is case-insensitive on Windows.
    #[test]
    fn folder_match_is_case_insensitive() {
        let root = scratch("case");
        stdfs::create_dir_all(root.join("MeShEs")).expect("create dir");
        assert!(has_mod_structure(&root));
        stdfs::remove_dir_all(&root).ok();
    }

    /// The probe does not require a directory, so a file named after a mod
    /// folder counts.
    #[test]
    fn a_file_named_like_a_mod_folder_also_counts() {
        let root = scratch("file");
        stdfs::write(root.join("textures"), b"not a directory").expect("write file");
        assert!(has_mod_structure(&root));
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn find_main_mod_folders_returns_only_qualifying_subdirectories() {
        let root = scratch("find");
        stdfs::create_dir_all(root.join("ModA").join("meshes")).expect("create");
        stdfs::create_dir_all(root.join("ModB").join("textures")).expect("create");
        stdfs::create_dir_all(root.join("Docs").join("readme")).expect("create");
        stdfs::write(root.join("loose.txt"), b"x").expect("write");

        let mut found: Vec<String> = find_main_mod_folders(&root)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, vec!["ModA".to_string(), "ModB".to_string()]);
        stdfs::remove_dir_all(&root).ok();
    }

    /// One level deep only: `Outer/Inner/meshes` must not be detected.
    #[test]
    fn find_main_mod_folders_does_not_recurse() {
        let root = scratch("nested");
        stdfs::create_dir_all(root.join("Outer").join("Inner").join("meshes")).expect("create");
        assert!(find_main_mod_folders(&root).is_empty());
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn find_main_mod_folders_on_missing_root_is_empty() {
        let root = std::env::temp_dir().join(format!("salma-msd-absent-{}", random_hex_string(8)));
        assert!(find_main_mod_folders(&root).is_empty());
    }

    /// A top-level `meshes/` makes the root itself a mod root, but
    /// `find_main_mod_folders` reports children only, so the caller falls
    /// through to the flat copy.
    #[test]
    fn mod_folder_at_root_is_not_reported_as_a_child_candidate() {
        let root = scratch("flat");
        stdfs::create_dir_all(root.join("meshes")).expect("create");
        assert!(has_mod_structure(&root));
        assert!(find_main_mod_folders(&root).is_empty());
        stdfs::remove_dir_all(&root).ok();
    }
}
