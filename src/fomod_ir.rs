/*!
 * @brief Defines the parsed FOMOD intermediate representation.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The model preserves document order, condition trees, file priorities, plugin types, and
 * condition flags for inference and install replay.
 */

use crate::types::PluginType;

/**
 * @enum FomodConditionOp
 * @brief Logical operator for combining child conditions.
 * @author Alex (<https://github.com/lextpf>)
 *
 * An empty And is true. An empty Or is false.
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FomodConditionOp {
    /**
     * @brief All child conditions must be true.
     * @author Alex (<https://github.com/lextpf>)
     */
    #[default]
    And,
    /**
     * @brief At least one child condition must be true.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The parser uses an empty `Or` as its always-false value when condition nesting exceeds
     * [`crate::fomod_dependency_evaluator::MAX_DEPENDENCY_DEPTH`].
     */
    Or,
}

/**
 * @fn `parse_condition_op(&str) -> FomodConditionOp`
 * @brief Default empty or unrecognized input to And.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Recognized names require exact case.
 */
pub fn parse_condition_op(s: &str) -> FomodConditionOp {
    match s {
        "And" => FomodConditionOp::And,
        "Or" => FomodConditionOp::Or,
        _ => FomodConditionOp::And,
    }
}

pub fn condition_op_to_string(op: FomodConditionOp) -> &'static str {
    match op {
        FomodConditionOp::And => "And",
        FomodConditionOp::Or => "Or",
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FomodConditionType {
    Flag,
    File,
    Game,
    Plugin,
    Fomod,
    Fomm,
    Fose,
    #[default]
    Composite,
}

/**
 * @struct FomodCondition
 * @brief Recursive condition tree node: either a leaf predicate or a composite.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The parser defaults absent `file_state` and `plugin_type` values to `"Active"`. direct
 * constructors must set both fields to match a parsed tree.
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodCondition {
    pub r#type: FomodConditionType,
    pub op: FomodConditionOp,
    pub flag_name: String,
    pub flag_value: String,
    pub file_path: String,
    /**
     * @brief Leaf: required state of the file named by file_path (File).
     * @author Alex (<https://github.com/lextpf>)
     */
    pub file_state: String,
    pub version: String,
    pub plugin_name: String,
    /**
     * @brief Require an activation state for the named game plugin.
     * @author Alex (<https://github.com/lextpf>)
     */
    pub plugin_type: String,
    pub fomod_name: String,
    pub children: Vec<FomodCondition>,
}

/**
 * @struct FomodFileEntry
 * @brief One source-to-destination mapping, from a file or folder element.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodFileEntry {
    pub source: String,
    /**
     * @brief Mod-relative destination path, normalized the same way as source.
     * @author Alex (<https://github.com/lextpf>)
     *
     * For a `<file>` entry the parser first resolves it through
     * [`crate::utils::resolve_file_destination`], which substitutes the source filename for an
     * empty destination and appends the source filename to a destination that ends with a
     * separator.
     */
    pub destination: String,
    /**
     * @brief Overwrite priority; higher values win conflicts (default 0).
     * @author Alex (<https://github.com/lextpf>)
     */
    pub priority: i32,
    pub is_folder: bool,
    /**
     * @brief Install this entry even when the owning plugin is not selected.
     * @author Alex (<https://github.com/lextpf>)
     */
    pub always_install: bool,
    /**
     * @brief Install when deselected unless the effective type is NotUsable.
     * @author Alex (<https://github.com/lextpf>)
     */
    pub install_if_usable: bool,
}

/**
 * @struct FomodTypePattern
 * @brief A condition that, when met, overrides a plugin's declared type.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodTypePattern {
    /**
     * @brief Condition that triggers this override.
     * @author Alex (<https://github.com/lextpf>)
     *
     * A `<pattern>` with no `<dependencies>` child keeps the default here, and that default is an
     * empty `And` composite, which is always true.
     */
    pub condition: FomodCondition,
    pub result_type: PluginType,
}

/**
 * @struct FomodPlugin
 * @brief A selectable option within a group, carrying files and condition flags.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodPlugin {
    pub name: String,
    pub r#type: PluginType,
    /**
     * @brief Conditional type overrides from dependencyType/patterns, in XML document order.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The evaluator takes the first pattern whose condition is met, so this order decides which
     * override wins.
     */
    pub type_patterns: Vec<FomodTypePattern>,
    /**
     * @brief Files installed when this plugin is selected, in XML document order.
     * @author Alex (<https://github.com/lextpf>)
     */
    pub files: Vec<FomodFileEntry>,
    /**
     * @brief Flag name/value pairs set when selected, in XML document order.
     * @author Alex (<https://github.com/lextpf>)
     */
    pub condition_flags: Vec<(String, String)>,
    pub dependencies: Option<FomodCondition>,
}

/**
 * @enum FomodGroupType
 * @brief Selection cardinality constraint for a group of plugins.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FomodGroupType {
    /**
     * @brief User must select exactly one plugin in the group.
     * @author Alex (<https://github.com/lextpf>)
     */
    SelectExactlyOne,
    SelectAtMostOne,
    /**
     * @brief User must select one or more plugins (at least one required).
     * @author Alex (<https://github.com/lextpf>)
     */
    SelectAtLeastOne,
    SelectAll,
    #[default]
    SelectAny,
}

/**
 * @fn `parse_group_type(&str) -> FomodGroupType`
 * @brief Default empty or unrecognized input to SelectAny.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Recognized names require exact case.
 */
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

pub fn group_type_to_string(group_type: FomodGroupType) -> &'static str {
    match group_type {
        FomodGroupType::SelectExactlyOne => "SelectExactlyOne",
        FomodGroupType::SelectAtMostOne => "SelectAtMostOne",
        FomodGroupType::SelectAtLeastOne => "SelectAtLeastOne",
        FomodGroupType::SelectAll => "SelectAll",
        FomodGroupType::SelectAny => "SelectAny",
    }
}

/**
 * @struct FomodGroup
 * @brief A named group of plugins sharing one selection cardinality constraint.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodGroup {
    pub name: String,
    pub r#type: FomodGroupType,
    /**
     * @brief Selectable options within this group, in the order the plugins attribute selects.
     * @author Alex (<https://github.com/lextpf>)
     *
     * This order fixes each plugin's flat index across the whole installer.
     */
    pub plugins: Vec<FomodPlugin>,
}

/**
 * @struct FomodStep
 * @brief One wizard page presented to the user, optionally gated by a visibility condition.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodStep {
    pub name: String,
    /**
     * @brief Zero-based position in the wizard sequence (default 0).
     * @author Alex (<https://github.com/lextpf>)
     *
     * The parser assigns it after applying the `<installSteps order="...">` attribute, so it is the
     * presentation position, not the XML child position.
     */
    pub ordinal: i32,
    pub visible: Option<FomodCondition>,
    pub groups: Vec<FomodGroup>,
}

/**
 * @struct FomodConditionalPattern
 * @brief Files installed when a condition is met, from conditionalFileInstalls.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodConditionalPattern {
    /**
     * @brief Condition evaluated after all wizard steps complete.
     * @author Alex (<https://github.com/lextpf>)
     *
     * As on [`FomodTypePattern::condition`], a `<pattern>` with no `<dependencies>` child keeps the
     * default empty `And` composite, which is always true.
     */
    pub condition: FomodCondition,
    pub files: Vec<FomodFileEntry>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FomodInstaller {
    pub module_dependencies: Option<FomodCondition>,
    /**
     * @brief Unconditionally installed files from requiredInstallFiles, in XML document order.
     * @author Alex (<https://github.com/lextpf>)
     */
    pub required_files: Vec<FomodFileEntry>,
    /**
     * @brief Wizard pages from installSteps, already sorted by the installSteps attribute.
     * @author Alex (<https://github.com/lextpf>)
     */
    pub steps: Vec<FomodStep>,
    /**
     * @brief Post-wizard conditional installs from conditionalFileInstalls, in XML document order.
     * @author Alex (<https://github.com/lextpf>)
     *
     * An `order` attribute never reorders `<pattern>` children; only steps, groups and plugins are
     * reordered.
     */
    pub conditional_patterns: Vec<FomodConditionalPattern>,
}

/**
 * @fn `total_flat_plugins(&FomodInstaller) -> i32`
 * @brief Use i32 because the CSP solver stores flat indices as i32.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
pub fn total_flat_plugins(installer: &FomodInstaller) -> i32 {
    let mut total = 0i32;
    for step in &installer.steps {
        for group in &step.groups {
            total += group.plugins.len() as i32;
        }
    }
    total
}

/**
 * @fn `compute_flat_starts(&FomodInstaller) -> Vec<Vec<i32>>`
 * @brief Assign global plugin indices in step, group, then plugin order.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
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

    #[test]
    fn parse_condition_op_exact_matches() {
        assert_eq!(parse_condition_op("And"), FomodConditionOp::And);
        assert_eq!(parse_condition_op("Or"), FomodConditionOp::Or);
    }

    #[test]
    fn parse_condition_op_miss_defaults_to_and() {
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
