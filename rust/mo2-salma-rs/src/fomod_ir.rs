//! FOMOD intermediate representation - Rust port of `src/FomodIR.hpp`.
//!
//! Strongly-typed IR mirroring the FOMOD ModuleConfig XML schema. The XML is
//! parsed once (see [`crate::fomod_ir_parser`]) into these structures, which
//! the dependency evaluator, forward simulator, and CSP solver consume without
//! re-reading XML.
//!
//! Hierarchy (struct -> XML element):
//!
//! | Struct                    | XML element                                |
//! |---------------------------|--------------------------------------------|
//! | [`FomodInstaller`]        | `<config>`                                 |
//! | [`FomodStep`]             | `<installStep>`                            |
//! | [`FomodGroup`]            | `<group>`                                  |
//! | [`FomodPlugin`]           | `<plugin>`                                 |
//! | [`FomodFileEntry`]        | `<file>` / `<folder>`                      |
//! | [`FomodCondition`]        | `<dependencies>` / `<pattern>`             |
//! | [`FomodConditionalPattern`] | `<pattern>` in `<conditionalFileInstalls>` |
//!
//! Field names, variant sets, and per-field defaults are kept identical to the
//! C++ structs (fields named `type` in C++ become `r#type` here).

use crate::types::PluginType;

/// Logical operator for combining child conditions. Mirror of
/// `mo2core::FomodConditionOp`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FomodConditionOp {
    /// All child conditions must be true (logical conjunction).
    #[default]
    And,
    /// At least one child condition must be true (logical disjunction).
    Or,
}

/// Parse a `FomodConditionOp` name. Mirror of the C++
/// `enum_map<FomodConditionOp>` specialization in `src/FomodIR.hpp` used via
/// `parse_enum`: exact case-sensitive string match, lookup miss (including the
/// empty string) returns the map default `And`.
pub fn parse_condition_op(s: &str) -> FomodConditionOp {
    match s {
        "And" => FomodConditionOp::And,
        "Or" => FomodConditionOp::Or,
        _ => FomodConditionOp::And,
    }
}

/// Map a [`FomodConditionOp`] to its FOMOD name. Mirror of the C++
/// `enum_to_string<FomodConditionOp>`. Every variant is in the map, so the
/// C++ `"Unknown"` miss value is unreachable; the exhaustive match encodes
/// that directly.
pub fn condition_op_to_string(op: FomodConditionOp) -> &'static str {
    match op {
        FomodConditionOp::And => "And",
        FomodConditionOp::Or => "Or",
    }
}

/// Discriminator for the predicate a leaf condition tests. Mirror of
/// `mo2core::FomodConditionType`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FomodConditionType {
    /// `<flagDependency>` - tests a user-set condition flag.
    Flag,
    /// `<fileDependency>` - tests whether a file is Active/Inactive/Missing.
    File,
    /// `<gameDependency>` - tests the game version.
    Game,
    /// `<pluginDependency>` - tests whether a game plugin (.esp/.esm) is active.
    Plugin,
    /// `<fomodDependency>` - tests whether a named FOMOD package is installed.
    Fomod,
    /// `<fommDependency>` - tests the FOMM version.
    Fomm,
    /// `<foseDependency>` - tests the script extender (FOSE/SKSE/etc.) version.
    Fose,
    /// `<dependencies>` - composite node combining children with And/Or.
    #[default]
    Composite,
}

/// Recursive condition tree node - either a leaf predicate or a composite.
/// Mirror of `mo2core::FomodCondition`.
///
/// When `r#type != Composite`, the leaf fields (`flag_name`, `flag_value`,
/// `file_path`, `version`, `plugin_name`, `fomod_name`) are populated
/// according to the discriminator and `op` / `children` are ignored. When
/// `r#type == Composite`, only `op` and `children` are meaningful and the
/// leaf fields are ignored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodCondition {
    /// Discriminator; default `Composite`.
    pub r#type: FomodConditionType,
    /// Combining operator for composite nodes; default `And`.
    pub op: FomodConditionOp,
    /// Leaf: flag name (`Flag`).
    pub flag_name: String,
    /// Leaf: flag value (`Flag`).
    pub flag_value: String,
    /// Leaf: file path (`File`).
    pub file_path: String,
    /// Leaf: Active, Inactive, Missing (`File`).
    pub file_state: String,
    /// Leaf: version string (`Game`, `Fomm`, `Fose`).
    pub version: String,
    /// Leaf: plugin name (`Plugin`).
    pub plugin_name: String,
    /// Leaf: Active/Inactive (`Plugin`).
    pub plugin_type: String,
    /// Leaf: FOMOD package name (`Fomod`).
    pub fomod_name: String,
    /// Composite children.
    pub children: Vec<FomodCondition>,
}

/// A single source-to-destination file or folder mapping. Mirror of
/// `mo2core::FomodFileEntry`.
///
/// Represents a `<file>` or `<folder>` element. Folder entries are expanded
/// into individual file atoms during installation planning.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodFileEntry {
    /// Archive-relative source path (normalized, with archive prefix).
    pub source: String,
    /// Mod-relative destination path (normalized).
    pub destination: String,
    /// Overwrite priority; higher values win conflicts (default 0).
    pub priority: i32,
    /// True if this entry came from a `<folder>` element.
    pub is_folder: bool,
    /// XML `alwaysInstall` attribute.
    pub always_install: bool,
    /// XML `installIfUsable` attribute.
    pub install_if_usable: bool,
}

/// A condition that, when met, overrides a plugin's declared type. Mirror of
/// `mo2core::FomodTypePattern`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodTypePattern {
    /// Condition that triggers this override.
    pub condition: FomodCondition,
    /// Plugin type to apply when the condition is met (default `Optional`).
    pub result_type: PluginType,
}

/// A selectable option within a group, carrying files and condition flags.
/// Mirror of `mo2core::FomodPlugin`.
///
/// `r#type` (possibly overridden by `type_patterns`) controls selection
/// constraints. When selected, the plugin's `files` are added to the install
/// plan and its `condition_flags` are set for downstream condition evaluation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodPlugin {
    /// Display name from the `name` attribute.
    pub name: String,
    /// Base selection type (default `Optional`; may be overridden by
    /// `type_patterns`).
    pub r#type: PluginType,
    /// Conditional type overrides from `<dependencyType>/<patterns>`.
    pub type_patterns: Vec<FomodTypePattern>,
    /// Files installed when this plugin is selected.
    pub files: Vec<FomodFileEntry>,
    /// Flag name/value pairs set when selected.
    pub condition_flags: Vec<(String, String)>,
    /// Optional prerequisite conditions for this plugin.
    pub dependencies: Option<FomodCondition>,
}

/// Selection cardinality constraint for a group of plugins. Mirror of
/// `mo2core::FomodGroupType`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FomodGroupType {
    /// User must select exactly one plugin in the group.
    SelectExactlyOne,
    /// User may select zero or one plugin (radio + none).
    SelectAtMostOne,
    /// User must select one or more plugins (at least one required).
    SelectAtLeastOne,
    /// All plugins are force-selected; user cannot deselect any.
    SelectAll,
    /// User may select any combination, including none (checkboxes).
    #[default]
    SelectAny,
}

/// Parse a `FomodGroupType` name. Mirror of the C++
/// `enum_map<FomodGroupType>` specialization in `src/FomodIR.hpp` used via
/// `parse_enum`: exact case-sensitive string match, lookup miss (including
/// the empty string) returns the map default `SelectAny`.
pub fn parse_group_type(s: &str) -> FomodGroupType {
    match s {
        "SelectExactlyOne" => FomodGroupType::SelectExactlyOne,
        "SelectAtMostOne" => FomodGroupType::SelectAtMostOne,
        "SelectAtLeastOne" => FomodGroupType::SelectAtLeastOne,
        "SelectAll" => FomodGroupType::SelectAll,
        "SelectAny" => FomodGroupType::SelectAny,
        _ => FomodGroupType::SelectAny,
    }
}

/// Map a [`FomodGroupType`] to its FOMOD name. Mirror of the C++
/// `enum_to_string<FomodGroupType>`. Every variant is in the map, so the
/// C++ `"Unknown"` miss value is unreachable; the exhaustive match encodes
/// that directly.
pub fn group_type_to_string(group_type: FomodGroupType) -> &'static str {
    match group_type {
        FomodGroupType::SelectExactlyOne => "SelectExactlyOne",
        FomodGroupType::SelectAtMostOne => "SelectAtMostOne",
        FomodGroupType::SelectAtLeastOne => "SelectAtLeastOne",
        FomodGroupType::SelectAll => "SelectAll",
        FomodGroupType::SelectAny => "SelectAny",
    }
}

/// A named group of plugins sharing a selection cardinality constraint.
/// Mirror of `mo2core::FomodGroup`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodGroup {
    /// Display name from the `name` attribute.
    pub name: String,
    /// Selection cardinality constraint for this group (default `SelectAny`).
    pub r#type: FomodGroupType,
    /// Selectable options within this group.
    pub plugins: Vec<FomodPlugin>,
}

/// One wizard page presented to the user, optionally gated by a visibility
/// condition. Mirror of `mo2core::FomodStep`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodStep {
    /// Display name from the `name` attribute.
    pub name: String,
    /// Zero-based position in the wizard sequence (default 0).
    pub ordinal: i32,
    /// Visibility condition; step is shown only when met.
    pub visible: Option<FomodCondition>,
    /// Option groups presented on this wizard page.
    pub groups: Vec<FomodGroup>,
}

/// Files installed when a condition is met, from `<conditionalFileInstalls>`.
/// Mirror of `mo2core::FomodConditionalPattern`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodConditionalPattern {
    /// Condition evaluated after all wizard steps complete.
    pub condition: FomodCondition,
    /// Files installed when the condition is met.
    pub files: Vec<FomodFileEntry>,
}

/// Top-level IR for a complete FOMOD installer definition. Mirror of
/// `mo2core::FomodInstaller`.
///
/// Holds the full parsed content of a ModuleConfig.xml: module-level
/// dependencies, unconditionally required files, the ordered wizard steps,
/// and any conditional install patterns evaluated after all steps complete.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodInstaller {
    /// Global prerequisites from `<moduleDependencies>`.
    pub module_dependencies: Option<FomodCondition>,
    /// Unconditionally installed files from `<requiredInstallFiles>`.
    pub required_files: Vec<FomodFileEntry>,
    /// Ordered wizard pages from `<installSteps>`.
    pub steps: Vec<FomodStep>,
    /// Post-wizard conditional installs from `<conditionalFileInstalls>`.
    pub conditional_patterns: Vec<FomodConditionalPattern>,
}

/// Count the total number of plugins across all steps and groups. Mirror of
/// `mo2core::total_flat_plugins`; returns `i32` like the C++ `int`, suitable
/// for sizing flat index arrays in the CSP solver.
pub fn total_flat_plugins(installer: &FomodInstaller) -> i32 {
    let mut total = 0i32;
    for step in &installer.steps {
        for group in &step.groups {
            total += group.plugins.len() as i32;
        }
    }
    total
}

/// Build a `[step][group]` -> flat plugin start index map. Mirror of
/// `mo2core::compute_flat_starts`: `result[step_idx][group_idx]` is the flat
/// index of that group's first plugin.
pub fn compute_flat_starts(installer: &FomodInstaller) -> Vec<Vec<i32>> {
    let mut flat_starts = Vec::with_capacity(installer.steps.len());
    let mut flat = 0i32;
    for step in &installer.steps {
        let mut row = Vec::with_capacity(step.groups.len());
        for group in &step.groups {
            row.push(flat);
            flat += group.plugins.len() as i32;
        }
        flat_starts.push(row);
    }
    flat_starts
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- defaults (must match the C++ in-struct initializers) ---

    #[test]
    fn condition_default_is_composite_and_with_no_children() {
        let c = FomodCondition::default();
        assert_eq!(c.r#type, FomodConditionType::Composite);
        assert_eq!(c.op, FomodConditionOp::And);
        assert!(c.children.is_empty());
        assert!(c.flag_name.is_empty());
        assert!(c.file_state.is_empty());
    }

    #[test]
    fn file_entry_defaults() {
        let e = FomodFileEntry::default();
        assert_eq!(e.priority, 0);
        assert!(!e.is_folder);
        assert!(!e.always_install);
        assert!(!e.install_if_usable);
    }

    #[test]
    fn plugin_and_type_pattern_default_to_optional() {
        assert_eq!(FomodPlugin::default().r#type, PluginType::Optional);
        assert_eq!(
            FomodTypePattern::default().result_type,
            PluginType::Optional
        );
    }

    #[test]
    fn group_defaults_to_select_any_and_step_ordinal_zero() {
        assert_eq!(FomodGroup::default().r#type, FomodGroupType::SelectAny);
        let s = FomodStep::default();
        assert_eq!(s.ordinal, 0);
        assert!(s.visible.is_none());
    }

    // --- enum string maps (exact case-sensitive matching, default on miss) ---

    #[test]
    fn parse_condition_op_exact_matches() {
        assert_eq!(parse_condition_op("And"), FomodConditionOp::And);
        assert_eq!(parse_condition_op("Or"), FomodConditionOp::Or);
    }

    #[test]
    fn parse_condition_op_miss_defaults_to_and() {
        // C++ EnumStringMap::from_string is an exact case-sensitive
        // comparison; every miss returns the map default (And).
        assert_eq!(parse_condition_op(""), FomodConditionOp::And);
        assert_eq!(parse_condition_op("or"), FomodConditionOp::And);
        assert_eq!(parse_condition_op("OR"), FomodConditionOp::And);
        assert_eq!(parse_condition_op("and"), FomodConditionOp::And);
        assert_eq!(parse_condition_op("Xor"), FomodConditionOp::And);
        assert_eq!(parse_condition_op(" Or"), FomodConditionOp::And);
    }

    #[test]
    fn parse_group_type_exact_matches() {
        assert_eq!(
            parse_group_type("SelectExactlyOne"),
            FomodGroupType::SelectExactlyOne
        );
        assert_eq!(
            parse_group_type("SelectAtMostOne"),
            FomodGroupType::SelectAtMostOne
        );
        assert_eq!(
            parse_group_type("SelectAtLeastOne"),
            FomodGroupType::SelectAtLeastOne
        );
        assert_eq!(parse_group_type("SelectAll"), FomodGroupType::SelectAll);
        assert_eq!(parse_group_type("SelectAny"), FomodGroupType::SelectAny);
    }

    #[test]
    fn parse_group_type_miss_defaults_to_select_any() {
        assert_eq!(parse_group_type(""), FomodGroupType::SelectAny);
        assert_eq!(parse_group_type("selectall"), FomodGroupType::SelectAny);
        assert_eq!(parse_group_type("SELECTALL"), FomodGroupType::SelectAny);
        assert_eq!(parse_group_type("SelectSome"), FomodGroupType::SelectAny);
        assert_eq!(parse_group_type("SelectAll "), FomodGroupType::SelectAny);
    }

    #[test]
    fn enum_to_string_round_trips() {
        for op in [FomodConditionOp::And, FomodConditionOp::Or] {
            assert_eq!(parse_condition_op(condition_op_to_string(op)), op);
        }
        for gt in [
            FomodGroupType::SelectExactlyOne,
            FomodGroupType::SelectAtMostOne,
            FomodGroupType::SelectAtLeastOne,
            FomodGroupType::SelectAll,
            FomodGroupType::SelectAny,
        ] {
            assert_eq!(parse_group_type(group_type_to_string(gt)), gt);
        }
    }

    // --- flat index helpers ---

    fn installer_with_counts(counts: &[&[usize]]) -> FomodInstaller {
        let mut installer = FomodInstaller::default();
        for group_counts in counts {
            let mut step = FomodStep::default();
            for &n in group_counts.iter() {
                let mut group = FomodGroup::default();
                group.plugins.resize_with(n, FomodPlugin::default);
                step.groups.push(group);
            }
            installer.steps.push(step);
        }
        installer
    }

    #[test]
    fn total_flat_plugins_empty_installer_is_zero() {
        assert_eq!(total_flat_plugins(&FomodInstaller::default()), 0);
        assert!(compute_flat_starts(&FomodInstaller::default()).is_empty());
    }

    #[test]
    fn total_flat_plugins_sums_all_groups() {
        let installer = installer_with_counts(&[&[2, 3], &[1], &[0, 4]]);
        assert_eq!(total_flat_plugins(&installer), 10);
    }

    #[test]
    fn compute_flat_starts_walks_steps_and_groups_in_order() {
        let installer = installer_with_counts(&[&[2, 3], &[1], &[0, 4]]);
        let starts = compute_flat_starts(&installer);
        assert_eq!(starts, vec![vec![0, 2], vec![5], vec![6, 6]]);
    }
}
