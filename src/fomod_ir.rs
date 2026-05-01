//! FOMOD intermediate representation: the parsed shape of a ModuleConfig.xml.
//!
//! [`crate::fomod_ir_parser`] builds these structures once. The dependency
//! evaluator, the propagator, the forward simulator and the CSP solver all read
//! them, and none of them goes back to the XML.
//!
//! Containment, with the XML element each field comes from:
//!
//! ```text
//! FomodInstaller                                    <config>
//!  |- module_dependencies: Option<FomodCondition>    <moduleDependencies>
//!  |- required_files: Vec<FomodFileEntry>            <requiredInstallFiles>/<file|folder>
//!  |- steps: Vec<FomodStep>                          <installSteps>/<installStep>
//!  |   |- visible: Option<FomodCondition>            <visible>
//!  |   '- groups: Vec<FomodGroup>                    <optionalFileGroups>/<group>
//!  |        '- plugins: Vec<FomodPlugin>             <plugins>/<plugin>
//!  |             |- r#type: PluginType               <typeDescriptor>/<type>
//!  |             |- type_patterns: Vec<FomodTypePattern>
//!  |             |    (condition + result_type)      <dependencyType>/<patterns>/<pattern>
//!  |             |- files: Vec<FomodFileEntry>       <files>/<file|folder>
//!  |             |- condition_flags: Vec<(String, String)>
//!  |             |                                   <conditionFlags>/<flag>
//!  |             '- dependencies: Option<FomodCondition>
//!  |                                                 <dependencies>
//!  '- conditional_patterns: Vec<FomodConditionalPattern>
//!       (condition + files)                          <conditionalFileInstalls>/<patterns>/<pattern>
//! ```
//!
//! [`FomodCondition`] is the one type not tied to a single element. It comes
//! from a `<dependencies>` element (a composite node) or from one of the seven
//! leaf elements `<flagDependency>`, `<fileDependency>`, `<gameDependency>`,
//! `<pluginDependency>`, `<fomodDependency>`, `<fommDependency>` and
//! `<foseDependency>`. The tree above marks the five places a condition hangs
//! off the IR: module dependencies, step visibility, plugin dependencies,
//! type-pattern conditions and conditional-pattern conditions.
//!
//! # Which vectors carry document order
//!
//! Every later stage depends on this split:
//!
//! | Field                                                                 | Order                                            |
//! |-----------------------------------------------------------------------|--------------------------------------------------|
//! | `steps`, `groups`, `plugins`                                          | sorted by the parent element's `order` attribute |
//! | `required_files`, `FomodPlugin::files`, `type_patterns`,              |                                                  |
//! | `condition_flags`, `conditional_patterns`, `FomodCondition::children` | raw XML child order                              |
//!
//! The sort runs through [`crate::utils::get_ordered_nodes`]: an absent `order`
//! attribute and the exact value `"Ascending"` sort by `name` ascending,
//! `"Descending"` sorts by `name` in reverse, and every other value, including
//! `"Explicit"`, keeps XML order. [`FomodStep::ordinal`] records the position
//! after that sort, and the plugin order fixes each plugin's flat index.
//!
//! On the document-order side, file-entry order becomes the `document_order`
//! tiebreaker on [`crate::fomod_atom::FomodAtom`], and `type_patterns` order
//! decides which type override wins.

use crate::types::PluginType;

/// Logical operator for combining child conditions.
///
/// The empty-children case is asymmetric and load-bearing: it is how the IR
/// encodes both "always true" and "always false". See the variant docs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FomodConditionOp {
    /// All child conditions must be true. A composite with no children is
    /// true. This is the operator of `FomodCondition::default()`, so a default
    /// condition is always true, and the parser installs that default whenever
    /// a `<pattern>` carries no `<dependencies>` element.
    #[default]
    And,
    /// At least one child condition must be true. A composite with no children
    /// is false. The parser uses an empty `Or` as its always-false value when
    /// condition nesting exceeds
    /// [`crate::fomod_dependency_evaluator::MAX_DEPENDENCY_DEPTH`].
    Or,
}

/// Parse a `FomodConditionOp` name: exact, case-sensitive match. Any other
/// string, the empty string included, returns `And`.
pub fn parse_condition_op(s: &str) -> FomodConditionOp {
    match s {
        "And" => FomodConditionOp::And,
        "Or" => FomodConditionOp::Or,
        _ => FomodConditionOp::And,
    }
}

/// Map a [`FomodConditionOp`] to the name FOMOD spells it with. Exhaustive, so
/// it always round-trips back through [`parse_condition_op`].
pub fn condition_op_to_string(op: FomodConditionOp) -> &'static str {
    match op {
        FomodConditionOp::And => "And",
        FomodConditionOp::Or => "Or",
    }
}

/// Discriminator for the predicate a leaf condition tests.
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

/// Recursive condition tree node: either a leaf predicate or a composite.
///
/// `r#type` decides which fields mean anything. `Composite` uses `op` and
/// `children` and ignores the leaf fields; every other discriminator uses its
/// leaf fields and ignores `op` and `children`. The leaf fields are
/// `flag_name`, `flag_value`, `file_path`, `file_state`, `version`,
/// `plugin_name`, `plugin_type` and `fomod_name`.
///
/// Shape of a tree, and which fields carry the value for each node kind:
///
/// ```text
/// Composite(op = And)         op + children meaningful, leaf fields ignored
///  |- Flag(flag_name, flag_value)
///  |- File(file_path, file_state)
///  '- Composite(op = Or)
///       |- Plugin(plugin_name, plugin_type)
///       '- Game(version)
/// ```
///
/// Composite truth table, including the two empty cases the whole IR relies on:
///
/// | Node           | Children    | Evaluates to                                                                                    |
/// |----------------|-------------|-------------------------------------------------------------------------------------------------|
/// | Composite(And) | none        | true. This is `FomodCondition::default()`, and what a `<pattern>` with no `<dependencies>` gets |
/// | Composite(Or)  | none        | false. This is the parser's depth-truncation bail                                               |
/// | Composite(And) | one or more | true only when every child is true                                                              |
/// | Composite(Or)  | one or more | true when any child is true                                                                     |
///
/// Hand-built conditions trip over `file_state` and `plugin_type`: the parser
/// defaults both to `"Active"` when the XML attribute is absent, so an empty
/// string never occurs on a parsed tree. An empty `file_state` still behaves
/// like `"Active"`, but the evaluator logs an "Unknown file dependency state"
/// warning for it; an empty `plugin_type` behaves like `"Active"` silently.
///
/// Evaluation lives in [`crate::fomod_dependency_evaluator`], not here.
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
    /// Leaf: required state of the file named by `file_path` (`File`). FOMOD
    /// defines `"Active"` (the parser's default), `"Inactive"` and
    /// `"Missing"`. Any other value warns and is treated as `"Active"`.
    pub file_state: String,
    /// Leaf: version string (`Game`, `Fomm`, `Fose`).
    pub version: String,
    /// Leaf: plugin name (`Plugin`).
    pub plugin_name: String,
    /// Leaf: required activation state of the game plugin file (.esp/.esm)
    /// named by `plugin_name` (`Plugin`). FOMOD defines `"Active"` (the
    /// parser's default) and `"Inactive"`; any other value behaves like
    /// `"Active"`. This is not the FOMOD selection type: that is
    /// [`crate::types::PluginType`], held by `FomodPlugin::r#type`. Two
    /// different concepts with similar names, both in scope in this module.
    pub plugin_type: String,
    /// Leaf: FOMOD package name (`Fomod`).
    pub fomod_name: String,
    /// Composite children.
    pub children: Vec<FomodCondition>,
}

/// One source-to-destination mapping, from a `<file>` or `<folder>` element.
/// Folder entries expand into individual file atoms during install planning.
///
/// The parser normalizes `source` and `destination` with
/// [`crate::utils::normalize_path`]. That normalization is lossy and
/// load-bearing: strings are ASCII-lowercased, backslashes become forward
/// slashes, leading `./` and `/` and the trailing `/` are stripped, repeated
/// slashes collapse, and `.` and `..` segments are dropped. Later stages
/// compare these strings against archive entry names and target-tree keys,
/// normalized the same way, so a case-sensitive or backslash-bearing
/// comparison against either field will not match.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodFileEntry {
    /// Archive-relative source path: the archive prefix joined with the XML
    /// `source` attribute, then normalized (lowercase, forward slashes, no
    /// leading or trailing slash).
    pub source: String,
    /// Mod-relative destination path, normalized the same way as `source`.
    /// For a `<file>` entry the parser first resolves it through
    /// [`crate::utils::resolve_file_destination`], which substitutes the
    /// source filename for an empty destination and appends the source
    /// filename to a destination that ends with a separator.
    pub destination: String,
    /// Overwrite priority; higher values win conflicts (default 0).
    pub priority: i32,
    /// True if this entry came from a `<folder>` element.
    pub is_folder: bool,
    /// Install this entry even when the owning plugin is not selected. Comes
    /// from the XML `alwaysInstall` attribute, which is true only for the
    /// values `"true"` and `"1"` (case-insensitive) and false when absent.
    pub always_install: bool,
    /// Install this entry when the owning plugin is not selected, unless the
    /// plugin's effective type is `NotUsable`. Comes from the XML
    /// `installIfUsable` attribute, parsed with the same true/1 rule as
    /// `always_install`.
    pub install_if_usable: bool,
}

/// A condition that, when met, overrides a plugin's declared type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodTypePattern {
    /// Condition that triggers this override. A `<pattern>` with no
    /// `<dependencies>` child keeps the default here, and that default is an
    /// empty `And` composite, which is always true.
    pub condition: FomodCondition,
    /// Plugin type to apply when the condition is met (default `Optional`).
    pub result_type: PluginType,
}

/// A selectable option within a group, carrying files and condition flags.
///
/// `r#type`, possibly overridden by `type_patterns`, controls the selection
/// constraint. Selecting the plugin adds its `files` to the install plan and
/// sets its `condition_flags` for later condition evaluation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodPlugin {
    /// Display name from the `name` attribute.
    pub name: String,
    /// Base selection type (default `Optional`; may be overridden by
    /// `type_patterns`).
    pub r#type: PluginType,
    /// Conditional type overrides from `<dependencyType>/<patterns>`, in XML
    /// document order. The evaluator takes the first pattern whose condition
    /// is met, so this order decides which override wins.
    pub type_patterns: Vec<FomodTypePattern>,
    /// Files installed when this plugin is selected, in XML document order.
    /// Entries that set `always_install` or `install_if_usable` are also
    /// installed when the plugin is not selected.
    pub files: Vec<FomodFileEntry>,
    /// Flag name/value pairs set when selected, in XML document order. A later
    /// pair with the same name overwrites an earlier one.
    pub condition_flags: Vec<(String, String)>,
    /// Optional prerequisite conditions for this plugin. `None` means the
    /// `<plugin>` has no `<dependencies>` child.
    pub dependencies: Option<FomodCondition>,
}

/// Selection cardinality constraint for a group of plugins.
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

/// Parse a `FomodGroupType` name: exact, case-sensitive match. Any other
/// string, the empty string included, returns `SelectAny`.
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

/// Map a [`FomodGroupType`] to the name FOMOD spells it with. Exhaustive, so
/// it always round-trips back through [`parse_group_type`].
pub fn group_type_to_string(group_type: FomodGroupType) -> &'static str {
    match group_type {
        FomodGroupType::SelectExactlyOne => "SelectExactlyOne",
        FomodGroupType::SelectAtMostOne => "SelectAtMostOne",
        FomodGroupType::SelectAtLeastOne => "SelectAtLeastOne",
        FomodGroupType::SelectAll => "SelectAll",
        FomodGroupType::SelectAny => "SelectAny",
    }
}

/// A named group of plugins sharing one selection cardinality constraint.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodGroup {
    /// Display name from the `name` attribute.
    pub name: String,
    /// Selection cardinality constraint for this group (default `SelectAny`,
    /// which is also what an unrecognized `type` attribute parses to).
    pub r#type: FomodGroupType,
    /// Selectable options within this group, in the order the
    /// `<plugins order="...">` attribute selects. This order fixes each
    /// plugin's flat index across the whole installer.
    pub plugins: Vec<FomodPlugin>,
}

/// One wizard page presented to the user, optionally gated by a visibility
/// condition.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodStep {
    /// Display name from the `name` attribute.
    pub name: String,
    /// Zero-based position in the wizard sequence (default 0). The parser
    /// assigns it after applying the `<installSteps order="...">` attribute,
    /// so it is the presentation position, not the XML child position.
    pub ordinal: i32,
    /// Visibility condition; step is shown only when met. `None` means the
    /// step has no `<visible>` element and is always shown.
    pub visible: Option<FomodCondition>,
    /// Option groups presented on this wizard page, in the order the
    /// `<optionalFileGroups order="...">` attribute selects.
    pub groups: Vec<FomodGroup>,
}

/// Files installed when a condition is met, from `<conditionalFileInstalls>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodConditionalPattern {
    /// Condition evaluated after all wizard steps complete. As on
    /// [`FomodTypePattern::condition`], a `<pattern>` with no `<dependencies>`
    /// child keeps the default empty `And` composite, which is always true.
    pub condition: FomodCondition,
    /// Files installed when the condition is met.
    pub files: Vec<FomodFileEntry>,
}

/// Top-level IR for one FOMOD installer definition: the whole parsed content
/// of a ModuleConfig.xml. Module-level dependencies, unconditionally required
/// files, the ordered wizard steps, and the conditional install patterns
/// evaluated after all steps complete.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodInstaller {
    /// Global prerequisites from `<moduleDependencies>`. `None` means the
    /// `<config>` has no `<moduleDependencies>` element.
    pub module_dependencies: Option<FomodCondition>,
    /// Unconditionally installed files from `<requiredInstallFiles>`, in XML
    /// document order.
    pub required_files: Vec<FomodFileEntry>,
    /// Wizard pages from `<installSteps>`, already sorted by the
    /// `<installSteps order="...">` attribute.
    pub steps: Vec<FomodStep>,
    /// Post-wizard conditional installs from `<conditionalFileInstalls>`, in
    /// XML document order. An `order` attribute never reorders `<pattern>`
    /// children; only steps, groups and plugins are reordered.
    pub conditional_patterns: Vec<FomodConditionalPattern>,
}

/// Total plugin count across all steps and groups. `i32`, to match the flat
/// index arrays the CSP solver sizes from it.
pub fn total_flat_plugins(installer: &FomodInstaller) -> i32 {
    let mut total = 0i32;
    for step in &installer.steps {
        for group in &step.groups {
            total += group.plugins.len() as i32;
        }
    }
    total
}

/// Build a `[step][group]` to flat-plugin-start index map:
/// `result[step_idx][group_idx]` is the flat index of that group's first
/// plugin. Walks steps and groups in IR order, so the indices it hands out
/// agree with [`total_flat_plugins`].
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

    // --- per-field defaults ---

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
        // Matching is exact and case-sensitive; every miss returns And.
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
