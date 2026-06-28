/*!
 * @brief detects installable content roots in archives without a FOMOD.
 * @author Alex (https://github.com/lextpf)
 *
 * known game-data markers identify a mod root. the scan checks direct child directories and keeps
 * filesystem enumeration order.
 */

use std::fs;
use std::path::{Path, PathBuf};

use crate::logger::Logger;

// the game-data folder names that mark a directory as a mod root.
// declaration order is observable only through short-circuiting in `has_mod_structure`, which
// cannot change the boolean result.
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

/**
 * @fn has_mod_structure(&Path) -> bool
 * @brief count any existing marker path, including a regular file.
 * @author Alex (https://github.com/lextpf)
 *
 * matching follows filesystem case rules. a regular file with a marker name also counts.
 *
 * @return true when any MOD_FOLDERS child exists.
 */
pub fn has_mod_structure(dir: &Path) -> bool {
    MOD_FOLDERS.iter().any(|folder| dir.join(folder).exists())
}

/**
 * @fn find_main_mod_folders(&Path) -> Vec<PathBuf>
 * @brief return unsorted direct-child matches and keep partial results on read errors.
 * @author Alex (https://github.com/lextpf)
 *
 * a read failure logs a warning and returns the candidates collected so far. result order follows
 * filesystem enumeration and is not sorted.
 *
 * @return matching direct child directories.
 */
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
                // a mid-iteration fault stops the scan and keeps the partial results, rather than
                // skipping just this entry.
                logger.log_warning(&format!(
                    "[install] Cannot scan \"{}\": {err}",
                    archive_root.display()
                ));
                break;
            }
        };
        let path = entry.path();
        // is_dir follows symlinks and junctions, so linked mod folders remain candidates.
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

    #[test]
    fn folder_match_is_case_insensitive() {
        let root = scratch("case");
        stdfs::create_dir_all(root.join("MeShEs")).expect("create dir");
        assert!(has_mod_structure(&root));
        stdfs::remove_dir_all(&root).ok();
    }

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

    #[test]
    fn mod_folder_at_root_is_not_reported_as_a_child_candidate() {
        let root = scratch("flat");
        stdfs::create_dir_all(root.join("meshes")).expect("create");
        assert!(has_mod_structure(&root));
        assert!(find_main_mod_folders(&root).is_empty());
        stdfs::remove_dir_all(&root).ok();
    }
}
