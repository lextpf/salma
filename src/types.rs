//! Shared types - Rust port of `src/Types.hpp`.

use std::collections::HashSet;

/// Discriminator for file vs folder copy operations. Mirror of
/// `mo2core::FileOpType`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FileOpType {
    /// Single file copy.
    #[default]
    File,
    /// Recursive folder copy.
    Folder,
}

/// A single queued file or folder copy operation. Mirror of
/// `mo2core::FileOperation`.
///
/// `priority` controls overwrite order (higher wins); `document_order` breaks
/// ties using enqueue position. Note that the two executors differ in how they
/// use these: [`crate::fomod_service::execute_file_operations`] sorts by
/// `(priority, document_order)`, while `FileOperations::execute` sorts by
/// `priority` alone and relies on the stable sort to preserve insertion order.
/// See PARITY-NOTES "Task 14".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileOperation {
    /// File or folder.
    pub op_type: FileOpType,
    /// Source path under the extracted archive root.
    pub source: String,
    /// Destination path under the mod directory.
    pub destination: String,
    /// FOMOD priority attribute (MO2 default: 0).
    pub priority: i32,
    /// Enqueue counter, used as the priority tiebreaker. Not strictly XML
    /// byte-position.
    pub document_order: i32,
}

/// Outcome of a FOMOD install/replay attempt. Mirror of
/// `mo2core::InstallResult`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallResult {
    /// Whether installation completed without error.
    pub success: bool,
    /// Path to the installed mod directory.
    pub mod_path: String,
    /// Error message if `success` is false.
    pub error: String,
}

/// FOMOD plugin type descriptor. Mirror of `mo2core::PluginType` in
/// `src/Types.hpp`.
///
/// Maps directly to the `<type>` element values defined by the FOMOD
/// ModuleConfig schema. Controls whether a plugin is auto-selected,
/// user-selectable, or greyed out.
///
/// `Default` is `Optional`, matching both the C++ `enum_map<PluginType>`
/// default value (lookup miss -> `Optional`) and the in-struct field defaults
/// of `FomodPlugin::type` / `FomodTypePattern::result_type` in
/// `src/FomodIR.hpp`.
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

/// External state passed to the FOMOD dependency evaluator. Mirror of
/// `mo2core::FomodDependencyContext` in `src/Types.hpp`.
///
/// Provides the environment needed to evaluate `<fileDependency>`,
/// `<gameDependency>`, and other non-flag dependency types. The C++ callers
/// pass this by nullable pointer; Rust callers pass
/// `Option<&FomodDependencyContext>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FomodDependencyContext {
    /// Root path of the game installation.
    pub game_path: String,
    /// Files present in the mod directory (normalized: lowercase,
    /// forward-slash).
    pub installed_files: HashSet<String>,
    /// Active game plugins (.esp/.esm, lowercase).
    pub installed_plugins: HashSet<String>,
    /// Previously installed FOMOD packages. Matched case-sensitively (the
    /// C++ evaluator does NOT lowercase fomod names, unlike plugin names).
    pub installed_fomods: HashSet<String>,
    /// Game version string for comparison.
    pub game_version: String,
    /// Extracted archive root directory.
    pub archive_root: String,
}
