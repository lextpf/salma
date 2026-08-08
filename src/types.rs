//! Types shared across the engine: the queued copy operation model, the install
//! outcome, the FOMOD plugin types, and the context the dependency evaluator
//! reads.
//!
//! These are plain data. The behavior that gives them meaning lives in
//! [`crate::fomod_service`], [`crate::file_operations`] and
//! [`crate::fomod_dependency_evaluator`], and the doc comments here point at it
//! where a field carries a precondition those modules do not enforce.

use std::collections::HashSet;

/// Discriminator for file against folder copy operations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FileOpType {
    /// Single file copy.
    #[default]
    File,
    /// Recursive folder copy.
    Folder,
}

/// A single queued file or folder copy operation.
///
/// `priority` controls overwrite order (higher wins) and `document_order` breaks
/// ties by enqueue position, but the two executors read them differently:
/// [`crate::fomod_service::execute_file_operations`] sorts by
/// `(priority, document_order)`, while `FileOperations::execute` sorts by
/// `priority` alone and leans on the stable sort to hold insertion order. The
/// difference changes which file wins a conflict, so check which executor you
/// are feeding before relying on either. See PARITY-NOTES.md.
///
/// Both executors pass `source` and `destination` verbatim to `Path::new`, so a
/// comparison or a prefix test on either field must not assume one path
/// separator. See the two field docs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileOperation {
    /// File or folder.
    pub op_type: FileOpType,
    /// Source path under the extracted archive root. Not normalized:
    /// `fomod_service::enqueue_entry` builds it by joining the OS-native
    /// extraction base with the entry path taken from `ModuleConfig.xml`, so on
    /// Windows one string can carry a backslash base and a forward-slash tail.
    pub source: String,
    /// Destination path under the mod directory. Same convention as `source`:
    /// OS-native at the base, forward-slash in the FOMOD-derived tail, never
    /// normalized.
    pub destination: String,
    /// FOMOD priority attribute (MO2 default: 0).
    pub priority: i32,
    /// Enqueue counter, used as the priority tiebreaker. Not strictly XML
    /// byte-position.
    pub document_order: i32,
}

/// Outcome of a FOMOD install replay attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallResult {
    /// Whether installation completed without error.
    pub success: bool,
    /// Path to the installed mod directory.
    pub mod_path: String,
    /// Error message if `success` is false.
    pub error: String,
}

/// FOMOD plugin type descriptor.
///
/// Maps directly to the `<type>` element values defined by the FOMOD
/// ModuleConfig schema. Controls whether a plugin is auto-selected,
/// user-selectable, or greyed out.
///
/// The default is `Optional`, and two things rely on it:
/// [`crate::utils::parse_plugin_type_string`] returns `Optional` for any name it
/// does not recognise, including the empty string, so an unknown or missing
/// `<type>` is never an error; and `r#type` on
/// [`crate::fomod_ir::FomodPlugin`] plus `result_type` on
/// [`crate::fomod_ir::FomodTypePattern`] inherit it through
/// `#[derive(Default)]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PluginType {
    /// Always installed, cannot be deselected.
    Required,
    /// Pre-selected but user can deselect.
    Recommended,
    /// Not pre-selected, user can select.
    #[default]
    Optional,
    /// Greyed out, cannot be selected.
    NotUsable,
    /// Selectable but the FOMOD installer warns the user before applying.
    CouldBeUsable,
}

/// External state passed to the FOMOD dependency evaluator.
///
/// Carries the environment needed to evaluate `<fileDependency>`,
/// `<gameDependency>` and the other non-flag dependency types. Callers pass
/// `Option<&FomodDependencyContext>`; `None` means the evaluator has no
/// environment to consult.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FomodDependencyContext {
    /// Root path of the game installation.
    pub game_path: String,
    /// Files present in the mod directory. Every entry must already be in
    /// `utils::normalize_path` form, that is lowercase with forward slashes.
    /// The evaluator normalizes the path it looks up and then does a plain set
    /// lookup, so an entry stored in any other form never matches. Nothing
    /// enforces this.
    pub installed_files: HashSet<String>,
    /// Active game plugins (.esp/.esm, lowercase).
    pub installed_plugins: HashSet<String>,
    /// Previously installed FOMOD packages. Matched case-sensitively, unlike
    /// plugin names, which the evaluator lowercases first. Store these exactly
    /// as the FOMOD names them.
    pub installed_fomods: HashSet<String>,
    /// Game version string for comparison.
    pub game_version: String,
    /// Extracted archive root directory.
    pub archive_root: String,
}
