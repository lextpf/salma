/*!
 * @brief resolves source archives recorded by MO2 metadata.
 * @author Alex (https://github.com/lextpf)
 *
 * absolute paths are final. relative paths use the first existing candidate:
 *
 * @verbatim
 * SALMA_DOWNLOADS_PATH/value
 * mod_folder/value
 * parent(mods_dir)/value
 * parent(mods_dir)/downloads/value
 * grandparent(mods_dir)/downloads/value
 * @endverbatim
 *
 * an empty mods_dir skips its three candidates. parent_path removes trailing separators without
 * removing the final directory component.
 */

use std::path::{Component, Path, PathBuf};

// keep roots unchanged and remove trailing separators before removing a directory component.
fn parent_path(p: &Path) -> PathBuf {
    let has_relative = p.components().any(|c| {
        matches!(
            c,
            Component::Normal(_) | Component::CurDir | Component::ParentDir
        )
    });
    if !has_relative {
        return p.to_path_buf();
    }

    // a run of trailing separators contributes one empty element, so dropping the last element
    // removes only the separators. conversion replaces non-unicode path units before trimming.
    let s = p.as_os_str().to_string_lossy();
    let trimmed = s.trim_end_matches(['\\', '/']);
    if trimmed.len() < s.len() {
        return PathBuf::from(trimmed);
    }

    p.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| p.to_path_buf())
}

fn has_parent_path(p: &Path) -> bool {
    !parent_path(p).as_os_str().is_empty()
}

/**
 * @fn resolve_mod_archive(&str, &Path, &Path) -> PathBuf
 * @brief resolve archive_value to an existing archive, or an empty path on a miss.
 * @author Alex (https://github.com/lextpf)
 *
 * an empty `archive_value` resolves to nothing without touching the disk.
 */
pub fn resolve_mod_archive(archive_value: &str, mod_folder: &Path, mods_dir: &Path) -> PathBuf {
    if archive_value.is_empty() {
        return PathBuf::new();
    }

    let archive_path = Path::new(archive_value);
    if archive_path.is_absolute() {
        return if archive_path.exists() {
            archive_path.to_path_buf()
        } else {
            PathBuf::new()
        };
    }

    // `var_os` rather than `var`: a downloads path with non-UTF-8 components must still be usable,
    // and `var` would drop it. empty is treated as unset.
    let downloads = std::env::var_os("SALMA_DOWNLOADS_PATH").filter(|d| !d.is_empty());

    build_candidates(
        archive_path,
        downloads.as_ref().map(Path::new),
        mod_folder,
        mods_dir,
    )
    .into_iter()
    .find(|c| c.exists())
    .unwrap_or_default()
}

// the ordered candidate list for a relative archive_path, with the downloads directory passed in
// rather than read from the environment.
// split out of `resolve_mod_archive` so the search order is testable without mutating
// process-global environment state, which would race the other tests in this binary.
fn build_candidates(
    archive_path: &Path,
    downloads: Option<&Path>,
    mod_folder: &Path,
    mods_dir: &Path,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(downloads) = downloads {
        candidates.push(downloads.join(archive_path));
    }

    candidates.push(mod_folder.join(archive_path));
    if !mods_dir.as_os_str().is_empty() {
        let parent = parent_path(mods_dir);
        let parent = parent.as_path();
        candidates.push(parent.join(archive_path));
        candidates.push(parent.join("downloads").join(archive_path));
        if has_parent_path(parent) {
            candidates.push(parent_path(parent).join("downloads").join(archive_path));
        }
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::random_hex_string;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("salma-res-{tag}-{}", random_hex_string(8)));
        fs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    fn unique_name() -> String {
        format!("payload-{}.7z", random_hex_string(12))
    }

    fn touch(p: &Path) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(p, b"archive").expect("write file");
    }

    #[test]
    fn empty_archive_value_resolves_to_nothing() {
        let root = scratch("empty");
        assert_eq!(
            resolve_mod_archive("", &root, &root),
            PathBuf::new(),
            "an empty installationFile must not resolve"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn absolute_existing_value_resolves_to_itself() {
        let root = scratch("abs-hit");
        let archive = root.join("payload.7z");
        touch(&archive);
        let got = resolve_mod_archive(archive.to_str().unwrap(), &root, &root);
        assert_eq!(got, archive);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn absolute_missing_value_does_not_fall_through() {
        let root = scratch("abs-miss");
        let absent = root.join("ghost.7z");
        touch(&root.join("ghost.7z.decoy"));
        let got = resolve_mod_archive(absent.to_str().unwrap(), &root, &root);
        assert_eq!(got, PathBuf::new());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn relative_value_resolves_under_the_mod_folder() {
        let root = scratch("modfolder");
        let name = unique_name();
        let mod_folder = root.join("MyMod");
        let archive = mod_folder.join(&name);
        touch(&archive);
        let got = resolve_mod_archive(&name, &mod_folder, Path::new(""));
        assert_eq!(got, archive);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn relative_value_resolves_beside_the_mods_dir() {
        let root = scratch("parent");
        let name = unique_name();
        let mods_dir = root.join("mods");
        fs::create_dir_all(&mods_dir).expect("create mods");
        let archive = root.join(&name);
        touch(&archive);
        let got = resolve_mod_archive(&name, &root.join("nowhere"), &mods_dir);
        assert_eq!(got, archive);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn relative_value_resolves_in_sibling_downloads() {
        let root = scratch("dl1");
        let name = unique_name();
        let mods_dir = root.join("mods");
        fs::create_dir_all(&mods_dir).expect("create mods");
        let archive = root.join("downloads").join(&name);
        touch(&archive);
        let got = resolve_mod_archive(&name, &root.join("nowhere"), &mods_dir);
        assert_eq!(got, archive);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn relative_value_resolves_in_grandparent_downloads() {
        let root = scratch("dl2");
        let name = unique_name();
        let mods_dir = root.join("instance").join("mods");
        fs::create_dir_all(&mods_dir).expect("create mods");
        let archive = root.join("downloads").join(&name);
        touch(&archive);
        let got = resolve_mod_archive(&name, &root.join("nowhere"), &mods_dir);
        assert_eq!(got, archive);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_mods_dir_skips_the_mods_dir_candidates() {
        let root = scratch("nomods");
        let name = unique_name();
        touch(&root.join(&name));
        let got = resolve_mod_archive(&name, &root.join("nowhere"), Path::new(""));
        assert_eq!(got, PathBuf::new());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unresolvable_value_returns_empty() {
        let root = scratch("miss");
        let mods_dir = root.join("mods");
        fs::create_dir_all(&mods_dir).expect("create mods");
        let got = resolve_mod_archive(&unique_name(), &root.join("nowhere"), &mods_dir);
        assert_eq!(got, PathBuf::new());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mod_folder_candidate_outranks_the_mods_dir_chain() {
        let root = scratch("order");
        let name = unique_name();
        let mods_dir = root.join("mods");
        let mod_folder = mods_dir.join("MyMod");
        fs::create_dir_all(&mod_folder).expect("create");
        let near = mod_folder.join(&name);
        let far = root.join(&name);
        touch(&near);
        touch(&far);
        let got = resolve_mod_archive(&name, &mod_folder, &mods_dir);
        assert_eq!(got, near, "mod folder is candidate 3, mods_dir parent is 4");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn candidate_order_matches_the_documented_chain() {
        let archive = Path::new("mod.7z");
        let downloads = Path::new(r"D:\dl");
        let mod_folder = Path::new(r"C:\game\mods\MyMod");
        let mods_dir = Path::new(r"C:\game\mods");

        let got = build_candidates(archive, Some(downloads), mod_folder, mods_dir);
        assert_eq!(
            got,
            vec![
                PathBuf::from(r"D:\dl\mod.7z"),
                PathBuf::from(r"C:\game\mods\MyMod\mod.7z"),
                PathBuf::from(r"C:\game\mod.7z"),
                PathBuf::from(r"C:\game\downloads\mod.7z"),
                PathBuf::from(r"C:\downloads\mod.7z"),
            ]
        );
    }

    #[test]
    fn candidate_order_drops_only_the_downloads_entry_when_unset() {
        let archive = Path::new("mod.7z");
        let mod_folder = Path::new(r"C:\game\mods\MyMod");
        let mods_dir = Path::new(r"C:\game\mods");

        let with = build_candidates(archive, Some(Path::new(r"D:\dl")), mod_folder, mods_dir);
        let without = build_candidates(archive, None, mod_folder, mods_dir);
        assert_eq!(with.len(), without.len() + 1);
        assert_eq!(with[1..], without[..]);
    }

    #[test]
    fn candidate_order_respects_empty_and_shallow_mods_dir() {
        let archive = Path::new("mod.7z");
        let mod_folder = Path::new(r"C:\game\mods\MyMod");

        let empty = build_candidates(archive, None, mod_folder, Path::new(""));
        assert_eq!(empty, vec![PathBuf::from(r"C:\game\mods\MyMod\mod.7z")]);

        let shallow = build_candidates(archive, None, mod_folder, Path::new(r"C:\mods"));
        assert_eq!(
            shallow,
            vec![
                PathBuf::from(r"C:\game\mods\MyMod\mod.7z"),
                PathBuf::from(r"C:\mod.7z"),
                PathBuf::from(r"C:\downloads\mod.7z"),
                PathBuf::from(r"C:\downloads\mod.7z"),
            ]
        );
    }

    #[test]
    fn parent_path_mirrors_cpp_on_roots_and_relatives() {
        assert_eq!(
            parent_path(Path::new(r"C:\mods\x")),
            PathBuf::from(r"C:\mods")
        );
        assert_eq!(parent_path(Path::new(r"C:\mods")), PathBuf::from(r"C:\"));
        assert_eq!(parent_path(Path::new(r"C:\")), PathBuf::from(r"C:\"));
        assert_eq!(parent_path(Path::new("mods")), PathBuf::from(""));

        assert!(has_parent_path(Path::new(r"C:\mods")));
        assert!(has_parent_path(Path::new(r"C:\")));
        assert!(!has_parent_path(Path::new("mods")));
    }

    #[test]
    fn parent_path_keeps_the_directory_when_it_ends_in_a_separator() {
        // the behavior this pins: Path::parent() would say r"C:\MO2".
        assert_eq!(
            Path::new(r"C:\MO2\mods\").parent(),
            Some(Path::new(r"C:\MO2")),
            "guard: this is the std behavior we must NOT inherit"
        );
        assert_eq!(
            parent_path(Path::new(r"C:\MO2\mods\")),
            PathBuf::from(r"C:\MO2\mods")
        );
        assert_eq!(
            parent_path(Path::new("C:/MO2/mods/")),
            PathBuf::from("C:/MO2/mods")
        );
        assert_eq!(
            parent_path(Path::new(r"C:\MO2\mods\\\")),
            PathBuf::from(r"C:\MO2\mods")
        );
    }

    #[test]
    fn trailing_separator_mods_dir_shifts_the_whole_chain() {
        let archive = Path::new("mod.7z");
        let mod_folder = Path::new(r"C:\game\mods\MyMod");

        let plain = build_candidates(archive, None, mod_folder, Path::new(r"C:\game\mods"));
        let trailing = build_candidates(archive, None, mod_folder, Path::new(r"C:\game\mods\"));

        assert_eq!(plain[1], PathBuf::from(r"C:\game\mod.7z"));
        assert_eq!(trailing[1], PathBuf::from(r"C:\game\mods\mod.7z"));
        assert_eq!(plain[2], PathBuf::from(r"C:\game\downloads\mod.7z"));
        assert_eq!(trailing[2], PathBuf::from(r"C:\game\mods\downloads\mod.7z"));
    }
}
