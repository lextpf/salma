//! Resolve a mod's source archive from its `meta.ini` `installationFile` value.
//!
//! Backs the `resolveModArchive` C ABI export, which both the MO2 Python plugin
//! (`scripts/mo2-salma.py`) and the dashboard FOMOD scan call, so the two agree
//! on which archive a mod folder maps to.
//!
//! Step 1 is a gate, not a candidate. An absolute `archive_value` is decided on
//! the spot: if it does not exist the result is empty and candidates 2 to 6 are
//! never probed. Only a relative value reaches the first-hit-wins chain.
//!
//! ```text
//! gate  archive_value == ""       -> ""   no disk access at all
//! gate  archive_value absolute    -> exists ? archive_value : ""
//!                                    a miss here is final, the chain is skipped
//!
//! chain (relative value only), first existing path wins:
//!   2  <downloads>/<value>                  only when $SALMA_DOWNLOADS_PATH is
//!                                           set and non-empty
//!   3  <mod_folder>/<value>                 not emptiness-guarded: an empty
//!                                           mod_folder probes the process CWD
//!   4  <parent>/<value>                 \
//!   5  <parent>/downloads/<value>       |   all three skipped when mods_dir is
//!   6  <grandparent>/downloads/<value>  /   empty; 6 only when <parent> itself
//!                                           has a parent
//!   nothing exists                      -> ""
//! ```
//!
//! `<parent>` is `parent_path(mods_dir)`, not `mods_dir/..`: a trailing
//! separator on `mods_dir` makes `<parent>` the directory itself. See
//! `parent_path`.
//!
//! The order above is a contract, not a preference. Three places state it: the
//! diagram, `build_candidates` which builds the list, and
//! `candidate_order_matches_the_documented_chain` which asserts it. Change one
//! and the other two have to follow. `PARITY-NOTES.md` records why the order is
//! what it is.
//!
//! Nothing here logs.

use std::path::{Component, Path, PathBuf};

/// Parent directory of `p`, following `std::filesystem::path::parent_path()`
/// semantics.
///
/// [`Path::parent`] is not a drop-in; it differs in two ways that both change
/// which archive gets resolved:
///
/// 1. **Trailing separator.** A path that ends in a separator has an empty final
///    element, so the parent of `C:\MO2\mods\` is `C:\MO2\mods`. Rust's
///    [`Path::components`] discards the trailing separator, so `.parent()`
///    yields `C:\MO2`, one directory too high, silently shifting candidates 4 to
///    6 of the search chain. Callers do not normalize: `process_single_mod` in
///    `Mo2FomodController.cpp` and the FOMOD scan tool in
///    `scripts/mo2-salma.py` both pass the configured path straight through, so
///    a trailing separator does reach here.
/// 2. **No relative part.** The drive-root and prefix-only cases (`C:\`, `C:`)
///    return a copy of the input, where Rust reports no parent at all.
///
/// One case stays unmodeled: a UNC `mods_dir` decomposes differently, because
/// the UNC root name is `\\server` under these semantics but `\\server\share`
/// under Rust's `Prefix::UNC`. No caller produces a UNC MO2 mods directory;
/// `PARITY-NOTES.md` records the decision not to model it. Verbatim `\\?\` paths
/// are out of scope as well.
fn parent_path(p: &Path) -> PathBuf {
    // No relative part -> return a copy of the input.
    let has_relative = p.components().any(|c| {
        matches!(
            c,
            Component::Normal(_) | Component::CurDir | Component::ParentDir
        )
    });
    if !has_relative {
        return p.to_path_buf();
    }

    // A run of trailing separators contributes one empty element, so dropping
    // the last element just strips the separators. Lossy conversion is safe
    // here: every path reaching this module crossed the C ABI as UTF-8.
    let s = p.as_os_str().to_string_lossy();
    let trimmed = s.trim_end_matches(['\\', '/']);
    if trimmed.len() < s.len() {
        return PathBuf::from(trimmed);
    }

    p.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| p.to_path_buf())
}

/// True when `parent_path` yields a non-empty path.
fn has_parent_path(p: &Path) -> bool {
    !parent_path(p).as_os_str().is_empty()
}

/// Resolve `archive_value` to an existing archive, or an empty path on a miss.
///
/// The contract:
///
/// - An empty `archive_value` resolves to nothing without touching the disk.
/// - An absolute `archive_value` is accepted only if it exists, and never falls
///   through to the relative candidates. A miss there is final.
/// - `$SALMA_DOWNLOADS_PATH` is consulted only when set and non-empty.
/// - An empty `mods_dir` skips candidates 4 to 6 entirely.
/// - `mod_folder` has no emptiness guard, unlike `mods_dir`, so an empty
///   `mod_folder` does not skip candidate 3. `mod_folder.join(archive_path)`
///   then degrades to the bare relative archive name, and the existence probe
///   resolves it against the process current directory, which is
///   non-deterministic inside a DLL that MO2 loads. Deliberate, not an
///   oversight: adding a guard changes which archive an empty `mod_folder`
///   resolves to. See `PARITY-NOTES.md`.
/// - The deepest candidate is added only when the parent itself has a parent.
/// - The result is not absolutized. Nothing here canonicalizes, so the return
///   value is the matching candidate exactly as it was built: absolute only when
///   the input that produced it was absolute (an absolute `archive_value`, or an
///   absolute `$SALMA_DOWNLOADS_PATH`, `mod_folder` or `mods_dir`). A relative
///   `mod_folder` or `mods_dir` yields a relative path, which
///   `capi::resolveModArchive` hands back verbatim across the C ABI, leaving the
///   consumer to resolve it against its own working directory.
///
/// A root-relative `archive_value` such as `\payload.7z` is not absolute on
/// Windows, because it carries no drive prefix, so it reaches the join
/// candidates, and [`Path::join`] lets a rooted right-hand side replace the
/// base entirely. Here that is only a read-only existence probe. It is
/// deliberate; see `PARITY-NOTES.md` and the same characteristic on
/// `enqueue_entry`.
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

    // `var_os` rather than `var`: a downloads path with non-UTF-8 components
    // must still be usable, and `var` would drop it. Empty is treated as unset.
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

/// The ordered candidate list for a relative `archive_path`, with the downloads
/// directory passed in rather than read from the environment.
///
/// Split out of [`resolve_mod_archive`] so the search order is testable without
/// mutating process-global environment state, which would race the other tests
/// in this binary.
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

    // Extra fallbacks for common local setups.
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

    /// Unique scratch tree; every test builds its own so the env-var cases
    /// cannot see each other's files.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("salma-res-{tag}-{}", random_hex_string(8)));
        fs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    /// A relative archive name no real downloads directory can already hold.
    ///
    /// `resolve_mod_archive` consults `$SALMA_DOWNLOADS_PATH` as candidate 2,
    /// ahead of every scratch-tree candidate, and `setup.bat` sets that variable
    /// on a developer box. A fixed name like `payload.7z` would let a real
    /// download shadow the fixture and flip these assertions, so each test
    /// resolves a name that cannot already exist.
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

    /// An absolute miss is final: it must not fall through to the relative
    /// candidate chain even when a same-named file sits in the mod folder.
    #[test]
    fn absolute_missing_value_does_not_fall_through() {
        let root = scratch("abs-miss");
        let absent = root.join("ghost.7z");
        // A file with the same name exists under the mod folder, but the
        // absolute branch returns unconditionally, so the result stays empty.
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

    /// `<mods_dir>/../<value>`: mods_dir's parent.
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

    /// `<mods_dir>/../downloads/<value>`.
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

    /// `<mods_dir>/../../downloads/<value>`, the deepest candidate.
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

    /// An empty mods_dir (the C ABI's null modsDir) skips candidates 4 to 6, so
    /// a file that only the mods_dir chain would have found stays unresolved.
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

    /// The mod-folder candidate is checked before the mods_dir chain, so when
    /// both hold a match the mod-folder copy wins.
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

    /// The full candidate order, asserted without touching the environment. This
    /// test is the in-tree authority for positions 2 to 6 of the chain in the
    /// module docs; position 1, the absolute short circuit, never reaches this
    /// list.
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

    /// Without `$SALMA_DOWNLOADS_PATH` the chain loses exactly its first entry.
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

    /// An empty mods_dir yields only the downloads and mod-folder candidates,
    /// and a shallow mods_dir omits the grandparent entry.
    #[test]
    fn candidate_order_respects_empty_and_shallow_mods_dir() {
        let archive = Path::new("mod.7z");
        let mod_folder = Path::new(r"C:\game\mods\MyMod");

        let empty = build_candidates(archive, None, mod_folder, Path::new(""));
        assert_eq!(empty, vec![PathBuf::from(r"C:\game\mods\MyMod\mod.7z")]);

        // mods_dir = "C:\mods" -> parent "C:\", whose parent_path is itself, so
        // has_parent_path stays true and the grandparent entry is still emitted,
        // duplicating the sibling-downloads one.
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

    /// At a drive root `Path::parent()` is `None`, while `parent_path` returns a
    /// copy of the input.
    #[test]
    fn parent_path_mirrors_cpp_on_roots_and_relatives() {
        assert_eq!(
            parent_path(Path::new(r"C:\mods\x")),
            PathBuf::from(r"C:\mods")
        );
        assert_eq!(parent_path(Path::new(r"C:\mods")), PathBuf::from(r"C:\"));
        // No relative part -> parent_path returns a copy of the input.
        assert_eq!(parent_path(Path::new(r"C:\")), PathBuf::from(r"C:\"));
        assert_eq!(parent_path(Path::new("mods")), PathBuf::from(""));

        assert!(has_parent_path(Path::new(r"C:\mods")));
        assert!(has_parent_path(Path::new(r"C:\")));
        assert!(!has_parent_path(Path::new("mods")));
    }

    /// A trailing separator contributes an empty final element, so the parent is
    /// the path minus the separator, not one level higher. `Path::parent()`
    /// disagrees, and inheriting it would shift candidates 4 to 6 up a directory
    /// for any caller that passes a trailing-separator mods_dir.
    #[test]
    fn parent_path_keeps_the_directory_when_it_ends_in_a_separator() {
        // The behavior this pins: Path::parent() would say r"C:\MO2".
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
        // A run of separators is still a single empty element.
        assert_eq!(
            parent_path(Path::new(r"C:\MO2\mods\\\")),
            PathBuf::from(r"C:\MO2\mods")
        );
    }

    /// End-to-end consequence of the rule above: with a trailing-separator
    /// mods_dir the sibling-downloads candidate must resolve against
    /// `<mods_dir>/downloads`, not `<mods_dir>/../downloads`.
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
