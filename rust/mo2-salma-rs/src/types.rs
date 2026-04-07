//! Shared types - Rust port of the pieces of `src/Types.hpp` the port needs
//! so far. Later tasks add `FileOperation`, `FomodDependencyContext`, and
//! `InstallResult` when their consumers arrive.

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
