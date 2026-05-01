//! Top-level install orchestrator: extract, detect, dispatch, clean up.
//!
//! Behind the `install` and `installWithConfig` C ABI exports. It extracts the
//! archive to a temp directory, decides whether the tree carries a FOMOD
//! installer, and delegates either to [`crate::fomod_service`] for the replay
//! or to the content-root copy driven by [`crate::mod_structure_detector`].
//!
//! ## Pipeline and what each exit leaves on disk
//!
//! ```text
//!   install_mod
//!     |
//!     FileOperations::reset_disk_full()
//!     |
//!     archive exists?  -- no --> Err "Archive file not found: {path}"
//!     |                          nothing was created
//!     |
//!     create_dir_all(mod_path)  -- Err --> Err "Cannot create mod directory: {e}"
//!     |                                    no temp directory exists yet
//!     |
//!     create_dir_all(%TEMP%/fomod-<8 hex>/archive)
//!     |   |
//!     |   +-- Err --> Err "Cannot create temp directory: {e}"
//!     |               leaks %TEMP%/fomod-<8 hex> if that parent was created first
//!     |
//!     run_install
//!     |   ArchiveService::extract -> <temp>/archive
//!     |   find_fomod_folder
//!     |     +-- Some -> handle_fomod_install: parse ModuleConfig.xml, build the
//!     |     |           dependency context, run the required / optional /
//!     |     |           conditional passes into <temp>/unfomod, then
//!     |     |           move_directory_contents(<temp>/unfomod -> mod_path)
//!     |     +-- None -> handle_non_fomod_install: find_main_mod_folders, then
//!     |                 copy_directory_contents of the chosen content root
//!     |
//!     remove_dir_all(<temp>)   runs on both the Ok and the Err of run_install;
//!     |                        a removal failure only warns
//!     |
//!     run_install failed?  -- yes --> that Err is returned; mod_path holds
//!     |                               nothing this install wrote, because every
//!     |                               fatal exit precedes the copy and move
//!     |                               passes, and those report failure by
//!     |                               logging, not by Err. Anything a previous
//!     |                               install left there survives
//!     |
//!     disk_full_encountered()?  -- yes --> Err "Install aborted: disk full ..."
//!     |                                    <temp> is already gone; mod_path
//!     |                                    keeps whatever the passes copied,
//!     |                                    and nothing is rolled back
//!     |
//!     Ok(mod_path)
//! ```
//!
//! ## Error model
//!
//! Every fatal condition surfaces as an `Err`.
//! [`install_mod`](InstallationService::install_mod) yields
//! `Result<String, InstallError>` and `capi` hands the carried string to the
//! caller unchanged, so those bytes are observable across the ABI.
//!
//! Seven salma-authored failure messages are byte-exact contracts, tabulated in
//! `PARITY-NOTES.md` under "Exceptions-return-Result contract". A failure
//! raised inside a library - archive extraction, XML parsing - carries that
//! back end's own wording instead, so do not assert on those strings.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::archive_service::ArchiveService;
use crate::file_operations::FileOperations;
use crate::fomod_ir_parser::parse_module_config;
use crate::fomod_service::{FomodService, execute_file_operations};
use crate::json::{self, Value};
use crate::logger::Logger;
use crate::mod_structure_detector::find_main_mod_folders;
use crate::types::{FileOperation, FomodDependencyContext};
use crate::utils::{is_inside, random_hex_string, to_lower};

/// A fatal install failure, carrying the message returned across the C ABI.
///
/// `capi::install` and `capi::installWithConfig` hand this string straight back
/// to the caller, so its bytes are observable.
///
/// For the seven salma-authored failures the bytes are the contract, character
/// for character. Failures that originate in a library are not: archive
/// extraction and XML parsing interpolate that back end's own error text, which
/// changes with the dependency. Do not assert on those strings. See the module
/// doc's "Error model" section and `PARITY-NOTES.md`,
/// "Exceptions-return-Result contract".
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

/// The five base-game plugins seeded into every dependency context, in
/// insertion order.
const SEED_PLUGINS: [&str; 5] = [
    "skyrim.esm",
    "update.esm",
    "dawnguard.esm",
    "hearthfires.esm",
    "dragonborn.esm",
];

/// Windows reserved device names rejected as a `moduleName`. Compared against
/// the lowercased stem, so `con.txt` is rejected exactly like `con`.
const RESERVED_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// FOMOD install orchestrator.
///
/// Holds no state. `capi::install` and `capi::installWithConfig` each build an
/// instance inside the call and drop it before returning, so nothing carries
/// from one ABI call to the next.
///
/// Not thread-safe. The struct is empty, so the hazard is not its fields: the
/// disk-full marker [`InstallationService::install_mod`] resets on entry and
/// reads on exit is a process-global atomic in [`FileOperations`]. Two installs
/// running at once would each clear and observe the other's disk pressure, so a
/// disk-full failure could be reported against the wrong install or lost
/// entirely. The server serializes its engine calls with a mutex for this same
/// class of reason; see `SalmaEngine.cpp`.
#[derive(Debug, Default)]
pub struct InstallationService;

impl InstallationService {
    /// Construct a service instance.
    pub fn new() -> Self {
        InstallationService
    }

    /// Install `archive_path` into `mod_path`, optionally driven by a selections
    /// JSON at `json_path`. An empty `json_path` derives one from the archive
    /// stem. Returns `mod_path` on success.
    ///
    /// Lifecycle:
    /// 1. Reset the sticky disk-full marker.
    /// 2. Fail if the archive does not exist.
    /// 3. Create `mod_path` and a fresh `%TEMP%/fomod-<8 hex>/archive`.
    /// 4. Extract, locate a `fomod` folder, dispatch (all inside `run_install`).
    /// 5. Remove the temp tree if step 3 got that far. A removal failure warns
    ///    and does not fail the install.
    /// 6. Turn a disk-full marker into a hard failure.
    ///
    /// **Cleanup.** The single `fs::remove_dir_all(&temp_dir)` runs after
    /// `run_install` returns, on both its exit paths. It does not run at all on
    /// the two earlier returns:
    ///
    /// - A failed `fs::create_dir_all(mod_path)` returns
    ///   `Err("Cannot create mod directory: {e}")` and leaks nothing, because no
    ///   temp directory exists yet.
    /// - A failed `fs::create_dir_all(<temp>/archive)` returns
    ///   `Err("Cannot create temp directory: {e}")` and can leak:
    ///   `create_dir_all` creates `%TEMP%/fomod-<8 hex>` before its `archive`
    ///   child, so the parent directory survives when the child fails. That
    ///   leak is deliberate: both creations sit ahead of the region the cleanup
    ///   covers. An operator who finds orphaned `%TEMP%\fomod-*` directories is
    ///   looking at this path. See PARITY-NOTES.md before moving them.
    ///
    /// `mod_path` is deliberately not cleaned up on failure: once the copy
    /// passes have run, partial content is left for the caller.
    pub fn install_mod(
        &mut self,
        archive_path: &str,
        mod_path: &str,
        json_path: &str,
    ) -> Result<String, InstallError> {
        // Clear the sticky disk-full marker before any work, so a previous
        // install's disk pressure does not poison this run.
        FileOperations::reset_disk_full();

        let logger = Logger::instance();
        let start = Instant::now();

        logger.log("[install] === Starting mod installation ===");
        logger.log(&format!("[install] Archive: {archive_path}"));
        logger.log(&format!("[install] Target mod directory: {mod_path}"));
        logger.log("[install] Initialization finished");

        if !Path::new(archive_path).exists() {
            return Err(InstallError::new(format!(
                "Archive file not found: {archive_path}"
            )));
        }

        // Size is read only to log it; a failure warns, reports 0, and keeps
        // going.
        let archive_size = match fs::metadata(archive_path) {
            Ok(meta) => meta.len(),
            Err(err) => {
                logger.log_warning(&format!("[install] Could not read archive size: {err}"));
                0
            }
        };
        logger.log(&format!(
            "[install] Archive size: {archive_size} bytes ({:.2} MB)",
            archive_size as f64 / 1024.0 / 1024.0
        ));

        // This failure returns before any temp directory exists, so there is
        // nothing to clean up on this path.
        fs::create_dir_all(mod_path)
            .map_err(|e| InstallError::new(format!("Cannot create mod directory: {e}")))?;
        logger.log(&format!("[install] Created mod directory: {mod_path}"));

        let temp_dir = std::env::temp_dir().join(format!("fomod-{}", random_hex_string(8)));
        let archive_extract_dir = temp_dir.join("archive");
        fs::create_dir_all(&archive_extract_dir)
            .map_err(|e| InstallError::new(format!("Cannot create temp directory: {e}")))?;
        logger.log(&format!(
            "[install] Temporary directory: {}",
            temp_dir.display()
        ));

        let outcome = self.run_install(
            archive_path,
            mod_path,
            json_path,
            &temp_dir,
            &archive_extract_dir,
        );

        // Cleanup is symmetric across both exit paths. A removal failure only
        // warns.
        match fs::remove_dir_all(&temp_dir) {
            Ok(()) => logger.log("[install] Cleaned up temporary directory"),
            Err(err) if outcome.is_ok() => logger.log_warning(&format!(
                "[install] WARNING: Failed to cleanup temp directory: {err}"
            )),
            Err(_) => {
                logger.log_warning("[install] Failed to cleanup temp directory on error path")
            }
        }

        logger.log(&format!(
            "[install] Total installation time: {:.2} seconds",
            start.elapsed().as_secs_f64()
        ));

        let result = outcome?;

        // Some files could not be copied because the volume ran out of space.
        // Surface it as a hard failure so a half-empty mod is never reported as
        // installed. The temp tree is already removed above, so returning here
        // leaks nothing.
        if FileOperations::disk_full_encountered() {
            return Err(InstallError::new(
                "Install aborted: disk full while copying files. Free space and retry.",
            ));
        }

        Ok(result)
    }

    /// Extract, find the FOMOD folder, dispatch. Split out of
    /// [`InstallationService::install_mod`] so the temp-directory cleanup runs
    /// on both exit paths without being written twice.
    fn run_install(
        &mut self,
        archive_path: &str,
        mod_path: &str,
        json_path: &str,
        temp_dir: &Path,
        archive_extract_dir: &Path,
    ) -> Result<String, InstallError> {
        let logger = Logger::instance();
        logger.log("[install] Extracting archive...");
        let extract_start = Instant::now();
        let archive_service = ArchiveService::new();
        archive_service
            .extract(
                archive_path,
                archive_extract_dir
                    .to_str()
                    .ok_or_else(|| InstallError::new("Temp directory path is not valid UTF-8"))?,
            )
            // The archive back end's own error text becomes the ABI error
            // string. The failure is the contract; the wording is not.
            .map_err(|e| InstallError::new(e.to_string()))?;
        logger.log(&format!(
            "[install] Archive extracted to {} in {:.2} seconds",
            temp_dir.display(),
            extract_start.elapsed().as_secs_f64()
        ));

        logger.log("[install] Searching for FOMOD folder...");
        let fomod_folder = find_fomod_folder(archive_extract_dir);

        match fomod_folder {
            None => {
                logger.log("[install] No FOMOD folder detected - using standard installation");
                handle_non_fomod_install(archive_extract_dir, mod_path, archive_path, json_path)
            }
            Some(folder) => {
                logger.log(&format!(
                    "[install] FOMOD folder found: {}",
                    folder.display()
                ));
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

/// Find the first directory named `fomod` (case-insensitively) that holds a
/// `moduleconfig.xml` (case-insensitively).
///
/// There is no shallowest-path preference here, unlike the ModuleConfig lookup
/// in [`crate::fomod_inference_service`], which tracks depth and prefers the
/// shallowest hit. For an archive carrying more than one FOMOD, inference and
/// install can therefore disagree about which `ModuleConfig.xml` is
/// authoritative.
///
/// **Traversal order.** The walk is level-then-descend, not pre-order
/// depth-first. It reads one directory, tests every immediate subdirectory, and
/// returns on the first one named `fomod` that holds a `moduleconfig.xml`. Only
/// after that whole loop finishes does it recurse into the collected
/// subdirectories, in `read_dir` order. There is no explicit stack; the
/// recursion plus the `subdirs` vector is what produces this order.
///
/// The order decides the winner whenever `read_dir` returns a wrapper directory
/// before a shallow `fomod`:
///
/// ```text
///   root/
///     Data/
///       fomod/ModuleConfig.xml     <- a pre-order depth-first walk stops here
///     fomod/ModuleConfig.xml       <- this walk stops here
///
///   visit order, this walk:  Data, fomod       -> root/fomod
///   visit order, pre-order:  Data, Data/fomod  -> root/Data/fomod
/// ```
///
/// PARITY-NOTES.md still describes install as taking the first pre-order
/// depth-first hit, which this walk does not do. Correct that entry, or change
/// the walk (move the recursion inside the child loop), before relying on
/// either description.
///
/// Returns `None` when no such folder exists, including when `archive_root`
/// cannot be read.
fn find_fomod_folder(archive_root: &Path) -> Option<PathBuf> {
    // Tests every immediate subdirectory of `dir` first, then descends into
    // them in read_dir order. A directory named `fomod` that has no
    // moduleconfig.xml is still descended into, because `subdirs` collects it
    // unconditionally.
    //
    // `path.is_dir()` follows symlinks and Windows junctions, and this
    // recursion has no visited set and no depth cap, so a link cycle under
    // `archive_root` recurses until the stack overflows. The input is always a
    // freshly extracted temp tree owned by this process, so that is not
    // reachable today.
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

/// Whether `dir` holds a regular file named `moduleconfig.xml`,
/// case-insensitively. A directory of that name does not count.
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
/// One candidate from [`find_main_mod_folders`] is copied without further
/// checks. Several candidates need a `moduleName` from the selections JSON to
/// disambiguate, and no match or several matches is fatal. No candidate at all
/// falls back to copying the archive root flat.
fn handle_non_fomod_install(
    archive_root: &Path,
    mod_path: &str,
    archive_path: &str,
    json_path: &str,
) -> Result<String, InstallError> {
    let logger = Logger::instance();
    logger.log(&format!(
        "[install] No 'fomod' folder found; checking for nested mod structure in: {}",
        archive_root.display()
    ));

    let effective_json = resolve_json_path(json_path, archive_path);
    let mut module_name_lower = String::new();

    if !effective_json.is_empty() && Path::new(&effective_json).exists() {
        let config = read_json_config(&effective_json, "JSON config");
        if let Some(name) = config.get("moduleName").filter(|v| v.is_string())
            && let Some(s) = name.as_str()
        {
            module_name_lower = to_lower(s);
            logger.log(&format!(
                "[install] Detected moduleName \"{module_name_lower}\" in JSON"
            ));
        }
    }

    // Path separators and parent segments clear the value rather than failing.
    // A cleared name then falls through to the ambiguous-folder failure below
    // when several candidates exist.
    if module_name_lower.contains('/')
        || module_name_lower.contains('\\')
        || module_name_lower.contains("..")
    {
        logger.log_warning(&format!(
            "[install] Rejecting moduleName with path separators: \"{module_name_lower}\""
        ));
        module_name_lower.clear();
    }

    // Windows reserved device names cannot be directory names and fail
    // silently, so they are cleared the same way.
    if !module_name_lower.is_empty() {
        let stem = match module_name_lower.rfind('.') {
            Some(dot) => &module_name_lower[..dot],
            None => module_name_lower.as_str(),
        };
        if RESERVED_NAMES.contains(&stem) {
            logger.log_warning(&format!(
                "[install] Rejecting Windows reserved device name: \"{module_name_lower}\""
            ));
            module_name_lower.clear();
        }
    }

    let main_mod_folders = find_main_mod_folders(archive_root);

    if !main_mod_folders.is_empty() {
        let chosen: PathBuf = if main_mod_folders.len() == 1 {
            logger.log(&format!(
                "[install] Only one mod folder \"{}\" found; copying it",
                file_name_of(&main_mod_folders[0])
            ));
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
                    let hit = p
                        .file_name()
                        .map(|n| to_lower(&n.to_string_lossy()) == module_name_lower)
                        .unwrap_or(false);
                    if hit {
                        logger.log(&format!(
                            "[install]      matches moduleName: \"{}\"",
                            file_name_of(p)
                        ));
                    }
                    hit
                })
                .collect();

            if matches.len() != 1 {
                return Err(InstallError::new(if matches.is_empty() {
                    format!("moduleName '{module_name_lower}' did not match any folder.")
                } else {
                    format!("moduleName '{module_name_lower}' matched multiple folders.")
                }));
            }

            logger.log(&format!(
                "[install] Copying contents of chosen mod folder \"{}\"",
                file_name_of(matches[0])
            ));
            matches[0].clone()
        };

        FileOperations::copy_directory_contents(&chosen, Path::new(mod_path));
        return Ok(mod_path.to_string());
    }

    // Fallback: copy everything from the archive root.
    logger.log(&format!(
        "[install] No nested mod structure detected; copying all files from archive \
         root to mod directory: {mod_path}"
    ));
    FileOperations::copy_directory_contents(archive_root, Path::new(mod_path));
    Ok(mod_path.to_string())
}

/// FOMOD install: parse the config, build the dependency context, run the three
/// file passes, then move the staged tree into place.
///
/// The passes stage into `<temp>/unfomod` and only the final
/// `move_directory_contents` touches `mod_path`, so a failure before that point
/// leaves the mod directory as it was.
fn handle_fomod_install(
    fomod_folder: &Path,
    archive_root: &Path,
    mod_path: &str,
    archive_path: &str,
    temp_dir: &Path,
    json_path: &str,
) -> Result<String, InstallError> {
    let logger = Logger::instance();
    let xml_path = fomod_folder.join("ModuleConfig.xml");
    // The join uses the fixed casing `ModuleConfig.xml` even though
    // find_fomod_folder matched case-insensitively. Windows resolves either
    // spelling, so this opens whatever casing the archive shipped.
    //
    // That only holds on a case-insensitive filesystem. On a case-sensitive one
    // an archive shipping `fomod/moduleconfig.xml` passes find_fomod_folder,
    // which lowercases both names, and then fails the fs::read below as
    // "Cannot parse XML (...)". The engine is Windows-only and CI runs on
    // windows-latest, so nothing exercises the other case. Lifting the
    // limitation means having contains_module_config return the matched entry
    // name so this join can reuse the shipped casing.
    let src_base = fomod_folder
        .parent()
        .unwrap_or(fomod_folder)
        .to_string_lossy()
        .into_owned();
    let dst_base = temp_dir.join("unfomod");

    let effective_json = resolve_json_path(json_path, archive_path);

    fs::create_dir_all(&dst_base)
        .map_err(|e| InstallError::new(format!("Cannot create staging directory: {e}")))?;

    // `parse_module_config` detects the encoding from the raw bytes, so the
    // file is read as bytes rather than as a string.
    let bytes =
        fs::read(&xml_path).map_err(|e| InstallError::new(format!("Cannot parse XML ({e})")))?;
    let installer = parse_module_config(&bytes, "")
        // The inner description comes from the XML loader. The failure is the
        // contract; the wording is not.
        .map_err(|e| InstallError::new(format!("Cannot parse XML ({e})")))?;
    logger.log(&format!("[install] Loaded XML: {}", xml_path.display()));

    let config_json = if !effective_json.is_empty() && Path::new(&effective_json).exists() {
        let value = read_json_config(&effective_json, "FOMOD JSON");
        // Logged after the parse attempt, so the line appears even when the
        // parse failed and `value` came back null.
        logger.log(&format!("[install] Loaded JSON: {effective_json}"));
        value
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
            logger.log(&format!(
                "[install] Game path from JSON: {}",
                context.game_path
            ));
        }
        if let Some(v) = config_json.get("gameVersion").filter(|v| v.is_string())
            && let Some(s) = v.as_str()
        {
            context.game_version = s.to_string();
            logger.log(&format!(
                "[install] Game version from JSON: {}",
                context.game_version
            ));
        }
    }

    // Scan the game Data directory for plugins, one level deep.
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
            // The logged count is the whole set, which already holds the five
            // seeded masters, so it overstates what this scan found.
            logger.log(&format!(
                "[install] Found {} plugins in game Data directory",
                context.installed_plugins.len()
            ));
        }
    }

    // Populate installed_files from an existing mod directory, for re-installs.
    let mod_dir = Path::new(mod_path);
    if mod_dir.is_dir() {
        collect_installed_files(mod_dir, mod_dir, &mut context.installed_files);
        if !context.installed_files.is_empty() {
            logger.log(&format!(
                "[install] Scanned {} existing files in mod directory",
                context.installed_files.len()
            ));
        }
    }

    let mut fomod_service = FomodService::new();
    fomod_service.set_installer(installer);

    logger.log("[install] Checking module-level dependencies...");
    if !fomod_service.check_module_dependencies(Some(&context)) {
        return Err(InstallError::new(
            "Module-level dependencies not met - installation cannot proceed",
        ));
    }

    if !config_json.is_null() {
        logger.log("[install] Validating JSON selections...");
        // A step or group `name` of the wrong type aborts the whole install; a
        // merely invalid selection only warns.
        let valid = fomod_service
            .validate_json_selections(&config_json)
            .map_err(|e| InstallError::new(e.to_string()))?;
        if !valid {
            logger.log_warning(
                "[install] WARNING: JSON selections have group-type constraint violations",
            );
        }
    }

    // Caller-owned operations vector and document-order counter: every pass
    // appends to the same vector so one sort orders the whole install.
    let mut file_ops: Vec<FileOperation> = Vec::new();
    let mut next_doc_order: i32 = 0;
    let dst_base_str = dst_base.to_string_lossy().into_owned();

    logger.log("[install] Processing required install files...");
    fomod_service.process_required_files(
        &src_base,
        &dst_base_str,
        &mut file_ops,
        &mut next_doc_order,
    );

    // Even without selections this still installs Required plugins and
    // alwaysInstall / installIfUsable entries from unselected plugins.
    logger.log("[install] Processing optional install files...");
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

    logger.log("[install] Processing conditional file installs...");
    fomod_service.process_conditional_files(
        &src_base,
        &dst_base_str,
        Some(&context),
        &mut file_ops,
        &mut next_doc_order,
    );

    let file_op_failures = execute_file_operations(&mut file_ops);
    if file_op_failures > 0 {
        // Always 0 in practice; see `execute_file_operations`' own doc comment.
        logger.log_warning(&format!(
            "[install] {file_op_failures} file operations failed during FOMOD install"
        ));
    }

    // Move the staged result into the mod directory. `dst_base` lives inside the
    // temp tree that install_mod removes, so nothing needs to survive here.
    logger.log(&format!(
        "[install] Moving unfomod files to mod directory: {mod_path}"
    ));
    FileOperations::move_directory_contents(&dst_base, mod_dir);

    logger.log(&format!(
        "[install] FOMOD installation steps completed in {}",
        temp_dir.display()
    ));
    Ok(mod_path.to_string())
}

/// The file name of `p` as a display string, for the log lines that quote a
/// candidate folder by name.
fn file_name_of(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Recursively collect mod-relative file paths into `out`.
///
/// Regular files only. Keys are relative to `root`, use forward slashes, and
/// keep their on-disk case. `out` is added to, never cleared.
///
/// Keeping the case is deliberate even though it makes some keys unmatchable:
/// `FomodDependencyEvaluator::evaluate_file_dependency` normalizes, and so
/// lowercases, the value it looks up, so a mixed-case installed file never
/// satisfies a file dependency. Lowercasing here would change which files a
/// re-install decides are already present. See PARITY-NOTES.md.
///
/// The prefix is stripped lexically rather than resolved, so a path reached
/// through a link keeps the link's spelling.
///
/// **Traversal.** The recursion descends whenever `path.is_dir()` is true, and
/// `Path::is_dir` resolves through `fs::metadata`, so directory symlinks and
/// Windows junctions are followed and the files behind them land in `out`.
/// There is no visited set and no depth cap, so a link cycle under `root`
/// recurses until the stack overflows, which is fatal inside an FFI call:
/// `capi::guard` catches panics, not stack exhaustion. `find_fomod_folder` has
/// the same shape. Contrast [`FileOperations::copy_folder`], which uses
/// `DirEntry::file_type` and skips linked entries on purpose.
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
/// An unreadable file and a malformed one are both warnings, never fatal: the
/// caller then proceeds as if no selections were supplied. `label` names the
/// config in that warning.
fn read_json_config(path: &str, label: &str) -> Value {
    let Ok(text) = fs::read_to_string(path) else {
        // An unreadable file, non-UTF-8 content included, skips the parse and
        // leaves the config null.
        return Value::Null;
    };
    match json::parse(&text) {
        Ok(value) => value,
        Err(err) => {
            // Two call sites, two labels: "JSON config" on the non-FOMOD path,
            // "FOMOD JSON" on the FOMOD path.
            Logger::instance()
                .log_warning(&format!("[install] Failed to parse {label} {path}: {err}"));
            Value::Null
        }
    }
}

/// Resolve the selections JSON path, or an empty string when there is none.
///
/// Two rules, both weaker than they look:
///
/// - A non-empty `json_path` is returned verbatim. There is no existence check,
///   no extension check, no traversal check and no [`is_inside`] check. All
///   validation of this argument lives in the mo2-server
///   `InstallationController`.
/// - An empty `json_path` derives `<archive stem>.json` next to the archive.
///   `file_stem` strips only the last extension, so `mod.tar.gz` derives
///   `mod.tar.json`. The derived path is returned only if it exists.
///
/// The [`is_inside`] containment guard on the derived path is conditional: it
/// runs only when `archive_path` has a non-empty parent. For a bare filename
/// the parent is empty, the guard is skipped, the derived `<stem>.json` carries
/// no directory component, its existence probe hits the process current working
/// directory, and the path is returned unchecked. Inside a DLL loaded by MO2
/// that working directory is not under this code's control. Deliberate; see
/// PARITY-NOTES.md.
///
/// Returns an empty string when no selections file applies.
fn resolve_json_path(json_path: &str, archive_path: &str) -> String {
    // An explicit path from the caller is trusted as-is.
    if !json_path.is_empty() {
        return json_path.to_string();
    }

    let p = Path::new(archive_path);
    let parent = p.parent().unwrap_or(Path::new(""));
    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
    let derived = parent.join(format!("{stem}.json"));

    if derived.exists() {
        if !parent.as_os_str().is_empty() && !is_inside(parent, &derived) {
            Logger::instance().log_warning(&format!(
                "[install] Rejecting derived JSON path outside archive directory: {}",
                derived.display()
            ));
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

    /// A `fomod` directory holding only a subdirectory named moduleconfig.xml
    /// is not a hit: a regular file is required.
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
            read_json_config(bad.to_str().unwrap(), "JSON config").is_null(),
            "a parse error is caught and warned about, never fatal"
        );
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_json_config_reads_as_null() {
        let root = scratch("json-absent");
        assert!(
            read_json_config(root.join("nope.json").to_str().unwrap(), "JSON config").is_null()
        );
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

    /// Separators, parent segments, and reserved device names all clear the
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

    /// Keys keep their on-disk casing, even though that makes a mixed-case file
    /// unmatchable by a file dependency downstream.
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
