/*!
 * @brief replays a parsed FOMOD selection into file operations.
 * @author Alex (https://github.com/lextpf)
 *
 * required, selected, automatic, and conditional passes append to one queue. execution sorts by
 * priority and document order, so the last write wins.
 *
 * ### :material-alert-circle-outline: failure handling
 *
 * malformed step or group names fail the install. plugin entries accept both supported selection
 * schemas. copy errors are logged and do not propagate.
 *
 * @warning destination screening normalizes the path before the raw Path::join. a rooted
 * destination can replace the installation base.
 */

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::file_operations::FileOperations;
use crate::fomod_dependency_evaluator::{evaluate_condition, evaluate_plugin_type};
use crate::fomod_ir::{
    FomodCondition, FomodFileEntry, FomodGroupType, FomodInstaller, FomodPlugin,
    group_type_to_string,
};
use crate::json::Value;
use crate::logger::Logger;
use crate::types::{FileOpType, FileOperation, FomodDependencyContext, PluginType};
use crate::utils::{is_safe_destination, to_lower};

/**
 * @enum SelectionsError
 * @brief the one malformed-input condition that aborts a selections walk.
 * @author Alex (https://github.com/lextpf)
 *
 * an unusable step or group `name` is an error rather than an empty name, because it fails the
 * whole install: `capi::install` propagates it.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionsError {
    NameTypeError,
}

impl std::fmt::Display for SelectionsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionsError::NameTypeError => {
                f.write_str("selections JSON: step/group entry is not an object, or its \"name\" is not a string")
            }
        }
    }
}

impl std::error::Error for SelectionsError {}

/**
 * @struct FomodService
 * @brief FOMOD processing, install replay, and plugin flag evaluation.
 * @author Alex (https://github.com/lextpf)
 *
 * not thread-safe; use one instance per installation. discard it after an optional-selection error
 * because plugin flags do not roll back.
 *
 * | pass | source                    | queued entries                         |
 * |------|---------------------------|----------------------------------------|
 * | 1    | bound JSON plugins        | all entries after dependency checks    |
 * | 1b   | bound IR groups           | required plugins not handled by pass 1 |
 * | 2    | IR steps absent from JSON | remaining required plugins             |
 * | 3    | all IR steps              | remaining automatic or usable entries  |
 *
 * each pass sees flags written by earlier passes. an error rolls back queued operations and
 * document order, but not plugin flags.
 */
#[derive(Debug, Clone, Default)]
pub struct FomodService {
    installer: FomodInstaller,
    plugin_flags: HashMap<String, String>,
}

impl FomodService {
    pub fn new() -> Self {
        FomodService::default()
    }

    /**
     * @fn set_installer(&mut self, FomodInstaller)
     * @brief establish the IR required by later processing methods.
     * @author Alex (https://github.com/lextpf)
     *
     * must be called before any other method.
     */
    pub fn set_installer(&mut self, installer: FomodInstaller) {
        self.installer = installer;
    }

    pub fn installer(&self) -> &FomodInstaller {
        &self.installer
    }

    pub fn plugin_flags(&self) -> &HashMap<String, String> {
        &self.plugin_flags
    }

    /**
     * @fn check_module_dependencies(&self, Option<&FomodDependencyContext>) -> bool
     * @brief treat absent module dependencies as satisfied.
     * @author Alex (https://github.com/lextpf)
     *
     * never fails: the dependency evaluator is total.
     * @return `true` when the dependencies are met or absent.
     */
    pub fn check_module_dependencies(&self, context: Option<&FomodDependencyContext>) -> bool {
        let Some(condition) = &self.installer.module_dependencies else {
            Logger::instance().log("[fomod] No module-level dependencies found");
            return true;
        };

        // the unmet line goes through `log` with an "ERROR:" prefix in its text, not through
        // `log_error`, so it stays at info level.
        Logger::instance().log("[fomod] Checking module-level dependencies...");
        let met = evaluate_condition(condition, &self.plugin_flags, context);

        if !met {
            Logger::instance().log(
                "[fomod] ERROR: Module-level dependencies not met - installation cannot proceed",
            );
            return false;
        }

        Logger::instance().log("[fomod] Module-level dependencies satisfied");
        true
    }

    /**
     * @fn process_required_files(&self, &str, &str, &mut Vec<FileOperation>, &mut i32)
     * @brief skip unsafe destinations and advance order only for queued operations.
     * @author Alex (https://github.com/lextpf)
     *
     * unsafe destinations are skipped by `enqueue_entry`, and `next_doc_order` advances once per
     * enqueued operation.
     */
    pub fn process_required_files(
        &self,
        src_base: &str,
        dst_base: &str,
        ops: &mut Vec<FileOperation>,
        next_doc_order: &mut i32,
    ) {
        if self.installer.required_files.is_empty() {
            Logger::instance().log("[fomod] No required install files found");
            return;
        }

        Logger::instance().log(&format!(
            "[fomod] Processing {} required install files...",
            self.installer.required_files.len()
        ));

        // the tallies exist only for the log line below; an entry counts only when it actually
        // produced an operation.
        let mut file_count = 0;
        let mut folder_count = 0;
        for entry in &self.installer.required_files {
            let before = ops.len();
            enqueue_entry(entry, src_base, dst_base, ops, next_doc_order);
            if ops.len() > before {
                if entry.is_folder {
                    folder_count += 1;
                } else {
                    file_count += 1;
                }
            }
        }

        Logger::instance().log(&format!(
            "[fomod] Queued {file_count} files and {folder_count} folders from required install files"
        ));
    }

    /**
     * @fn validate_json_selections(&self, &Value) -> Result<bool, SelectionsError>
     * @brief report every violation and treat a missing steps array as valid.
     * @author Alex (https://github.com/lextpf)
     *
     * validation never stops early, so the log lists every violation.
     * @return `Ok(false)` when any group violates its constraint.
     */
    pub fn validate_json_selections(&self, config_json: &Value) -> Result<bool, SelectionsError> {
        let Some(steps) = config_json.get("steps").filter(|s| s.is_array()) else {
            Logger::instance().log("[fomod] No steps in JSON - validation skipped");
            return Ok(true);
        };

        Logger::instance().log("[fomod] Validating JSON selections against FOMOD schema...");

        // build step -> group -> set(plugin name) from the JSON. a nested entry appears only once a
        // readable plugin name lands in it, so a group with no readable plugins is never validated
        // at all.
        let mut selected_plugins: HashMap<String, HashMap<String, HashSet<String>>> =
            HashMap::new();

        for json_step in array_items(Some(steps)) {
            let Some(step_name) = name_field(json_step) else {
                return Err(SelectionsError::NameTypeError);
            };
            let step_name = step_name.to_string();

            let Some(groups) = json_step.get("groups").filter(|g| g.is_array()) else {
                continue;
            };
            for json_group in array_items(Some(groups)) {
                let Some(group_name) = name_field(json_group) else {
                    return Err(SelectionsError::NameTypeError);
                };
                let group_name = group_name.to_string();

                let Some(plugins) = json_group.get("plugins").filter(|p| p.is_array()) else {
                    continue;
                };
                for json_plugin in array_items(Some(plugins)) {
                    let plugin_name = read_plugin_name(json_plugin);
                    if plugin_name.is_empty() {
                        continue;
                    }
                    selected_plugins
                        .entry(step_name.clone())
                        .or_default()
                        .entry(group_name.clone())
                        .or_default()
                        .insert(plugin_name);
                }
            }
        }

        // validate against the IR. lookups are by name only: unlike process_optional_files there is
        // no occurrence matching here, so two IR steps sharing a name both see the same selection
        // set.
        let mut all_valid = true;
        for step in &self.installer.steps {
            let Some(step_sel) = selected_plugins.get(&step.name) else {
                continue;
            };
            for group in &step.groups {
                let Some(sel) = step_sel.get(&group.name) else {
                    continue;
                };

                let selected_in_group = group
                    .plugins
                    .iter()
                    .filter(|plugin| sel.contains(&plugin.name))
                    .count() as i32;

                // warn about selections the IR group does not contain. log-only: it has no effect
                // on the return value, and the warning order is unspecified because `sel` is a hash
                // set.
                for sel_name in sel {
                    if !group.plugins.iter().any(|plugin| &plugin.name == sel_name) {
                        Logger::instance().log_warning(&format!(
                            "[fomod] Group \"{}\": selected plugin \"{sel_name}\" not found in group",
                            group.name
                        ));
                    }
                }

                let type_str = group_type_to_string(group.r#type);
                if !validate_cardinality(
                    group.r#type,
                    selected_in_group,
                    group.plugins.len() as i32,
                ) {
                    all_valid = false;
                    Logger::instance().log(&format!(
                        "[fomod] WARNING: Group \"{}\" type \"{type_str}\" validation failed: {selected_in_group} selected, {} total (note: Required plugins are auto-installed and may not appear in JSON selections)",
                        group.name,
                        group.plugins.len()
                    ));
                } else {
                    Logger::instance().log(&format!(
                        "[fomod] Group \"{}\" type \"{type_str}\": {selected_in_group}/{} plugins selected - VALID",
                        group.name,
                        group.plugins.len()
                    ));
                }
            }
        }

        Logger::instance().log(&format!(
            "[fomod] JSON selections validated: {}",
            if all_valid {
                "ALL VALID"
            } else {
                "SOME VIOLATIONS"
            }
        ));
        Ok(all_valid)
    }

    pub fn process_optional_files(
        &mut self,
        config_json: &Value,
        src_base: &str,
        dst_base: &str,
        context: Option<&FomodDependencyContext>,
        ops: &mut Vec<FileOperation>,
        next_doc_order: &mut i32,
    ) -> Result<(), SelectionsError> {
        let initial_ops_size = ops.len();
        let initial_doc_order = *next_doc_order;

        // no steps array: nothing to do and no rollback to apply. the total line is still logged on
        // this path.
        let Some(steps) = config_json.get("steps").filter(|s| s.is_array()) else {
            Logger::instance().log("[fomod] No valid steps in JSON - optional selections skipped");
            Logger::instance().log(&format!(
                "[fomod] Total file operations queued from optional files: {}",
                ops.len()
            ));
            return Ok(());
        };

        let result = self.process_optional_files_inner(
            steps,
            src_base,
            dst_base,
            context,
            ops,
            next_doc_order,
        );

        if let Err(err) = result {
            // roll the whole call back, log, then propagate.
            ops.truncate(initial_ops_size);
            *next_doc_order = initial_doc_order;
            Logger::instance().log_error(&format!(
                "[fomod] Exception during optional file processing, rolled back queued operations: {err}"
            ));
            // the error path skips the total line below.
            return Err(err);
        }

        Logger::instance().log(&format!(
            "[fomod] Total file operations queued from optional files: {}",
            ops.len()
        ));
        result
    }

    // the guarded body of FomodService::process_optional_files.
    // split out so the caller can apply the rollback on any `Err` return.
    fn process_optional_files_inner(
        &mut self,
        steps: &Value,
        src_base: &str,
        dst_base: &str,
        context: Option<&FomodDependencyContext>,
        ops: &mut Vec<FileOperation>,
        next_doc_order: &mut i32,
    ) -> Result<(), SelectionsError> {
        let FomodService {
            installer,
            plugin_flags,
        } = self;

        let mut processed_plugins: HashSet<String> = HashSet::new();

        Logger::instance().log(&format!(
            "[fomod] Processing optional files from JSON with {} step(s)",
            array_items(Some(steps)).len()
        ));

        let mut ir_steps_by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for (si, step) in installer.steps.iter().enumerate() {
            ir_steps_by_name
                .entry(step.name.as_str())
                .or_default()
                .push(si);
        }
        let mut step_occurrence: HashMap<String, usize> = HashMap::new();

        // pass 1 applies explicit JSON selections.
        for json_step in array_items(Some(steps)) {
            let Some(step_name) = name_field(json_step) else {
                return Err(SelectionsError::NameTypeError);
            };
            let step_name = step_name.to_string();
            Logger::instance().log(&format!("[fomod] Processing step: \"{step_name}\""));

            // a step with no groups array still consumes an occurrence of its name before skipping.
            if !json_step.get("groups").is_some_and(Value::is_array) {
                Logger::instance()
                    .log(&format!("[fomod] Step \"{step_name}\" has no groups array"));
                post_increment(&mut step_occurrence, &step_name);
                continue;
            }

            // post-increment, then bind to the occ-th IR step of that name.
            let occ = post_increment(&mut step_occurrence, &step_name);
            let ir_step_idx = ir_steps_by_name
                .get(step_name.as_str())
                .and_then(|indices| indices.get(occ))
                .copied();

            // a missing IR step does not consume a second occurrence.
            let Some(ir_step_idx) = ir_step_idx else {
                Logger::instance().log_warning(&format!(
                    "[fomod] Could not find IR step \"{step_name}\" occurrence {occ}"
                ));
                continue;
            };
            let ir_step = &installer.steps[ir_step_idx];

            // step visibility gates everything below, including the per-step Required auto-install
            // pass.
            if step_hidden(&ir_step.visible, plugin_flags, context) {
                Logger::instance().log(&format!(
                    "[fomod] Skipping step \"{step_name}\" due to unmet visibility dependencies"
                ));
                continue;
            }

            let mut ir_groups_by_name: HashMap<&str, Vec<usize>> = HashMap::new();
            for (gi, group) in ir_step.groups.iter().enumerate() {
                ir_groups_by_name
                    .entry(group.name.as_str())
                    .or_default()
                    .push(gi);
            }
            let mut group_occurrence: HashMap<String, usize> = HashMap::new();

            Logger::instance().log(&format!(
                "[fomod] Step has {} group(s)",
                array_items(json_step.get("groups")).len()
            ));

            for json_group in array_items(json_step.get("groups")) {
                let Some(group_name) = name_field(json_group) else {
                    return Err(SelectionsError::NameTypeError);
                };
                let group_name = group_name.to_string();
                Logger::instance().log(&format!("[fomod] Processing group: \"{group_name}\""));

                // same shape as the step branch: no plugins array still consumes an occurrence.
                if !json_group.get("plugins").is_some_and(Value::is_array) {
                    Logger::instance().log(&format!(
                        "[fomod] Group \"{group_name}\" has no plugins array"
                    ));
                    post_increment(&mut group_occurrence, &group_name);
                    continue;
                }

                let gocc = post_increment(&mut group_occurrence, &group_name);
                // unlike a missing IR step, a missing IR group does not skip the group: the plugin
                // loop still runs and still advances the plugin occurrence counters, every lookup
                // misses.
                let ir_group = ir_groups_by_name
                    .get(group_name.as_str())
                    .and_then(|indices| indices.get(gocc))
                    .map(|&gi| &ir_step.groups[gi]);

                // plugin name index for the bound group.
                let mut ir_plugins_by_name: HashMap<&str, Vec<usize>> = HashMap::new();
                if let Some(group) = ir_group {
                    for (pi, plugin) in group.plugins.iter().enumerate() {
                        ir_plugins_by_name
                            .entry(plugin.name.as_str())
                            .or_default()
                            .push(pi);
                    }
                }
                let mut plugin_occurrence: HashMap<String, usize> = HashMap::new();

                Logger::instance().log(&format!(
                    "[fomod] Group has {} plugin(s)",
                    array_items(json_group.get("plugins")).len()
                ));

                for json_plugin in array_items(json_group.get("plugins")) {
                    // tolerant of both schemas. an unreadable entry is skipped before the
                    // occurrence counter moves.
                    let plugin_name = read_plugin_name(json_plugin);
                    if plugin_name.is_empty() {
                        Logger::instance()
                            .log_warning("[fomod] Skipping plugin entry with no readable name");
                        continue;
                    }

                    Logger::instance().log(&format!(
                        "[fomod] Looking for plugin: \"{plugin_name}\" in step \"{step_name}\", group \"{group_name}\""
                    ));

                    let pocc = post_increment(&mut plugin_occurrence, &plugin_name);
                    let ir_plugin = ir_group.and_then(|group| {
                        ir_plugins_by_name
                            .get(plugin_name.as_str())
                            .and_then(|indices| indices.get(pocc))
                            .map(|&pi| &group.plugins[pi])
                    });

                    // a miss is log-only; the loop continues.
                    let Some(ir_plugin) = ir_plugin else {
                        Logger::instance().log_error(&format!(
                            "[fomod] Could not find plugin \"{plugin_name}\" in step/group IR"
                        ));
                        continue;
                    };

                    // plugin-level dependencies gate processing.
                    if let Some(dependencies) = &ir_plugin.dependencies
                        && !evaluate_condition(dependencies, plugin_flags, context)
                    {
                        Logger::instance().log(&format!(
                            "[fomod] Skipping plugin \"{plugin_name}\" due to unmet dependencies"
                        ));
                        continue;
                    }

                    Logger::instance().log(&format!(
                        "[fomod] Plugin \"{plugin_name}\" (explicitly selected)"
                    ));

                    apply_condition_flags(ir_plugin, plugin_flags);
                    enqueue_plugin_files(ir_plugin, src_base, dst_base, ops, next_doc_order);
                    processed_plugins.insert(make_plugin_key(
                        &step_name,
                        &group_name,
                        &plugin_name,
                    ));
                }
            }

            // pass 1b: auto-install Required plugins for this step. the key uses the JSON step
            // name, which the binding makes identical to the IR step's name, plus the IR group and
            // plugin names.
            for group in &ir_step.groups {
                for plugin in &group.plugins {
                    let key = make_plugin_key(&step_name, &group.name, &plugin.name);
                    if processed_plugins.contains(&key) {
                        continue;
                    }
                    if evaluate_plugin_type(plugin, plugin_flags, context) == PluginType::Required {
                        Logger::instance().log(&format!(
                            "[fomod] Auto-installing Required plugin: \"{}\"",
                            plugin.name
                        ));
                        apply_condition_flags(plugin, plugin_flags);
                        enqueue_plugin_files(plugin, src_base, dst_base, ops, next_doc_order);
                        processed_plugins.insert(key);
                    }
                }
            }
        }

        // pass 2 applies Required plugins in steps not covered by the JSON.
        // covered_steps uses names, so one JSON step covers every same-named IR step.
        let mut covered_steps: HashSet<&str> = HashSet::new();
        for json_step in array_items(Some(steps)) {
            let Some(step_name) = name_field(json_step) else {
                return Err(SelectionsError::NameTypeError);
            };
            covered_steps.insert(step_name);
        }

        for step in &installer.steps {
            if covered_steps.contains(step.name.as_str()) {
                continue;
            }
            if step_hidden(&step.visible, plugin_flags, context) {
                continue;
            }
            for group in &step.groups {
                for plugin in &group.plugins {
                    let key = make_plugin_key(&step.name, &group.name, &plugin.name);
                    if processed_plugins.contains(&key) {
                        continue;
                    }
                    if evaluate_plugin_type(plugin, plugin_flags, context) == PluginType::Required {
                        Logger::instance().log(&format!(
                            "[fomod] Auto-installing Required plugin: \"{}\"",
                            plugin.name
                        ));
                        apply_condition_flags(plugin, plugin_flags);
                        enqueue_plugin_files(plugin, src_base, dst_base, ops, next_doc_order);
                        processed_plugins.insert(key);
                    }
                }
            }
        }

        // pass 3 applies automatic entries from unselected plugins.
        // visibility uses the flags produced by passes 1 and 2.
        let mut auto_file_count = 0;
        for step in &installer.steps {
            if step_hidden(&step.visible, plugin_flags, context) {
                continue;
            }
            for group in &step.groups {
                for plugin in &group.plugins {
                    let key = make_plugin_key(&step.name, &group.name, &plugin.name);
                    if processed_plugins.contains(&key) {
                        continue;
                    }

                    let eff_type = evaluate_plugin_type(plugin, plugin_flags, context);
                    // required plugins were installed whole by passes 1 and 2.
                    if eff_type == PluginType::Required {
                        continue;
                    }

                    for entry in &plugin.files {
                        let should_install = entry.always_install
                            || (entry.install_if_usable && eff_type != PluginType::NotUsable);
                        if !should_install {
                            continue;
                        }
                        // an entry counts, and logs, only when it actually produced an operation.
                        let before = ops.len();
                        enqueue_entry(entry, src_base, dst_base, ops, next_doc_order);
                        if ops.len() > before {
                            auto_file_count += 1;
                            Logger::instance().log(&format!(
                                "[fomod] Auto-installing {} ({}): {} -> {}",
                                if entry.is_folder { "folder" } else { "file" },
                                if entry.always_install {
                                    "alwaysInstall"
                                } else {
                                    "installIfUsable"
                                },
                                entry.source,
                                entry.destination
                            ));
                        }
                    }
                }
            }
        }

        if auto_file_count > 0 {
            Logger::instance().log(&format!(
                "[fomod] Auto-install pass: {auto_file_count} alwaysInstall/installIfUsable file(s)"
            ));
        }

        Ok(())
    }

    pub fn process_conditional_files(
        &self,
        src_base: &str,
        dst_base: &str,
        context: Option<&FomodDependencyContext>,
        ops: &mut Vec<FileOperation>,
        next_doc_order: &mut i32,
    ) {
        if self.installer.conditional_patterns.is_empty() {
            Logger::instance().log("[fomod] No conditional file install patterns found");
            return;
        }

        let total = self.installer.conditional_patterns.len();
        Logger::instance().log(&format!(
            "[fomod] Processing {total} conditional file install patterns..."
        ));

        // the tallies exist only to be logged. `processed` is incremented before the per-pattern
        // line, so it reads as a 1-based "N of total" counter over the patterns that actually
        // matched.
        let mut processed = 0;
        let mut skipped = 0;
        for pattern in &self.installer.conditional_patterns {
            if !evaluate_condition(&pattern.condition, &self.plugin_flags, context) {
                skipped += 1;
                Logger::instance().log("[fomod] Skipping pattern due to unmet dependencies");
                continue;
            }

            processed += 1;
            Logger::instance().log(&format!(
                "[fomod] Processing conditional pattern {processed}/{total}"
            ));

            for entry in &pattern.files {
                enqueue_entry(entry, src_base, dst_base, ops, next_doc_order);
            }
        }

        Logger::instance().log(&format!(
            "[fomod] Queued {processed} conditional patterns, skipped {skipped} patterns"
        ));
    }
}

// empty sources and destinations rejected by the string screen enqueue nothing.
// the raw destination is joined after screening; a rooted path can replace the base.
fn enqueue_entry(
    entry: &FomodFileEntry,
    src_base: &str,
    dst_base: &str,
    ops: &mut Vec<FileOperation>,
    next_doc_order: &mut i32,
) {
    if entry.source.is_empty() {
        return;
    }

    if !is_safe_destination(&entry.destination) {
        Logger::instance().log_warning(&format!(
            "[fomod] Skipping path-traversal destination: {}",
            entry.destination
        ));
        return;
    }

    let src_path = Path::new(src_base).join(&entry.source);
    let dst_path = Path::new(dst_base).join(&entry.destination);
    let op_type = if entry.is_folder {
        FileOpType::Folder
    } else {
        FileOpType::File
    };

    ops.push(FileOperation {
        op_type,
        source: src_path.to_string_lossy().into_owned(),
        destination: dst_path.to_string_lossy().into_owned(),
        priority: entry.priority,
        document_order: *next_doc_order,
    });
    *next_doc_order += 1;
}

// enqueue every file entry of a plugin, in document order.
fn enqueue_plugin_files(
    plugin: &FomodPlugin,
    src_base: &str,
    dst_base: &str,
    ops: &mut Vec<FileOperation>,
    next_doc_order: &mut i32,
) {
    for entry in &plugin.files {
        enqueue_entry(entry, src_base, dst_base, ops, next_doc_order);
    }
}

// build the composite dedup key for a plugin: to_lower(step) + "\x1f" + to_lower(group) + "\x1f" +
// to_lower(plugin).
// the separator is ASCII Unit Separator (0x1F).
fn make_plugin_key(step: &str, group: &str, plugin: &str) -> String {
    format!(
        "{}\x1f{}\x1f{}",
        to_lower(step),
        to_lower(group),
        to_lower(plugin)
    )
}

// copy a plugin's conditionFlags into the flag map, skipping entries whose flag name is empty.
// called from passes 1, 1b and 2, never from pass 3.
fn apply_condition_flags(plugin: &FomodPlugin, flags: &mut HashMap<String, String>) {
    for (name, value) in &plugin.condition_flags {
        if !name.is_empty() {
            flags.insert(name.clone(), value.clone());
        }
    }
}

fn step_hidden(
    visible: &Option<FomodCondition>,
    flags: &HashMap<String, String>,
    context: Option<&FomodDependencyContext>,
) -> bool {
    match visible {
        Some(condition) => !evaluate_condition(condition, flags, context),
        None => false,
    }
}

fn post_increment(counters: &mut HashMap<String, usize>, name: &str) -> usize {
    let slot = counters.entry(name.to_string()).or_insert(0);
    let before = *slot;
    *slot += 1;
    before
}

// schema-tolerant plugin name reader.
// unlike a step or group name, an unreadable plugin entry never fails the install.
fn read_plugin_name(entry: &Value) -> String {
    if let Some(s) = entry.as_str() {
        return s.to_string();
    }
    // `Value::get` yields `None` for a non-object, so this one lookup covers the non-object,
    // missing-key and wrong-type cases alike.
    if let Some(name) = entry.get("name").and_then(Value::as_str) {
        return name.to_string();
    }
    String::new()
}

// read a step or group name, returning None when the value is unusable.
// every call site turns `None` into `SelectionsError::NameTypeError`, which fails the install.
fn name_field(src: &Value) -> Option<&str> {
    if !src.is_object() {
        return None;
    }
    match src.get("name") {
        None => Some(""),
        Some(value) => value.as_str(),
    }
}

// borrow the elements of a JSON array value; an empty slice for anything else (including None).
fn array_items(value: Option<&Value>) -> &[Value] {
    match value {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

fn validate_cardinality(group_type: FomodGroupType, selected: i32, total: i32) -> bool {
    match group_type {
        FomodGroupType::SelectExactlyOne => selected == 1,
        FomodGroupType::SelectAtLeastOne => selected >= 1,
        FomodGroupType::SelectAtMostOne => selected <= 1,
        FomodGroupType::SelectAll => selected == total,
        FomodGroupType::SelectAny => true,
    }
}

/**
 * @fn execute_file_operations(&mut Vec<FileOperation>) -> i32
 * @brief resolve conflicts by priority, then by global document order.
 * @author Alex (https://github.com/lextpf)
 *
 * `document_order` is a single sequence over the whole install: `enqueue_entry` post-increments it
 * once per queued operation while the required, optional and conditional passes append in that
 * order, so "highest `document_order`" means "enqueued latest".
 *
 * @return 0; the copy backend does not expose per-operation failures.
 */
pub fn execute_file_operations(ops: &mut Vec<FileOperation>) -> i32 {
    execute_file_operations_with(ops, |op| {
        match op.op_type {
            FileOpType::File => {
                FileOperations::copy_file(Path::new(&op.source), Path::new(&op.destination));
            }
            FileOpType::Folder => {
                FileOperations::copy_folder(Path::new(&op.source), Path::new(&op.destination));
            }
        }
        // the copy back end cannot report a failure.
        false
    })
}

// the sort-execute-clear body of execute_file_operations, with the copy back end injected (true
// marks a failed operation) so tests can observe the executed order and the failure counting
// without touching disk.
fn execute_file_operations_with<F>(ops: &mut Vec<FileOperation>, mut copy: F) -> i32
where
    F: FnMut(&FileOperation) -> bool,
{
    // priority ascending, then XML document order as the tiebreaker. higher priority is copied
    // later and overwrites, which is MO2's rule.
    ops.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(a.document_order.cmp(&b.document_order))
    });

    Logger::instance().log(&format!(
        "[fomod] Executing {} file operations in priority order...",
        ops.len()
    ));

    let mut failed = 0i32;
    for op in ops.iter() {
        if copy(op) {
            failed += 1;
            // the back end reports failure as a bool and carries no message, so this line names the
            // operation and nothing else.
            Logger::instance().log_error(&format!(
                "[fomod] Failed to execute file operation: {} -> {}",
                op.source, op.destination
            ));
        }
    }

    // `ops.len()` is read before the clear below, so the warning reports the batch size rather than
    // 0.
    if failed > 0 {
        Logger::instance().log_warning(&format!(
            "[fomod] {failed} of {} file operations failed",
            ops.len()
        ));
    }

    ops.clear();
    failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_ir::{FomodConditionType, FomodGroup, FomodStep, FomodTypePattern};

    fn entry(source: &str, destination: &str) -> FomodFileEntry {
        FomodFileEntry {
            source: source.to_string(),
            destination: destination.to_string(),
            ..FomodFileEntry::default()
        }
    }

    fn plugin(name: &str, files: Vec<FomodFileEntry>) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            files,
            ..FomodPlugin::default()
        }
    }

    fn group(name: &str, group_type: FomodGroupType, plugins: Vec<FomodPlugin>) -> FomodGroup {
        FomodGroup {
            name: name.to_string(),
            r#type: group_type,
            plugins,
        }
    }

    fn step(name: &str, groups: Vec<FomodGroup>) -> FomodStep {
        FomodStep {
            name: name.to_string(),
            groups,
            ..FomodStep::default()
        }
    }

    fn installer(steps: Vec<FomodStep>) -> FomodInstaller {
        FomodInstaller {
            steps,
            ..FomodInstaller::default()
        }
    }

    fn flag_leaf(name: &str, value: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Flag,
            flag_name: name.to_string(),
            flag_value: value.to_string(),
            ..FomodCondition::default()
        }
    }

    fn named(name: &str, key: &str, items: Vec<Value>) -> Value {
        let mut v = Value::object();
        v.insert("name", Value::string(name));
        v.insert(key, Value::Array(items));
        v
    }

    fn json_step(name: &str, groups: Vec<Value>) -> Value {
        named(name, "groups", groups)
    }

    fn json_group(name: &str, plugins: Vec<Value>) -> Value {
        named(name, "plugins", plugins)
    }

    fn json_plugin_v2(name: &str) -> Value {
        let mut v = Value::object();
        v.insert("name", Value::string(name));
        v.insert("selected", Value::Bool(true));
        v
    }

    fn selections(steps: Vec<Value>) -> Value {
        let mut v = Value::object();
        v.insert("steps", Value::Array(steps));
        v
    }

    fn run_optional(
        ir: FomodInstaller,
        config: &Value,
    ) -> Result<(FomodService, Vec<FileOperation>), SelectionsError> {
        let mut service = FomodService::new();
        service.set_installer(ir);
        let mut ops = Vec::new();
        let mut next_doc_order = 0;
        service.process_optional_files(
            config,
            "src",
            "dst",
            None,
            &mut ops,
            &mut next_doc_order,
        )?;
        Ok((service, ops))
    }

    fn sources(ops: &[FileOperation]) -> Vec<String> {
        ops.iter().map(|op| op.source.clone()).collect()
    }

    fn joined(base: &str, rel: &str) -> String {
        Path::new(base).join(rel).to_string_lossy().into_owned()
    }

    #[test]
    fn make_plugin_key_lowercases_every_component() {
        assert_eq!(
            make_plugin_key("Step One", "GROUP", "PlugIn"),
            "step one\x1fgroup\x1fplugin"
        );
        assert_eq!(
            make_plugin_key("s", "g", "p"),
            make_plugin_key("S", "G", "P")
        );
    }

    #[test]
    fn make_plugin_key_separator_is_unit_separator() {
        let key = make_plugin_key("a", "b", "c");
        assert_eq!(key, "a\u{1f}b\u{1f}c");
        // names containing the ASCII separators FOMOD text can carry (spaces, slashes) cannot
        // collide across component boundaries.
        assert_ne!(
            make_plugin_key("a b", "c", "d"),
            make_plugin_key("a", "b c", "d")
        );
    }

    #[test]
    fn enqueue_entry_skips_empty_source() {
        let mut ops = Vec::new();
        let mut order = 7;
        enqueue_entry(
            &entry("", "textures/a.dds"),
            "src",
            "dst",
            &mut ops,
            &mut order,
        );
        assert!(ops.is_empty());
        assert_eq!(order, 7, "doc order must not advance for a skipped entry");
    }

    #[test]
    fn enqueue_entry_skips_traversal_destination() {
        let mut ops = Vec::new();
        let mut order = 0;
        enqueue_entry(
            &entry("a.esp", "C:/windows/x"),
            "src",
            "dst",
            &mut ops,
            &mut order,
        );
        enqueue_entry(
            &entry("a.esp", "../../escape.esp"),
            "src",
            "dst",
            &mut ops,
            &mut order,
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].destination, joined("dst", "../../escape.esp"));
        assert_eq!(order, 1);
    }

    #[test]
    fn enqueue_entry_reproduces_the_rooted_destination_hole() {
        let mut ops = Vec::new();
        let mut order = 0;
        enqueue_entry(
            &entry("a.esp", "/etc/passwd"),
            "src",
            "dst",
            &mut ops,
            &mut order,
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].destination, joined("dst", "/etc/passwd"));
        assert!(!ops[0].destination.starts_with("dst"));
    }

    #[test]
    fn enqueue_entry_joins_paths_and_post_increments_doc_order() {
        let mut ops = Vec::new();
        let mut order = 3;
        let mut folder = entry("data/meshes", "meshes");
        folder.is_folder = true;
        folder.priority = 5;
        enqueue_entry(
            &entry("data/a.esp", "a.esp"),
            "src",
            "dst",
            &mut ops,
            &mut order,
        );
        enqueue_entry(&folder, "src", "dst", &mut ops, &mut order);

        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].source, joined("src", "data/a.esp"));
        assert_eq!(ops[0].destination, joined("dst", "a.esp"));
        assert_eq!(ops[0].op_type, FileOpType::File);
        assert_eq!(ops[0].priority, 0);
        assert_eq!(ops[0].document_order, 3);
        assert_eq!(ops[1].op_type, FileOpType::Folder);
        assert_eq!(ops[1].priority, 5);
        assert_eq!(ops[1].document_order, 4);
        assert_eq!(order, 5);
    }

    fn op(priority: i32, document_order: i32, source: &str) -> FileOperation {
        FileOperation {
            op_type: FileOpType::File,
            source: source.to_string(),
            destination: "d".to_string(),
            priority,
            document_order,
        }
    }

    #[test]
    fn execute_sorts_by_priority_then_document_order() {
        let mut ops = vec![
            op(0, 2, "c"),
            op(5, 0, "high-first-enqueued"),
            op(0, 1, "b"),
            op(0, 0, "a"),
            op(-1, 9, "lowest"),
        ];
        let mut seen = Vec::new();
        let failed = execute_file_operations_with(&mut ops, |o| {
            seen.push(o.source.clone());
            false
        });

        assert_eq!(failed, 0);
        assert_eq!(seen, vec!["lowest", "a", "b", "c", "high-first-enqueued"]);
        assert!(ops.is_empty(), "the queue is cleared after execution");
    }

    #[test]
    fn execute_counts_failures_without_aborting() {
        let mut ops = vec![op(0, 0, "ok"), op(0, 1, "bad"), op(0, 2, "ok2")];
        let mut seen = Vec::new();
        let failed = execute_file_operations_with(&mut ops, |o| {
            seen.push(o.source.clone());
            o.source == "bad"
        });

        assert_eq!(failed, 1);
        assert_eq!(seen, vec!["ok", "bad", "ok2"], "a failure does not abort");
        assert!(ops.is_empty());
    }

    #[test]
    fn execute_through_the_real_back_end_never_reports_a_failure() {
        // FileOperations::copy_file absorbs its own I/O errors (a missing source is a silent skip),
        // so the failure count is 0 even for paths that cannot possibly be copied.
        let mut ops = vec![
            op(0, 0, "no-such-source-file.esp"),
            FileOperation {
                op_type: FileOpType::Folder,
                source: "no-such-source-folder".to_string(),
                destination: "d".to_string(),
                priority: 0,
                document_order: 1,
            },
        ];
        assert_eq!(execute_file_operations(&mut ops), 0);
        assert!(ops.is_empty());
    }

    #[test]
    fn required_files_enqueue_in_document_order() {
        let mut service = FomodService::new();
        service.set_installer(FomodInstaller {
            required_files: vec![
                entry("a.esp", "a.esp"),
                entry("", "skipped"),
                entry("b.esp", "b.esp"),
            ],
            ..FomodInstaller::default()
        });
        let mut ops = Vec::new();
        let mut order = 0;
        service.process_required_files("src", "dst", &mut ops, &mut order);

        assert_eq!(
            sources(&ops),
            vec![joined("src", "a.esp"), joined("src", "b.esp")]
        );
        assert_eq!(ops[0].document_order, 0);
        assert_eq!(ops[1].document_order, 1);
        assert_eq!(order, 2);
    }

    #[test]
    fn conditional_patterns_respect_flags() {
        let mut ir = FomodInstaller::default();
        ir.conditional_patterns
            .push(crate::fomod_ir::FomodConditionalPattern {
                condition: flag_leaf("mode", "on"),
                files: vec![entry("on.esp", "on.esp")],
            });
        ir.conditional_patterns
            .push(crate::fomod_ir::FomodConditionalPattern {
                condition: flag_leaf("mode", "off"),
                files: vec![entry("off.esp", "off.esp")],
            });

        let mut service = FomodService::new();
        service.set_installer(ir.clone());
        let mut ops = Vec::new();
        let mut order = 0;
        service.process_conditional_files("src", "dst", None, &mut ops, &mut order);
        assert!(ops.is_empty());

        let mut ir_with_flag = ir.clone();
        let mut flag_plugin = plugin("P", vec![]);
        flag_plugin.condition_flags = vec![("mode".to_string(), "on".to_string())];
        ir_with_flag.steps.push(step(
            "S",
            vec![group("G", FomodGroupType::SelectAny, vec![flag_plugin])],
        ));

        let mut service = FomodService::new();
        service.set_installer(ir_with_flag);
        let config = selections(vec![json_step(
            "S",
            vec![json_group("G", vec![Value::string("P")])],
        )]);
        let mut ops = Vec::new();
        let mut order = 0;
        service
            .process_optional_files(&config, "src", "dst", None, &mut ops, &mut order)
            .expect("well-formed selections");
        service.process_conditional_files("src", "dst", None, &mut ops, &mut order);

        assert_eq!(sources(&ops), vec![joined("src", "on.esp")]);
    }

    #[test]
    fn module_dependencies_absent_is_satisfied() {
        let service = FomodService::new();
        assert!(service.check_module_dependencies(None));
    }

    #[test]
    fn module_dependencies_evaluate_against_flags() {
        let mut service = FomodService::new();
        service.set_installer(FomodInstaller {
            module_dependencies: Some(flag_leaf("ready", "1")),
            ..FomodInstaller::default()
        });
        assert!(!service.check_module_dependencies(None));

        service
            .plugin_flags
            .insert("ready".to_string(), "1".to_string());
        assert!(service.check_module_dependencies(None));
    }

    #[test]
    fn read_plugin_name_accepts_both_schemas_and_skips_the_rest() {
        assert_eq!(read_plugin_name(&Value::string("A")), "A");
        assert_eq!(read_plugin_name(&json_plugin_v2("B")), "B");
        assert_eq!(read_plugin_name(&Value::Int(3)), "");
        assert_eq!(read_plugin_name(&Value::Null), "");
        assert_eq!(read_plugin_name(&Value::array()), "");
        let mut no_name = Value::object();
        no_name.insert("selected", Value::Bool(true));
        assert_eq!(read_plugin_name(&no_name), "");
        let mut bad_name = Value::object();
        bad_name.insert("name", Value::Int(7));
        assert_eq!(read_plugin_name(&bad_name), "");
    }

    #[test]
    fn name_field_reproduces_nlohmann_value_tri_state() {
        let mut with_name = Value::object();
        with_name.insert("name", Value::string("N"));
        assert_eq!(name_field(&with_name), Some("N"));
        assert_eq!(name_field(&Value::object()), Some(""));
        assert_eq!(name_field(&Value::string("N")), None);
        let mut bad = Value::object();
        bad.insert("name", Value::Int(1));
        assert_eq!(name_field(&bad), None);
    }

    fn two_plugin_ir() -> FomodInstaller {
        installer(vec![step(
            "Step",
            vec![group(
                "Group",
                FomodGroupType::SelectAny,
                vec![
                    plugin("Alpha", vec![entry("alpha.esp", "alpha.esp")]),
                    plugin("Beta", vec![entry("beta.esp", "beta.esp")]),
                ],
            )],
        )])
    }

    #[test]
    fn schema_v1_and_v2_produce_identical_operations() {
        let v1 = selections(vec![json_step(
            "Step",
            vec![json_group("Group", vec![Value::string("Beta")])],
        )]);
        let v2 = selections(vec![json_step(
            "Step",
            vec![json_group("Group", vec![json_plugin_v2("Beta")])],
        )]);

        let (_, ops_v1) = run_optional(two_plugin_ir(), &v1).expect("v1");
        let (_, ops_v2) = run_optional(two_plugin_ir(), &v2).expect("v2");

        assert_eq!(sources(&ops_v1), vec![joined("src", "beta.esp")]);
        assert_eq!(ops_v1, ops_v2);
    }

    #[test]
    fn unreadable_plugin_entry_is_skipped_without_consuming_an_occurrence() {
        // two IR plugins share the name "Dup". a junk entry between the two JSON selections must
        // not advance the occurrence counter, so the second readable "Dup" still binds to the
        // second IR plugin.
        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![
                    plugin("Dup", vec![entry("first.esp", "first.esp")]),
                    plugin("Dup", vec![entry("second.esp", "second.esp")]),
                ],
            )],
        )]);
        let config = selections(vec![json_step(
            "S",
            vec![json_group(
                "G",
                vec![Value::string("Dup"), Value::Int(42), Value::string("Dup")],
            )],
        )]);

        let (_, ops) = run_optional(ir, &config).expect("tolerant of junk entries");
        assert_eq!(
            sources(&ops),
            vec![joined("src", "first.esp"), joined("src", "second.esp")]
        );
    }

    #[test]
    fn duplicate_step_and_group_names_bind_by_occurrence() {
        let ir = installer(vec![
            step(
                "S",
                vec![group(
                    "G",
                    FomodGroupType::SelectAny,
                    vec![plugin("P", vec![entry("s1.esp", "s1.esp")])],
                )],
            ),
            step(
                "S",
                vec![group(
                    "G",
                    FomodGroupType::SelectAny,
                    vec![plugin("P", vec![entry("s2.esp", "s2.esp")])],
                )],
            ),
        ]);
        let config = selections(vec![
            json_step("S", vec![json_group("G", vec![Value::string("P")])]),
            json_step("S", vec![json_group("G", vec![Value::string("P")])]),
        ]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(
            sources(&ops),
            vec![joined("src", "s1.esp"), joined("src", "s2.esp")]
        );
    }

    #[test]
    fn step_without_groups_array_still_consumes_an_occurrence() {
        let ir = installer(vec![
            step(
                "S",
                vec![group(
                    "G",
                    FomodGroupType::SelectAny,
                    vec![plugin("P", vec![entry("first.esp", "first.esp")])],
                )],
            ),
            step(
                "S",
                vec![group(
                    "G",
                    FomodGroupType::SelectAny,
                    vec![plugin("P", vec![entry("second.esp", "second.esp")])],
                )],
            ),
        ]);

        let mut groupless = Value::object();
        groupless.insert("name", Value::string("S"));
        let config = selections(vec![
            groupless,
            json_step("S", vec![json_group("G", vec![Value::string("P")])]),
        ]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(sources(&ops), vec![joined("src", "second.esp")]);
    }

    #[test]
    fn missing_ir_step_does_not_consume_a_second_occurrence() {
        // only one IR step named "S". three JSON steps named "S": the first binds (occ 0), the
        // second and third miss (occ 1 and 2) without double incrementing, which would matter if
        // more IR steps existed.
        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![plugin("P", vec![entry("only.esp", "only.esp")])],
            )],
        )]);
        let config = selections(vec![
            json_step("S", vec![json_group("G", vec![Value::string("P")])]),
            json_step("S", vec![json_group("G", vec![Value::string("P")])]),
            json_step("S", vec![json_group("G", vec![Value::string("P")])]),
        ]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(sources(&ops), vec![joined("src", "only.esp")]);
    }

    #[test]
    fn group_without_plugins_array_still_consumes_an_occurrence() {
        let ir = installer(vec![step(
            "S",
            vec![
                group(
                    "G",
                    FomodGroupType::SelectAny,
                    vec![plugin("P", vec![entry("g1.esp", "g1.esp")])],
                ),
                group(
                    "G",
                    FomodGroupType::SelectAny,
                    vec![plugin("P", vec![entry("g2.esp", "g2.esp")])],
                ),
            ],
        )]);

        let mut plugin_less = Value::object();
        plugin_less.insert("name", Value::string("G"));
        let config = selections(vec![json_step(
            "S",
            vec![plugin_less, json_group("G", vec![Value::string("P")])],
        )]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(sources(&ops), vec![joined("src", "g2.esp")]);
    }

    #[test]
    fn duplicate_plugin_names_bind_by_occurrence() {
        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![
                    plugin("P", vec![entry("p1.esp", "p1.esp")]),
                    plugin("P", vec![entry("p2.esp", "p2.esp")]),
                    plugin("P", vec![entry("p3.esp", "p3.esp")]),
                ],
            )],
        )]);
        let config = selections(vec![json_step(
            "S",
            vec![json_group(
                "G",
                vec![Value::string("P"), Value::string("P")],
            )],
        )]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(
            sources(&ops),
            vec![joined("src", "p1.esp"), joined("src", "p2.esp")]
        );
    }

    #[test]
    fn invisible_step_is_skipped_in_every_pass() {
        let mut hidden = step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![
                    plugin("Sel", vec![entry("sel.esp", "sel.esp")]),
                    FomodPlugin {
                        r#type: PluginType::Required,
                        ..plugin("Req", vec![entry("req.esp", "req.esp")])
                    },
                    FomodPlugin {
                        files: vec![FomodFileEntry {
                            always_install: true,
                            ..entry("always.esp", "always.esp")
                        }],
                        ..plugin("Always", vec![])
                    },
                ],
            )],
        );
        hidden.visible = Some(flag_leaf("show", "1"));

        let config = selections(vec![json_step(
            "S",
            vec![json_group("G", vec![Value::string("Sel")])],
        )]);
        let (_, ops) = run_optional(installer(vec![hidden]), &config).expect("well-formed");
        assert!(ops.is_empty(), "nothing from a hidden step is installed");
    }

    #[test]
    fn plugin_dependencies_gate_processing() {
        let mut gated = plugin("Gated", vec![entry("gated.esp", "gated.esp")]);
        gated.dependencies = Some(flag_leaf("allow", "1"));
        let ir = installer(vec![step(
            "S",
            vec![group("G", FomodGroupType::SelectAny, vec![gated])],
        )]);
        let config = selections(vec![json_step(
            "S",
            vec![json_group("G", vec![Value::string("Gated")])],
        )]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert!(ops.is_empty());
    }

    #[test]
    fn condition_flags_accumulate_and_skip_empty_names() {
        let mut setter = plugin("Setter", vec![]);
        setter.condition_flags = vec![
            ("".to_string(), "ignored".to_string()),
            ("mode".to_string(), "on".to_string()),
        ];
        let mut dependent = plugin("Dependent", vec![entry("dep.esp", "dep.esp")]);
        dependent.dependencies = Some(flag_leaf("mode", "on"));

        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![setter, dependent],
            )],
        )]);
        let config = selections(vec![json_step(
            "S",
            vec![json_group(
                "G",
                vec![Value::string("Setter"), Value::string("Dependent")],
            )],
        )]);

        let (service, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(sources(&ops), vec![joined("src", "dep.esp")]);
        assert_eq!(
            service.plugin_flags().get("mode").map(String::as_str),
            Some("on")
        );
        assert!(!service.plugin_flags().contains_key(""));
    }

    #[test]
    fn required_plugins_auto_install_in_a_json_covered_step() {
        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![
                    plugin("Sel", vec![entry("sel.esp", "sel.esp")]),
                    FomodPlugin {
                        r#type: PluginType::Required,
                        ..plugin("Req", vec![entry("req.esp", "req.esp")])
                    },
                    plugin("Other", vec![entry("other.esp", "other.esp")]),
                ],
            )],
        )]);
        let config = selections(vec![json_step(
            "S",
            vec![json_group("G", vec![Value::string("Sel")])],
        )]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(
            sources(&ops),
            vec![joined("src", "sel.esp"), joined("src", "req.esp")],
            "the explicit selection is enqueued first, then the Required pass"
        );
    }

    #[test]
    fn required_plugins_auto_install_for_steps_absent_from_the_json() {
        let ir = installer(vec![
            step(
                "Covered",
                vec![group(
                    "G",
                    FomodGroupType::SelectAny,
                    vec![plugin("Sel", vec![entry("sel.esp", "sel.esp")])],
                )],
            ),
            step(
                "Uncovered",
                vec![group(
                    "G2",
                    FomodGroupType::SelectAny,
                    vec![
                        FomodPlugin {
                            r#type: PluginType::Required,
                            ..plugin("Req", vec![entry("req.esp", "req.esp")])
                        },
                        plugin("Skipped", vec![entry("skipped.esp", "skipped.esp")]),
                    ],
                )],
            ),
        ]);
        let config = selections(vec![json_step(
            "Covered",
            vec![json_group("G", vec![Value::string("Sel")])],
        )]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(
            sources(&ops),
            vec![joined("src", "sel.esp"), joined("src", "req.esp")]
        );
    }

    #[test]
    fn required_type_from_a_type_pattern_also_auto_installs() {
        let mut patterned = plugin("Patterned", vec![entry("pat.esp", "pat.esp")]);
        patterned.type_patterns = vec![FomodTypePattern {
            condition: flag_leaf("mode", "on"),
            result_type: PluginType::Required,
        }];
        let mut setter = plugin("Setter", vec![]);
        setter.condition_flags = vec![("mode".to_string(), "on".to_string())];

        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![setter, patterned],
            )],
        )]);
        let config = selections(vec![json_step(
            "S",
            vec![json_group("G", vec![Value::string("Setter")])],
        )]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(sources(&ops), vec![joined("src", "pat.esp")]);
    }

    #[test]
    fn always_install_and_install_if_usable_from_unselected_plugins() {
        let unselected = FomodPlugin {
            files: vec![
                FomodFileEntry {
                    always_install: true,
                    ..entry("always.esp", "always.esp")
                },
                FomodFileEntry {
                    install_if_usable: true,
                    ..entry("usable.esp", "usable.esp")
                },
                entry("plain.esp", "plain.esp"),
            ],
            ..plugin("Unselected", vec![])
        };
        let not_usable = FomodPlugin {
            r#type: PluginType::NotUsable,
            files: vec![
                FomodFileEntry {
                    always_install: true,
                    ..entry("nu-always.esp", "nu-always.esp")
                },
                FomodFileEntry {
                    install_if_usable: true,
                    ..entry("nu-usable.esp", "nu-usable.esp")
                },
            ],
            ..plugin("NotUsable", vec![])
        };
        let selected = FomodPlugin {
            files: vec![FomodFileEntry {
                always_install: true,
                ..entry("sel-always.esp", "sel-always.esp")
            }],
            ..plugin("Selected", vec![])
        };

        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![selected, unselected, not_usable],
            )],
        )]);
        let config = selections(vec![json_step(
            "S",
            vec![json_group("G", vec![Value::string("Selected")])],
        )]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(
            sources(&ops),
            vec![
                joined("src", "sel-always.esp"),
                joined("src", "always.esp"),
                joined("src", "usable.esp"),
                joined("src", "nu-always.esp"),
            ]
        );
    }

    #[test]
    fn required_plugins_are_excluded_from_the_auto_install_pass() {
        // a Required plugin is installed whole by pass 1b or 2, so pass 3 must not re-enqueue its
        // alwaysInstall entry.
        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![FomodPlugin {
                    r#type: PluginType::Required,
                    files: vec![FomodFileEntry {
                        always_install: true,
                        ..entry("req.esp", "req.esp")
                    }],
                    ..plugin("Req", vec![])
                }],
            )],
        )]);
        let config = selections(vec![json_step("S", vec![])]);

        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(sources(&ops), vec![joined("src", "req.esp")]);
    }

    #[test]
    fn no_steps_array_still_returns_ok_and_enqueues_nothing() {
        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![FomodPlugin {
                    r#type: PluginType::Required,
                    ..plugin("Req", vec![entry("req.esp", "req.esp")])
                }],
            )],
        )]);
        let (_, ops) = run_optional(ir.clone(), &Value::object()).expect("no steps");
        assert!(ops.is_empty());

        let mut not_an_array = Value::object();
        not_an_array.insert("steps", Value::Int(1));
        let (_, ops) = run_optional(ir, &not_an_array).expect("steps not an array");
        assert!(ops.is_empty());
    }

    #[test]
    fn non_string_step_name_aborts_and_rolls_back() {
        let ir = installer(vec![
            step(
                "First",
                vec![group(
                    "G",
                    FomodGroupType::SelectAny,
                    vec![plugin("P", vec![entry("p.esp", "p.esp")])],
                )],
            ),
            step(
                "Second",
                vec![group(
                    "G",
                    FomodGroupType::SelectAny,
                    vec![plugin("Q", vec![entry("q.esp", "q.esp")])],
                )],
            ),
        ]);

        let mut bad_step = Value::object();
        bad_step.insert("name", Value::Int(9));
        bad_step.insert("groups", Value::array());
        let config = selections(vec![
            json_step("First", vec![json_group("G", vec![Value::string("P")])]),
            bad_step,
        ]);

        let mut service = FomodService::new();
        service.set_installer(ir);

        // pre-existing operations (e.g. from process_required_files) must survive the rollback
        // untouched.
        let mut ops = vec![op(0, 0, "pre-existing")];
        let mut order = 1;
        let err = service
            .process_optional_files(&config, "src", "dst", None, &mut ops, &mut order)
            .expect_err("a non-string step name aborts");

        assert_eq!(err, SelectionsError::NameTypeError);
        assert_eq!(sources(&ops), vec!["pre-existing".to_string()]);
        assert_eq!(order, 1, "next_doc_order is restored");
    }

    #[test]
    fn non_object_step_and_non_string_group_name_abort() {
        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectAny,
                vec![plugin("P", vec![entry("p.esp", "p.esp")])],
            )],
        )]);

        let config = selections(vec![Value::string("S")]);
        let mut service = FomodService::new();
        service.set_installer(ir.clone());
        let mut ops = Vec::new();
        let mut order = 0;
        assert_eq!(
            service.process_optional_files(&config, "src", "dst", None, &mut ops, &mut order),
            Err(SelectionsError::NameTypeError)
        );

        let mut bad_group = Value::object();
        bad_group.insert("name", Value::Bool(true));
        bad_group.insert("plugins", Value::array());
        let config = selections(vec![json_step("S", vec![bad_group])]);
        let mut service = FomodService::new();
        service.set_installer(ir);
        let mut ops = Vec::new();
        let mut order = 0;
        assert_eq!(
            service.process_optional_files(&config, "src", "dst", None, &mut ops, &mut order),
            Err(SelectionsError::NameTypeError)
        );
    }

    fn validate_with(group_type: FomodGroupType, total: usize, selected: &[&str]) -> bool {
        let plugins = (0..total)
            .map(|i| plugin(&format!("P{i}"), vec![]))
            .collect();
        let ir = installer(vec![step("S", vec![group("G", group_type, plugins)])]);
        let config = selections(vec![json_step(
            "S",
            vec![json_group(
                "G",
                selected.iter().map(|n| Value::string(*n)).collect(),
            )],
        )]);

        let mut service = FomodService::new();
        service.set_installer(ir);
        service
            .validate_json_selections(&config)
            .expect("well-formed")
    }

    #[test]
    fn validate_cardinality_per_group_type() {
        assert!(validate_with(FomodGroupType::SelectExactlyOne, 3, &["P0"]));
        assert!(!validate_with(
            FomodGroupType::SelectExactlyOne,
            3,
            &["P0", "P1"]
        ));

        assert!(validate_with(
            FomodGroupType::SelectAtLeastOne,
            3,
            &["P0", "P1"]
        ));
        // an empty group entry never reaches selected_plugins, so the group is not validated
        // - the failure needs a selection the IR does not contain.
        assert!(!validate_with(
            FomodGroupType::SelectAtLeastOne,
            3,
            &["Ghost"]
        ));

        assert!(validate_with(FomodGroupType::SelectAtMostOne, 3, &["P0"]));
        assert!(!validate_with(
            FomodGroupType::SelectAtMostOne,
            3,
            &["P0", "P1"]
        ));

        assert!(validate_with(FomodGroupType::SelectAll, 2, &["P0", "P1"]));
        assert!(!validate_with(FomodGroupType::SelectAll, 2, &["P0"]));

        assert!(validate_with(FomodGroupType::SelectAny, 3, &[]));
        assert!(validate_with(
            FomodGroupType::SelectAny,
            3,
            &["P0", "P1", "P2"]
        ));
    }

    #[test]
    fn validate_checks_every_group_before_returning_false() {
        let ir = installer(vec![step(
            "S",
            vec![
                group(
                    "Bad",
                    FomodGroupType::SelectExactlyOne,
                    vec![plugin("A", vec![]), plugin("B", vec![])],
                ),
                group(
                    "Good",
                    FomodGroupType::SelectExactlyOne,
                    vec![plugin("C", vec![])],
                ),
            ],
        )]);
        let config = selections(vec![json_step(
            "S",
            vec![
                json_group("Bad", vec![Value::string("A"), Value::string("B")]),
                json_group("Good", vec![Value::string("C")]),
            ],
        )]);

        let mut service = FomodService::new();
        service.set_installer(ir);
        assert_eq!(service.validate_json_selections(&config), Ok(false));
    }

    #[test]
    fn validate_missing_steps_is_true() {
        let mut service = FomodService::new();
        service.set_installer(installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectExactlyOne,
                vec![plugin("A", vec![]), plugin("B", vec![])],
            )],
        )]));
        assert_eq!(service.validate_json_selections(&Value::object()), Ok(true));

        let mut not_an_array = Value::object();
        not_an_array.insert("steps", Value::string("nope"));
        assert_eq!(service.validate_json_selections(&not_an_array), Ok(true));
    }

    #[test]
    fn validate_ignores_groups_the_json_does_not_mention() {
        let ir = installer(vec![step(
            "S",
            vec![group(
                "Untouched",
                FomodGroupType::SelectExactlyOne,
                vec![plugin("A", vec![])],
            )],
        )]);
        let config = selections(vec![json_step("S", vec![])]);
        let mut service = FomodService::new();
        service.set_installer(ir);
        assert_eq!(service.validate_json_selections(&config), Ok(true));
    }

    #[test]
    fn validate_propagates_the_name_type_error() {
        let ir = installer(vec![step("S", vec![])]);
        let mut service = FomodService::new();
        service.set_installer(ir);

        let config = selections(vec![Value::Int(1)]);
        assert_eq!(
            service.validate_json_selections(&config),
            Err(SelectionsError::NameTypeError)
        );

        let mut bad_group = Value::object();
        bad_group.insert("name", Value::Null);
        bad_group.insert("plugins", Value::array());
        let config = selections(vec![json_step("S", vec![bad_group])]);
        assert_eq!(
            service.validate_json_selections(&config),
            Err(SelectionsError::NameTypeError)
        );
    }

    #[test]
    fn validate_tolerates_unreadable_plugin_entries() {
        let ir = installer(vec![step(
            "S",
            vec![group(
                "G",
                FomodGroupType::SelectExactlyOne,
                vec![plugin("A", vec![]), plugin("B", vec![])],
            )],
        )]);
        let config = selections(vec![json_step(
            "S",
            vec![json_group(
                "G",
                vec![Value::string("A"), Value::Int(3), Value::Null],
            )],
        )]);
        let mut service = FomodService::new();
        service.set_installer(ir);
        assert_eq!(service.validate_json_selections(&config), Ok(true));
    }
}
