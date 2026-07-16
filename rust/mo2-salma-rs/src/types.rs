//! Shared types - Rust port of the pieces of `src/Types.hpp` the port needs
//! so far. Later tasks add `FileOperation`, `FomodDependencyContext`, and
//! `InstallResult` when their consumers arrive.

/// FOMOD plugin type descriptor. Mirror of `mo2core::PluginType` in
/// `src/Types.hpp`.
///
/// Maps directly to the `<type>` element values defined by the FOMOD
/// ModuleConfig schema. Controls whether a plugin is auto-selected,
/// user-selectable, or greyed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginType {
    /// Always installed, cannot be deselected.
    Required,
    /// Pre-selected but user can deselect.
    Recommended,
    /// Not pre-selected, user can select.
    Optional,
    /// Greyed out, cannot be selected.
    NotUsable,
    /// Selectable but the FOMOD installer warns the user before applying.
    CouldBeUsable,
}
