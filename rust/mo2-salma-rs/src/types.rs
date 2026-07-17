//! Shared types - Rust port of the pieces of `src/Types.hpp` the port needs
//! so far. Later tasks add `FileOperation` and `InstallResult` when their
//! consumers arrive.

use std::collections::HashSet;

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
