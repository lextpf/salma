//! Top-level install orchestrator: extract, detect, dispatch, clean up.
//!
//! Rust port of `src/InstallationService.hpp`/`.cpp`. This is what the
//! `install` / `installWithConfig` C ABI exports call. It extracts the archive
//! to a temp directory, decides whether the tree carries a FOMOD installer, and
//! delegates to either [`crate::fomod_service`] (the replay) or the non-FOMOD
//! content-root copy via [`crate::mod_structure_detector`].
//!
//! ## Error model
//!
//! The C++ signals every fatal condition by throwing `std::runtime_error`, and
//! `CApi::install` returns `e.what()` to the caller. Rust has no exceptions, so
//! [`install_mod`](InstallationService::install_mod) returns
//! `Result<String, InstallError>` and [`InstallError`] carries exactly the
//! string the C++ `what()` would have produced. `capi` returns that string
//! verbatim, so the ABI-visible behavior is unchanged.
//!
//! There is no Rust logger yet (Task 17). Every `Logger::instance().log*` call
//! is dropped; the branch that produced it is kept with a `// dropped log site`
//! comment so Task 17 can restore it verbatim.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::archive_service::ArchiveService;
use crate::file_operations::FileOperations;
use crate::fomod_ir_parser::parse_module_config;
use crate::fomod_service::{FomodService, execute_file_operations};
use crate::json::{self, Value};
use crate::mod_structure_detector::find_main_mod_folders;
use crate::types::{FileOperation, FomodDependencyContext};
use crate::utils::{is_inside, random_hex_string, to_lower};

/// A fatal install failure, carrying the exact message the C++
/// `std::runtime_error::what()` would return.
///
/// `capi::install` / `capi::installWithConfig` hand this string straight back
/// across the ABI, so its bytes are part of the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError(pub String);

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InstallError {}

impl InstallError {
    fn new(message: impl Into<String>) -> Self {
        InstallError(message.into())
    }
}

/// The five base-game plugins the C++ seeds into every dependency context
/// (`InstallationService.cpp:346-350`), in insertion order.
const SEED_PLUGINS: [&str; 5] = [
    "skyrim.esm",
    "update.esm",
    "dawnguard.esm",
    "hearthfires.esm",
    "dragonborn.esm",
];

/// Windows reserved device names rejected as a `moduleName`
/// (`InstallationService.cpp:222-225`). Compared against the lowercased stem,
/// so `con.txt` is rejected exactly like `con`.
const RESERVED_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// FOMOD install orchestrator.
///
/// Mirror of `mo2core::InstallationService`. The C++ class holds no members and
/// the C ABI constructs a fresh instance per call; this is the same shape, kept
/// as a struct rather than free functions so the call sites read identically.
///
/// Not thread-safe, matching `InstallationService.hpp:130-134`: the disk-full
/// marker it resets and reads is a process-global in [`FileOperations`], so two
/// concurrent installs would observe each other's disk pressure.
#[derive(Debug, Default)]
pub struct InstallationService;

impl InstallationService {
    /// Construct a service instance.
    pub fn new() -> Self {
        InstallationService
    }

    /// Install `archive_path` into `mod_path`, optionally driven by a selections
    /// JSON at `json_path` (empty means "derive from the archive stem").
    ///
    /// Mirror of `InstallationService::install_mod`
    /// (`InstallationService.cpp:23-146`). Returns `mod_path` on success.
    ///
    /// Lifecycle, in C++ order:
    /// 1. Reset the sticky disk-full marker.
    /// 2. Fail if the archive does not exist.
    /// 3. Create `mod_path` and a fresh `%TEMP%/fomod-<8 hex>/archive`.
    /// 4. Extract, locate a `fomod` folder, dispatch.
    /// 5. Remove the temp tree (failures warn, they do not fail the install).
    /// 6. Convert a disk-full marker into a hard failure.
    ///
    /// The temp tree is removed on BOTH exit paths. `mod_path` is deliberately
    /// NOT cleaned up on failure, matching `InstallationService.hpp:122-128`:
    /// once the copy passes have run, partial content is left for the caller.
    pub fn install_mod(
        &mut self,
        archive_path: &str,
        mod_path: &str,
        json_path: &str,
    ) -> Result<String, InstallError> {
        // Clear the sticky disk-full marker before any work so a previous
        // install's disk pressure does not poison this run (IS 36).
        FileOperations::reset_disk_full();

        // dropped log site: log("[install] === Starting mod installation ===")
        // dropped log site: log("[install] Archive: {}", archive_path)
        // dropped log site: log("[install] Target mod directory: {}", mod_path)
        // dropped log site: log("[install] Initialization finished")

        if !Path::new(archive_path).exists() {
            return Err(InstallError::new(format!(
                "Archive file not found: {archive_path}"
            )));
        }

        // dropped log site: log_warning("[install] Could not read archive size: {}")
        // on error, then log("[install] Archive size: {} bytes ({:.2f} MB)").
        // The size is read purely to log it, so the whole block is dropped.

        // The C++ `fs::create_directories` throws on failure and the exception
        // escapes install_mod BEFORE any temp dir exists, so there is nothing to
        // clean up on this path. Same here.
        fs::create_dir_all(mod_path)
            .map_err(|e| InstallError::new(format!("Cannot create mod directory: {e}")))?;
        // dropped log site: log("[install] Created mod directory: {}", mod_path)

        let temp_dir = std::env::temp_dir().join(format!("fomod-{}", random_hex_string(8)));
        let archive_extract_dir = temp_dir.join("archive");
        fs::create_dir_all(&archive_extract_dir)
            .map_err(|e| InstallError::new(format!("Cannot create temp directory: {e}")))?;
        // dropped log site: log("[install] Temporary directory: {}", temp_dir)

        let outcome = self.run_install(
            archive_path,
            mod_path,
            json_path,
            &temp_dir,
            &archive_extract_dir,
        );

        // Cleanup is symmetric across both exit paths (IS 103-113 on success,
        // IS 132-145 in the catch-all). Removal failures only warn.
        // dropped log site: log("[install] Cleaned up temporary directory") on
        // success, log_warning("[install] WARNING: Failed to cleanup temp
        // directory: {}") / ("[install] Failed to cleanup temp directory on
        // error path") on failure.
        let _ = fs::remove_dir_all(&temp_dir);

        // dropped log site: log("[install] Total installation time: {:.2f} seconds")

        let result = outcome?;

        // Some files could not be copied because the volume ran out of space.
        // Surface it as a hard failure so a half-empty mod is never reported as
        // installed (IS 120-128). The C++ throws here, INSIDE the try, so its
        // catch-all runs `remove_all` a second time; that is a no-op on an
        // already-removed tree, which is why one removal above suffices.
        if FileOperations::disk_full_encountered() {
            return Err(InstallError::new(
                "Install aborted: disk full while copying files. Free space and retry.",
            ));
        }

        Ok(result)
    }

    /// The body of the C++ `try` block (`InstallationService.cpp:68-101`):
    /// extract, find the FOMOD folder, dispatch. Split out so the caller can
    /// run the temp-dir cleanup on both exit paths without duplicating it.
    fn run_install(
        &mut self,
        archive_path: &str,
        mod_path: &str,
        json_path: &str,
        temp_dir: &Path,
        archive_extract_dir: &Path,
    ) -> Result<String, InstallError> {
        // dropped log site: log("[install] Extracting archive...")
        let archive_service = ArchiveService::new();
        archive_service
            .extract(
                archive_path,
                archive_extract_dir
                    .to_str()
                    .ok_or_else(|| InstallError::new("Temp directory path is not valid UTF-8"))?,
            )
            // The C++ lets bit7z / libarchive errors propagate as-is, so their
            // `what()` becomes the ABI's error string. The Rust backends carry
            // their own wording (see PARITY-NOTES "Task 15"), so the FAILURE is
            // parity but the TEXT is not.
            .map_err(|e| InstallError::new(e.to_string()))?;
        // dropped log site: log("[install] Archive extracted to {} in {:.2f} seconds")

        // dropped log site: log("[install] Searching for FOMOD folder...")
        let fomod_folder = find_fomod_folder(archive_extract_dir);

        match fomod_folder {
            None => {
                // dropped log site: log("[install] No FOMOD folder detected -
                // using standard installation")
                handle_non_fomod_install(archive_extract_dir, mod_path, archive_path, json_path)
            }
            Some(folder) => {
                // dropped log site: log("[install] FOMOD folder found: {}")
                handle_fomod_install(
                    &folder,
                    archive_extract_dir,
                    mod_path,
                    archive_path,
                    temp_dir,
                    json_path,
                )
            }
        }
    }
}

/// Find the first directory named `fomod` (case-insensitively) that contains a
/// `moduleconfig.xml` (case-insensitively).
///
/// Mirror of `InstallationService::find_fomod_folder`
/// (`InstallationService.cpp:148-168`). The C++ walks a
/// `recursive_directory_iterator` and returns the FIRST match in iteration
/// order; there is no shallowest-path preference here, unlike the inference
/// pipeline's ModuleConfig lookup. That asymmetry is real and is reproduced.
///
/// Returns `None` when no such folder exists.
fn find_fomod_folder(archive_root: &Path) -> Option<PathBuf> {
    // Explicit stack: the C++ `recursive_directory_iterator` yields entries
    // depth-first, visiting a directory's own entry before descending into it.
    // Matching that order matters because the FIRST match wins.
    fn walk(dir: &Path) -> Option<PathBuf> {
        let entries = fs::read_dir(dir).ok()?;
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name_matches = path
                .file_name()
                .map(|n| to_lower(&n.to_string_lossy()) == "fomod")
                .unwrap_or(false);
            if name_matches && contains_module_config(&path) {
                return Some(path);
            }
            subdirs.push(path);
        }
        for sub in subdirs {
            if let Some(hit) = walk(&sub) {
                return Some(hit);
            }
        }
        None
    }

    walk(archive_root)
}

/// Whether `dir` holds a regular file named `moduleconfig.xml`, case-insensitively
/// (`InstallationService.cpp:158-165`).
fn contains_module_config(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|child| {
        let path = child.path();
        path.is_file()
            && path
                .file_name()
                .map(|n| to_lower(&n.to_string_lossy()) == "moduleconfig.xml")
                .unwrap_or(false)
    })
}

/// Non-FOMOD install: pick a content root and copy it, or copy the whole tree.
///
/// Mirror of `InstallationService::handle_non_fomod_install`
/// (`InstallationService.cpp:170-295`).
fn handle_non_fomod_install(
    archive_root: &Path,
    mod_path: &str,
    archive_path: &str,
    json_path: &str,
) -> Result<String, InstallError> {
    // dropped log site: log("[install] No 'fomod' folder found; checking for
    // nested mod structure in: {}")

    let effective_json = resolve_json_path(json_path, archive_path);
    let mut module_name_lower = String::new();

    if !effective_json.is_empty() && Path::new(&effective_json).exists() {
        let config = read_json_config(&effective_json);
        if let Some(name) = config.get("moduleName").filter(|v| v.is_string())
            && let Some(s) = name.as_str()
        {
            module_name_lower = to_lower(s);
            // dropped log site: log("[install] Detected moduleName \"{}\" in JSON")
        }
    }

    // Path separators and parent segments are rejected by clearing the value,
    // NOT by failing (IS 209-216). A cleared name then falls through to the
    // ambiguous-folder failure below when several candidates exist.
    if module_name_lower.contains('/')
        || module_name_lower.contains('\\')
        || module_name_lower.contains("..")
    {
        // dropped log site: log_warning("[install] Rejecting moduleName with
        // path separators: \"{}\"")
        module_name_lower.clear();
    }

    // Windows reserved device names cannot be directory names and fail
    // silently, so they are rejected the same way (IS 218-235).
    if !module_name_lower.is_empty() {
        let stem = match module_name_lower.rfind('.') {
            Some(dot) => &module_name_lower[..dot],
            None => module_name_lower.as_str(),
        };
        if RESERVED_NAMES.contains(&stem) {
            // dropped log site: log_warning("[install] Rejecting Windows
            // reserved device name: \"{}\"")
            module_name_lower.clear();
        }
    }

    let main_mod_folders = find_main_mod_folders(archive_root);

    if !main_mod_folders.is_empty() {
        let chosen: PathBuf = if main_mod_folders.len() == 1 {
            // dropped log site: log("[install] Only one mod folder \"{}\" found;
            // copying it")
            main_mod_folders[0].clone()
        } else {
            if module_name_lower.is_empty() {
                return Err(InstallError::new(
                    "Multiple mod folders detected but no moduleName in JSON to disambiguate.",
                ));
            }

            let matches: Vec<&PathBuf> = main_mod_folders
                .iter()
                .filter(|p| {
                    // dropped log site: log("[install]      matches moduleName:
                    // \"{}\"") for each hit
                    p.file_name()
                        .map(|n| to_lower(&n.to_string_lossy()) == module_name_lower)
                        .unwrap_or(false)
                })
                .collect();

            if matches.len() != 1 {
                return Err(InstallError::new(if matches.is_empty() {
                    format!("moduleName '{module_name_lower}' did not match any folder.")
                } else {
                    format!("moduleName '{module_name_lower}' matched multiple folders.")
                }));
            }

            // dropped log site: log("[install] Copying contents of chosen mod
            // folder \"{}\"")
            matches[0].clone()
        };

        FileOperations::copy_directory_contents(&chosen, Path::new(mod_path));
        return Ok(mod_path.to_string());
    }

    // Fallback: copy everything from the archive root.
    // dropped log site: log("[install] No nested mod structure detected; copying
    // all files from archive root to mod directory: {}")
    FileOperations::copy_directory_contents(archive_root, Path::new(mod_path));
    Ok(mod_path.to_string())
}

/// FOMOD install: parse the config, build the dependency context, run the three
/// file passes, then move the staged tree into place.
///
/// Mirror of `InstallationService::handle_fomod_install`
/// (`InstallationService.cpp:297-466`).
fn handle_fomod_install(
    fomod_folder: &Path,
    archive_root: &Path,
    mod_path: &str,
    archive_path: &str,
    temp_dir: &Path,
    json_path: &str,
) -> Result<String, InstallError> {
    let xml_path = fomod_folder.join("ModuleConfig.xml");
    // The C++ joins the EXACT casing `ModuleConfig.xml` even though
    // find_fomod_folder matched case-insensitively; Windows resolves it either
    // way, so both sides open whatever casing the archive shipped.
    let src_base = fomod_folder
        .parent()
        .unwrap_or(fomod_folder)
        .to_string_lossy()
        .into_owned();
    let dst_base = temp_dir.join("unfomod");

    let effective_json = resolve_json_path(json_path, archive_path);

    fs::create_dir_all(&dst_base)
        .map_err(|e| InstallError::new(format!("Cannot create staging directory: {e}")))?;

    // Parse XML. The C++ `doc.load_file` auto-detects encoding; the Rust
    // `parse_module_config` performs the same detection over the raw bytes.
    let bytes =
        fs::read(&xml_path).map_err(|e| InstallError::new(format!("Cannot parse XML ({e})")))?;
    let installer = parse_module_config(&bytes, "")
        // C++: std::format("Cannot parse XML ({})", xml_result.description()).
        // The wording of the inner description differs between pugixml and the
        // Rust loader; the failure itself is parity. See PARITY-NOTES "Task 15".
        .map_err(|e| InstallError::new(format!("Cannot parse XML ({e})")))?;
    // dropped log site: log("[install] Loaded XML: {}", xml_path)

    let config_json = if !effective_json.is_empty() && Path::new(&effective_json).exists() {
        // dropped log site: log("[install] Loaded JSON: {}", effective_json)
        read_json_config(&effective_json)
    } else {
        Value::Null
    };

    let mut context = FomodDependencyContext {
        archive_root: archive_root.to_string_lossy().into_owned(),
        installed_plugins: SEED_PLUGINS.iter().map(|p| (*p).to_string()).collect(),
        ..FomodDependencyContext::default()
    };

    if !config_json.is_null() {
        if let Some(v) = config_json.get("gamePath").filter(|v| v.is_string())
            && let Some(s) = v.as_str()
        {
            context.game_path = s.to_string();
            // dropped log site: log("[install] Game path from JSON: {}")
        }
        if let Some(v) = config_json.get("gameVersion").filter(|v| v.is_string())
            && let Some(s) = v.as_str()
        {
            context.game_version = s.to_string();
            // dropped log site: log("[install] Game version from JSON: {}")
        }
    }

    // Scan the game Data directory for plugins (IS 367-386). One level deep,
    // matching `fs::directory_iterator`.
    if !context.game_path.is_empty() {
        let data_dir = Path::new(&context.game_path).join("Data");
        if data_dir.exists()
            && let Ok(entries) = fs::read_dir(&data_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path
                    .extension()
                    .map(|e| to_lower(&format!(".{}", e.to_string_lossy())))
                    .unwrap_or_default();
                if ext == ".esp" || ext == ".esm" || ext == ".esl" {
                    let name = path
                        .file_name()
                        .map(|n| to_lower(&n.to_string_lossy()))
                        .unwrap_or_default();
                    context.installed_plugins.insert(name);
                }
            }
            // dropped log site: log("[install] Found {} plugins in game Data
            // directory", context.installed_plugins.size())
        }
    }

    // Populate installed_files from an existing mod directory, for re-installs
    // (IS 388-404).
    let mod_dir = Path::new(mod_path);
    if mod_dir.is_dir() {
        collect_installed_files(mod_dir, mod_dir, &mut context.installed_files);
        // dropped log site: log("[install] Scanned {} existing files in mod
        // directory") when non-empty
    }

    let mut fomod_service = FomodService::new();
    fomod_service.set_installer(installer);

    // dropped log site: log("[install] Checking module-level dependencies...")
    if !fomod_service.check_module_dependencies(Some(&context)) {
        return Err(InstallError::new(
            "Module-level dependencies not met - installation cannot proceed",
        ));
    }

    if !config_json.is_null() {
        // dropped log site: log("[install] Validating JSON selections...")
        // A malformed plugin `name` is where the C++ `value("name", "")` throws,
        // which aborts the whole install; a merely INVALID selection only warns.
        let valid = fomod_service
            .validate_json_selections(&config_json)
            .map_err(|e| InstallError::new(e.to_string()))?;
        if !valid {
            // dropped log site: log_warning("[install] WARNING: JSON selections
            // have group-type constraint violations")
        }
    }

    // Caller-owned operations vector and document-order counter: every pass
    // appends to the same vector so one sort orders the whole install.
    let mut file_ops: Vec<FileOperation> = Vec::new();
    let mut next_doc_order: i32 = 0;
    let dst_base_str = dst_base.to_string_lossy().into_owned();

    // dropped log site: log("[install] Processing required install files...")
    fomod_service.process_required_files(
        &src_base,
        &dst_base_str,
        &mut file_ops,
        &mut next_doc_order,
    );

    // Even without selections this still installs Required plugins and
    // alwaysInstall / installIfUsable entries from unselected plugins.
    // dropped log site: log("[install] Processing optional install files...")
    fomod_service
        .process_optional_files(
            &config_json,
            &src_base,
            &dst_base_str,
            Some(&context),
            &mut file_ops,
            &mut next_doc_order,
        )
        .map_err(|e| InstallError::new(e.to_string()))?;

    // dropped log site: log("[install] Processing conditional file installs...")
    fomod_service.process_conditional_files(
        &src_base,
        &dst_base_str,
        Some(&context),
        &mut file_ops,
        &mut next_doc_order,
    );

    let _file_op_failures = execute_file_operations(&mut file_ops);
    // dropped log site: log_warning("[install] {} file operations failed during
    // FOMOD install") when > 0. Always 0 in practice; see
    // `execute_file_operations`' own doc comment.

    // Move the staged result into the mod directory. `dst_base` lives inside the
    // temp tree that install_mod removes, so nothing needs to survive here.
    // dropped log site: log("[install] Moving unfomod files to mod directory: {}")
    FileOperations::move_directory_contents(&dst_base, mod_dir);

    // dropped log site: log("[install] FOMOD installation steps completed in {}")
    Ok(mod_path.to_string())
}

/// Recursively collect mod-relative file paths into `out`.
///
/// Mirror of the `recursive_directory_iterator` block at
/// `InstallationService.cpp:391-398`: regular files only, path made relative to
/// the mod root, backslashes folded to forward slashes.
///
/// Two faithful reproductions worth naming:
/// - The C++ does NOT lowercase these keys, while
///   `FomodDependencyEvaluator::evaluate_file_dependency` normalizes (and thus
///   lowercases) the value it looks up. A mixed-case installed file therefore
///   never matches a file dependency. That is a latent C++ bug; it is
///   reproduced here, not fixed. See PARITY-NOTES "Task 15".
/// - The C++ `fs::relative` RESOLVES symlinks while this strips a prefix
///   lexically, the same divergence already documented for the Task 12
///   installed-file scan.
fn collect_installed_files(root: &Path, dir: &Path, out: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_installed_files(root, &path, out);
        } else if path.is_file()
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.insert(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Read a JSON config, yielding [`Value::Null`] on any failure.
///
/// Mirror of the `ifstream` + `f >> config` blocks at
/// `InstallationService.cpp:187-207` and `:327-341`. A parse error is CAUGHT and
/// warned about in the C++, leaving `config` default-constructed (null); it is
/// NOT fatal, despite `InstallationService.hpp:91` listing "Invalid JSON in
/// selections file" under fatal errors. The code wins over the doc; the
/// doc-vs-code gap is recorded in PARITY-NOTES "Task 15".
fn read_json_config(path: &str) -> Value {
    let Ok(text) = fs::read_to_string(path) else {
        // Unreadable file: the C++ `if (f)` guard skips the parse and leaves
        // config null. Non-UTF-8 lands here too, where nlohmann would have
        // thrown a parse_error and been caught to the same effect.
        return Value::Null;
    };
    match json::parse(&text) {
        Ok(value) => value,
        Err(_) => {
            // dropped log site: log_warning("[install] Failed to parse JSON
            // config {}: {}") / ("[install] Failed to parse FOMOD JSON {}: {}")
            Value::Null
        }
    }
}

/// Resolve the selections JSON path, or an empty string when there is none.
///
/// Mirror of `InstallationService::resolve_json_path`
/// (`InstallationService.cpp:468-494`): an explicit `json_path` is trusted
/// as-is; otherwise a sibling `<archive stem>.json` is accepted only when it
/// really sits beside the archive, guarded by [`is_inside`].
fn resolve_json_path(json_path: &str, archive_path: &str) -> String {
    // Caller provided an explicit path - trust it.
    if !json_path.is_empty() {
        return json_path.to_string();
    }

    let p = Path::new(archive_path);
    let parent = p.parent().unwrap_or(Path::new(""));
    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
    let derived = parent.join(format!("{stem}.json"));

    if derived.exists() {
        if !parent.as_os_str().is_empty() && !is_inside(parent, &derived) {
            // dropped log site: log_warning("[install] Rejecting derived JSON
            // path outside archive directory: {}")
            return String::new();
        }
        return derived.to_string_lossy().into_owned();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("salma-is-{tag}-{}", random_hex_string(8)));
        stdfs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            stdfs::create_dir_all(parent).expect("create parent");
        }
        stdfs::write(path, body).expect("write");
    }

    // -- find_fomod_folder -------------------------------------------------

    #[test]
    fn find_fomod_folder_requires_a_module_config() {
        let root = scratch("ff-none");
        stdfs::create_dir_all(root.join("fomod")).expect("create");
        assert_eq!(
            find_fomod_folder(&root),
            None,
            "empty fomod dir is not a hit"
        );
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn find_fomod_folder_matches_both_names_case_insensitively() {
        let root = scratch("ff-case");
        write(&root.join("FOMOD").join("moduleconfig.XML"), "<config/>");
        assert_eq!(find_fomod_folder(&root), Some(root.join("FOMOD")));
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn find_fomod_folder_descends_into_a_wrapper_directory() {
        let root = scratch("ff-nested");
        write(
            &root.join("Wrapper").join("fomod").join("ModuleConfig.xml"),
            "<config/>",
        );
        assert_eq!(
            find_fomod_folder(&root),
            Some(root.join("Wrapper").join("fomod"))
        );
        stdfs::remove_dir_all(&root).ok();
    }

    /// A `fomod` DIRECTORY holding only a subdirectory named moduleconfig.xml is
    /// not a hit: the C++ requires a regular file.
    #[test]
    fn find_fomod_folder_ignores_a_directory_named_module_config() {
        let root = scratch("ff-dir");
        stdfs::create_dir_all(root.join("fomod").join("ModuleConfig.xml")).expect("create");
        assert_eq!(find_fomod_folder(&root), None);
        stdfs::remove_dir_all(&root).ok();
    }

    // -- resolve_json_path -------------------------------------------------

    #[test]
    fn explicit_json_path_is_trusted_even_when_absent() {
        assert_eq!(
            resolve_json_path(r"C:\anywhere\explicit.json", r"C:\dl\mod.7z"),
            r"C:\anywhere\explicit.json"
        );
    }

    #[test]
    fn derived_json_path_is_used_when_it_sits_beside_the_archive() {
        let root = scratch("json-ok");
        let archive = root.join("mod.7z");
        write(&archive, "x");
        let sidecar = root.join("mod.json");
        write(&sidecar, "{}");
        assert_eq!(
            resolve_json_path("", archive.to_str().unwrap()),
            sidecar.to_string_lossy()
        );
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_sidecar_yields_no_json_path() {
        let root = scratch("json-miss");
        let archive = root.join("mod.7z");
        write(&archive, "x");
        assert_eq!(resolve_json_path("", archive.to_str().unwrap()), "");
        stdfs::remove_dir_all(&root).ok();
    }

    // -- read_json_config --------------------------------------------------

    #[test]
    fn malformed_json_config_reads_as_null_not_an_error() {
        let root = scratch("json-bad");
        let bad = root.join("bad.json");
        write(&bad, "{ this is not json");
        assert!(
            read_json_config(bad.to_str().unwrap()).is_null(),
            "a parse error is caught and warned about, never fatal"
        );
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_json_config_reads_as_null() {
        let root = scratch("json-absent");
        assert!(read_json_config(root.join("nope.json").to_str().unwrap()).is_null());
        stdfs::remove_dir_all(&root).ok();
    }

    // -- install_mod front door -------------------------------------------

    #[test]
    fn missing_archive_reports_the_cpp_message() {
        let mut svc = InstallationService::new();
        let root = scratch("no-archive");
        let archive = root.join("absent.7z");
        let err = svc
            .install_mod(
                archive.to_str().unwrap(),
                root.join("out").to_str().unwrap(),
                "",
            )
            .expect_err("a missing archive is fatal");
        assert_eq!(
            err.0,
            format!("Archive file not found: {}", archive.to_string_lossy())
        );
        stdfs::remove_dir_all(&root).ok();
    }

    // -- moduleName sanitization ------------------------------------------

    /// Separators, parent segments, and reserved device names all CLEAR the
    /// name rather than failing, which only becomes fatal when several
    /// candidate folders need disambiguating.
    #[test]
    fn rejected_module_names_fall_through_to_the_ambiguous_failure() {
        for bad in [
            "sub/dir", r"sub\dir", "..", "up..down", "con", "CON.txt", "lpt9", "nul.json",
        ] {
            let root = scratch("modname");
            let archive = root.join("mod.7z");
            write(&archive, "x");
            write(
                &root.join("mod.json"),
                &format!("{{\"moduleName\": \"{}\"}}", bad.replace('\\', "\\\\")),
            );
            let extracted = root.join("x");
            stdfs::create_dir_all(extracted.join("ModA").join("meshes")).expect("create");
            stdfs::create_dir_all(extracted.join("ModB").join("textures")).expect("create");

            let err = handle_non_fomod_install(
                &extracted,
                root.join("out").to_str().unwrap(),
                archive.to_str().unwrap(),
                "",
            )
            .expect_err("two candidates plus a rejected name is fatal");
            assert_eq!(
                err.0, "Multiple mod folders detected but no moduleName in JSON to disambiguate.",
                "moduleName {bad:?} should have been cleared"
            );
            stdfs::remove_dir_all(&root).ok();
        }
    }

    #[test]
    fn a_usable_module_name_selects_its_folder() {
        let root = scratch("modname-ok");
        let archive = root.join("mod.7z");
        write(&archive, "x");
        write(&root.join("mod.json"), "{\"moduleName\": \"ModB\"}");
        let extracted = root.join("x");
        stdfs::create_dir_all(extracted.join("ModA").join("meshes")).expect("create");
        write(&extracted.join("ModA").join("meshes").join("a.nif"), "A");
        stdfs::create_dir_all(extracted.join("ModB").join("textures")).expect("create");
        write(&extracted.join("ModB").join("textures").join("b.dds"), "B");

        let out = root.join("out");
        handle_non_fomod_install(
            &extracted,
            out.to_str().unwrap(),
            archive.to_str().unwrap(),
            "",
        )
        .expect("named folder resolves");

        assert!(out.join("textures").join("b.dds").exists(), "ModB copied");
        assert!(!out.join("meshes").exists(), "ModA not copied");
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unmatched_module_name_reports_the_cpp_message() {
        let root = scratch("modname-miss");
        let archive = root.join("mod.7z");
        write(&archive, "x");
        write(&root.join("mod.json"), "{\"moduleName\": \"Ghost\"}");
        let extracted = root.join("x");
        stdfs::create_dir_all(extracted.join("ModA").join("meshes")).expect("create");
        stdfs::create_dir_all(extracted.join("ModB").join("textures")).expect("create");

        let err = handle_non_fomod_install(
            &extracted,
            root.join("out").to_str().unwrap(),
            archive.to_str().unwrap(),
            "",
        )
        .expect_err("no match is fatal");
        assert_eq!(err.0, "moduleName 'ghost' did not match any folder.");
        stdfs::remove_dir_all(&root).ok();
    }

    // -- non-FOMOD copy paths ---------------------------------------------

    #[test]
    fn single_candidate_is_copied_without_a_module_name() {
        let root = scratch("single");
        let extracted = root.join("x");
        write(&extracted.join("Wrapper").join("meshes").join("a.nif"), "A");
        let out = root.join("out");

        handle_non_fomod_install(
            &extracted,
            out.to_str().unwrap(),
            root.join("mod.7z").to_str().unwrap(),
            "",
        )
        .expect("single candidate needs no disambiguation");

        assert!(out.join("meshes").join("a.nif").exists());
        stdfs::remove_dir_all(&root).ok();
    }

    /// No recognized content root at all: the whole tree is copied flat.
    #[test]
    fn no_candidate_copies_the_archive_root_flat() {
        let root = scratch("flat");
        let extracted = root.join("x");
        write(&extracted.join("readme.txt"), "hello");
        write(&extracted.join("docs").join("guide.md"), "doc");
        let out = root.join("out");

        handle_non_fomod_install(
            &extracted,
            out.to_str().unwrap(),
            root.join("mod.7z").to_str().unwrap(),
            "",
        )
        .expect("flat copy fallback");

        assert!(out.join("readme.txt").exists());
        assert!(out.join("docs").join("guide.md").exists());
        stdfs::remove_dir_all(&root).ok();
    }

    // -- installed-file scan ----------------------------------------------

    /// The C++ keeps the ORIGINAL casing here (no `to_lower`), so the port must
    /// too, even though it makes mixed-case files unmatchable downstream.
    #[test]
    fn installed_file_scan_preserves_case_and_uses_forward_slashes() {
        let root = scratch("scan");
        write(&root.join("Meshes").join("Armor").join("Cuirass.nif"), "x");
        write(&root.join("top.esp"), "x");

        let mut out = HashSet::new();
        collect_installed_files(&root, &root, &mut out);

        let mut got: Vec<String> = out.into_iter().collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                "Meshes/Armor/Cuirass.nif".to_string(),
                "top.esp".to_string()
            ]
        );
        stdfs::remove_dir_all(&root).ok();
    }
}
