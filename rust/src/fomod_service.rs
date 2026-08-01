//! FOMOD install replay - Rust port of `src/FomodService.hpp`/`.cpp`.
//!
//! Interprets a parsed [`FomodInstaller`] IR together with a JSON selections
//! document and produces the ordered [`FileOperation`] queue that realizes the
//! install. The pipeline the C++ `InstallationService` drives is unchanged:
//!
//! 1. [`FomodService::check_module_dependencies`] - gate on `<moduleDependencies>`.
//! 2. [`FomodService::validate_json_selections`] - group cardinality check.
//! 3. [`FomodService::process_required_files`] - `<requiredInstallFiles>`.
//! 4. [`FomodService::process_optional_files`] - selected plugins, flags,
//!    Required auto-install, and the alwaysInstall/installIfUsable pass.
//! 5. [`FomodService::process_conditional_files`] - `<conditionalFileInstalls>`.
//! 6. [`execute_file_operations`] - stable sort by `(priority, document_order)`
//!    and copy.
//!
//! ## Divergences from the C++ (all deliberate)
//!
//! - **Exceptions become `Result`.** The C++ `process_optional_files` and
//!   `validate_json_selections` read step/group names with
//!   `json.value("name", "")`, which THROWS `nlohmann::json::type_error` 306 on a
//!   non-object element and 302 on a present-but-non-string `name`. Those throws
//!   escape to the caller (`InstallationService::handle_fomod_install`), failing
//!   the whole install; `process_optional_files` additionally rolls its queued
//!   operations back first. Here both return `Result<_, SelectionsError>` and the
//!   private `name_field` reproduces `value()`'s exact tri-state, matching what
//!   `fomod_inference_service::name_field` does for the Tier-1 cache (see
//!   PARITY-NOTES "Task 12"). PLUGIN names stay tolerant: they go through
//!   `read_plugin_name`, mirroring the C++ anon-namespace helper.
//! - **The class doc comment in `FomodService.hpp` is STALE.** It claims plugin
//!   entries are read via `get<std::string>()` and that a non-string entry throws
//!   `nlohmann::json::type_error`. The actual code (`FomodService.cpp:22-33`) is
//!   the schema-tolerant `read_plugin_name`, which accepts schema-v1 strings AND
//!   schema-v2 objects and returns `""` for anything else so the caller skips it.
//!   The CODE is reproduced, not the doc.
//! - **`enqueue_entry` / `enqueue_plugin_files` / `make_plugin_key` are free
//!   functions.** The C++ has them as private members, but only `make_plugin_key`
//!   is `static` and none of the three touch instance state. Free functions let
//!   `process_optional_files` split-borrow `installer` and `plugin_flags`.
//! - **Logging is reproduced.** Every `Logger` call site of the C++ is present
//!   at the same level, with the same `[fomod]` wording and the same place in
//!   control flow, including the file/folder and processed/skipped tallies the
//!   C++ computes purely so it can log them. The two text divergences are both
//!   places where the C++ interpolates a caught exception's `what()`, which has
//!   no counterpart here: the rollback line in
//!   [`FomodService::process_optional_files`] interpolates [`SelectionsError`]'s
//!   `Display` instead (the error the port raises in place of that throw), and
//!   the per-operation failure line in [`execute_file_operations`] drops the
//!   trailing `: reason` entirely (its back end reports failure as a bool). The
//!   latter is unreachable in both languages, see [`execute_file_operations`].
//! - **The copy back end.** [`execute_file_operations`] calls
//!   [`FileOperations::copy_file`] / [`FileOperations::copy_folder`], the port of
//!   the C++ `FileOperations` statics. Those statics are documented and
//!   implemented as non-throwing (every error is caught and logged inside), so
//!   the C++ `failed` counter is 0 for every reachable input; the counting loop
//!   is ported anyway because the count is observable through the return value.

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

/// The one failure mode the C++ signals by throwing out of a JSON traversal.
///
/// `nlohmann::basic_json::value(key, default)` is NOT total: it throws
/// `type_error 306` when the receiver is not an object and `type_error 302` when
/// the key is present but not a string (only an ABSENT key uses the default).
/// Both throws propagate out of the C++ `FomodService` methods, so they are
/// modelled as an error rather than coerced to an empty name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionsError {
    /// A `steps[]` or `groups[]` element was not an object, or carried a
    /// present-but-non-string `name`.
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

/// Core FOMOD processing, install replay, and plugin flag evaluation. Mirror of
/// `mo2core::FomodService`.
///
/// Holds the parsed IR plus the condition flags accumulated while walking the
/// selections. Not thread-safe by design (the C++ says the same); one instance
/// per installation.
#[derive(Debug, Clone, Default)]
pub struct FomodService {
    /// Parsed FOMOD IR (set by [`FomodService::set_installer`]).
    installer: FomodInstaller,
    /// Accumulated condition flags from processed plugins.
    plugin_flags: HashMap<String, String>,
}

impl FomodService {
    /// Construct a service with an empty IR and no flags.
    pub fn new() -> Self {
        FomodService::default()
    }

    /// Set the parsed FOMOD IR for this installation. Mirror of
    /// `FomodService::set_installer`.
    ///
    /// Must be called before any other method; the caller parses the XML via
    /// [`crate::fomod_ir_parser::parse_module_config`] and passes the result.
    pub fn set_installer(&mut self, installer: FomodInstaller) {
        self.installer = installer;
    }

    /// Borrow the IR this service is replaying.
    pub fn installer(&self) -> &FomodInstaller {
        &self.installer
    }

    /// Borrow the condition flags accumulated so far. Mirror of the C++ private
    /// `plugin_flags_`, exposed read-only so callers and tests can observe flag
    /// propagation.
    pub fn plugin_flags(&self) -> &HashMap<String, String> {
        &self.plugin_flags
    }

    /// Evaluate top-level `<moduleDependencies>`. Mirror of
    /// `FomodService::check_module_dependencies`.
    ///
    /// Returns `true` when dependencies are met or absent. Never fails: the
    /// dependency evaluator is total.
    pub fn check_module_dependencies(&self, context: Option<&FomodDependencyContext>) -> bool {
        // (FS 109-113) Absent dependencies -> true.
        let Some(condition) = &self.installer.module_dependencies else {
            Logger::instance().log("[fomod] No module-level dependencies found");
            return true;
        };

        // (FS 115-126) The unmet line is a plain `log` in the C++ (INFO level
        // with an "ERROR:" prefix in the text), not `log_error`.
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

    /// Enqueue files from `<requiredInstallFiles>`. Mirror of
    /// `FomodService::process_required_files`.
    ///
    /// These are installed regardless of user selections. Unsafe destinations are
    /// skipped (see the private `enqueue_entry`); `next_doc_order` is advanced
    /// once per enqueued operation.
    pub fn process_required_files(
        &self,
        src_base: &str,
        dst_base: &str,
        ops: &mut Vec<FileOperation>,
        next_doc_order: &mut i32,
    ) {
        // (FS 139-143) Empty list -> nothing to do.
        if self.installer.required_files.is_empty() {
            Logger::instance().log("[fomod] No required install files found");
            return;
        }

        Logger::instance().log(&format!(
            "[fomod] Processing {} required install files...",
            self.installer.required_files.len()
        ));

        // (FS 148-160) The tallies exist only to be logged below; an entry counts
        // only when it actually produced an operation.
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

    /// Validate JSON selections against the IR's group cardinality constraints.
    /// Mirror of `FomodService::validate_json_selections`.
    ///
    /// Returns `Ok(false)` when ANY group violates its constraint (validation
    /// never aborts early; every group is still checked), `Ok(true)` otherwise -
    /// including when the document has no `steps` array. Returns
    /// `Err(SelectionsError::NameTypeError)` where the C++ `value("name", "")`
    /// would throw, which in the C++ aborts the whole install.
    pub fn validate_json_selections(&self, config_json: &Value) -> Result<bool, SelectionsError> {
        // (FS 193-197) No steps array -> validation skipped.
        let Some(steps) = config_json.get("steps").filter(|s| s.is_array()) else {
            Logger::instance().log("[fomod] No steps in JSON - validation skipped");
            return Ok(true);
        };

        Logger::instance().log("[fomod] Validating JSON selections against FOMOD schema...");

        // Build step -> group -> set(plugin name) from the JSON (FS 203-227).
        // Nested entries are only created once a readable plugin name lands in
        // them, exactly like the C++ `selected_plugins[step][group].insert(...)`.
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

        // Validate against the IR structure (FS 230-300). Note the lookups are by
        // NAME only - unlike process_optional_files there is no occurrence
        // matching here, so two IR steps sharing a name both see the same set.
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

                // (FS 252-271) Warn about selections not present in the IR group.
                // This pass is log-only; it has no effect on the return value.
                // The C++ iterates an `unordered_set`, so the relative order of
                // these warnings is unspecified there too.
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

    /// Process selected plugins: accumulate flags and enqueue file operations.
    /// Mirror of `FomodService::process_optional_files`.
    ///
    /// Walks the JSON `step -> group -> plugin` selections, binding each JSON
    /// name to the IR node with the same name by OCCURRENCE (the Nth JSON
    /// occurrence of a name binds to the Nth IR node with that name), then:
    ///
    /// 1. per JSON step, auto-installs Required-type plugins of the bound IR step
    ///    that were not already processed;
    /// 2. auto-installs Required plugins for IR steps whose name appears in NO
    ///    JSON step;
    /// 3. enqueues `alwaysInstall` / `installIfUsable` entries from every still
    ///    unprocessed plugin (skipping Required, already handled above, and
    ///    skipping `installIfUsable` when the effective type is `NotUsable`).
    ///
    /// Every pass re-checks step visibility against the flags known at that point.
    ///
    /// On `Err`, the C++ rollback contract is honoured: `ops` is truncated back to
    /// its length on entry and `next_doc_order` is restored, so a retry with the
    /// same vector after fixing the JSON is idempotent.
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

        // (FS 321-327) No steps array -> nothing to do. The C++ try/catch only
        // wraps the has_steps branch, so no rollback applies here; the total line
        // (FS 633) is outside the if/else and is emitted on this path too.
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
            // (FS 621-630) Roll the whole call back, log, then propagate. The C++
            // interpolates `ex.what()` from the nlohmann type_error; the port has
            // no such object and interpolates its own error text instead.
            ops.truncate(initial_ops_size);
            *next_doc_order = initial_doc_order;
            Logger::instance().log_error(&format!(
                "[fomod] Exception during optional file processing, rolled back queued operations: {err}"
            ));
            // The C++ rethrows from the catch, so FS 633 is NOT reached.
            return Err(err);
        }

        Logger::instance().log(&format!(
            "[fomod] Total file operations queued from optional files: {}",
            ops.len()
        ));
        result
    }

    /// The body of the C++ `try` block in `process_optional_files`. Split out so
    /// the caller can apply the rollback on any `Err` return, the way the C++
    /// `catch` clause does before re-throwing.
    fn process_optional_files_inner(
        &mut self,
        steps: &Value,
        src_base: &str,
        dst_base: &str,
        context: Option<&FomodDependencyContext>,
        ops: &mut Vec<FileOperation>,
        next_doc_order: &mut i32,
    ) -> Result<(), SelectionsError> {
        // Split-borrow: the IR is read while the flag map is written.
        let FomodService {
            installer,
            plugin_flags,
        } = self;

        let mut processed_plugins: HashSet<String> = HashSet::new();

        // (FS 333-334) First statement of the C++ try block.
        Logger::instance().log(&format!(
            "[fomod] Processing optional files from JSON with {} step(s)",
            array_items(Some(steps)).len()
        ));

        // (FS 337-341) Map IR steps by name for occurrence-based matching. The
        // C++ counters are `int`; `usize` here (they only ever increment, and the
        // step count is bounded by the JSON document size).
        let mut ir_steps_by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for (si, step) in installer.steps.iter().enumerate() {
            ir_steps_by_name
                .entry(step.name.as_str())
                .or_default()
                .push(si);
        }
        let mut step_occurrence: HashMap<String, usize> = HashMap::new();

        // --- Pass 1: explicit JSON selections (FS 343-525) ---
        for json_step in array_items(Some(steps)) {
            let Some(step_name) = name_field(json_step) else {
                return Err(SelectionsError::NameTypeError);
            };
            let step_name = step_name.to_string();
            Logger::instance().log(&format!("[fomod] Processing step: \"{step_name}\""));

            // (FS 348-353) A step with no groups array still CONSUMES an
            // occurrence of its name before skipping.
            if !json_step.get("groups").is_some_and(Value::is_array) {
                Logger::instance()
                    .log(&format!("[fomod] Step \"{step_name}\" has no groups array"));
                post_increment(&mut step_occurrence, &step_name);
                continue;
            }

            // (FS 356-366) Post-increment, then bind to the occ-th IR step of
            // that name.
            let occ = post_increment(&mut step_occurrence, &step_name);
            let ir_step_idx = ir_steps_by_name
                .get(step_name.as_str())
                .and_then(|indices| indices.get(occ))
                .copied();

            // (FS 368-373) A missing IR step does NOT consume a second
            // occurrence.
            let Some(ir_step_idx) = ir_step_idx else {
                Logger::instance().log_warning(&format!(
                    "[fomod] Could not find IR step \"{step_name}\" occurrence {occ}"
                ));
                continue;
            };
            let ir_step = &installer.steps[ir_step_idx];

            // (FS 376-383) Step visibility gates everything below, including the
            // per-step Required auto-install pass.
            if step_hidden(&ir_step.visible, plugin_flags, context) {
                Logger::instance().log(&format!(
                    "[fomod] Skipping step \"{step_name}\" due to unmet visibility dependencies"
                ));
                continue;
            }

            // (FS 386-390) Map IR groups by name for occurrence-based matching.
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

                // (FS 399-405) Same shape as the step branch: no plugins array
                // still consumes an occurrence.
                if !json_group.get("plugins").is_some_and(Value::is_array) {
                    Logger::instance().log(&format!(
                        "[fomod] Group \"{group_name}\" has no plugins array"
                    ));
                    post_increment(&mut group_occurrence, &group_name);
                    continue;
                }

                let gocc = post_increment(&mut group_occurrence, &group_name);
                // (FS 408-417) Unlike a missing IR step, a missing IR group does
                // NOT skip the group: the plugin loop still runs (and still
                // advances the plugin occurrence counters), every lookup simply
                // misses.
                let ir_group = ir_groups_by_name
                    .get(group_name.as_str())
                    .and_then(|indices| indices.get(gocc))
                    .map(|&gi| &ir_step.groups[gi]);

                // (FS 420-425) Plugin name index for the bound group.
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
                    // (FS 434-440) Tolerant of both schemas; an unreadable entry
                    // is skipped BEFORE the occurrence counter moves.
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

                    // (FS 490-495) A miss is log-only; the loop continues.
                    let Some(ir_plugin) = ir_plugin else {
                        Logger::instance().log_error(&format!(
                            "[fomod] Could not find plugin \"{plugin_name}\" in step/group IR"
                        ));
                        continue;
                    };

                    // (FS 464-472) Plugin-level dependencies gate processing.
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

            // (FS 500-524) Auto-install Required plugins for this step. The key
            // uses the JSON step name (not the bound IR step's name, which is the
            // same string by construction) plus the IR group/plugin names.
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

        // --- Pass 2: Required plugins for steps not covered by the JSON
        // (FS 528-566). `covered_steps` is a NAME set, so one JSON step covers
        // every IR step sharing its name. ---
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

        // --- Pass 3: alwaysInstall / installIfUsable from unselected plugins
        // (FS 569-612). Visibility is re-checked here too, against the flags as
        // they stand AFTER passes 1 and 2. ---
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
                    // (FS 585-587) Required was already handled by passes 1/2.
                    if eff_type == PluginType::Required {
                        continue;
                    }

                    for entry in &plugin.files {
                        let should_install = entry.always_install
                            || (entry.install_if_usable && eff_type != PluginType::NotUsable);
                        if !should_install {
                            continue;
                        }
                        // (FS 597-608) An entry counts, and logs, only when it
                        // actually produced an operation.
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

    /// Evaluate `<conditionalFileInstalls>` patterns against the current flag
    /// state and enqueue the files of every matching pattern. Mirror of
    /// `FomodService::process_conditional_files`.
    pub fn process_conditional_files(
        &self,
        src_base: &str,
        dst_base: &str,
        context: Option<&FomodDependencyContext>,
        ops: &mut Vec<FileOperation>,
        next_doc_order: &mut i32,
    ) {
        // (FS 648-652) Empty list -> nothing to do.
        if self.installer.conditional_patterns.is_empty() {
            Logger::instance().log("[fomod] No conditional file install patterns found");
            return;
        }

        let total = self.installer.conditional_patterns.len();
        Logger::instance().log(&format!(
            "[fomod] Processing {total} conditional file install patterns..."
        ));

        // (FS 656-679) The tallies exist only to be logged. `processed` is
        // pre-incremented before the per-pattern line, so it reads as a 1-based
        // "N of total" counter over the patterns that actually matched.
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

// ---------------------------------------------------------------------------
// Free helpers (private members in the C++, none of which touch instance state)
// ---------------------------------------------------------------------------

/// Convert an IR file entry into a [`FileOperation`] with absolute paths. Mirror
/// of `FomodService::enqueue_entry`.
///
/// - an empty `source` enqueues nothing;
/// - a destination rejected by [`is_safe_destination`] enqueues nothing and logs
///   a warning;
/// - otherwise `src_base/source` and `dst_base/destination` are joined and
///   `next_doc_order` is POST-incremented into the new operation.
///
/// The guard checks the NORMALIZED destination while the join uses the RAW one,
/// so a rooted destination such as `/etc/passwd` passes (normalization strips the
/// leading slash) and then REPLACES the base during the join - the root-component
/// rule is the same in `std::filesystem::operator/` and `Path::join`. That hole
/// exists in the C++ and is reproduced here rather than fixed.
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

/// Enqueue every file entry of a plugin, in document order. Mirror of
/// `FomodService::enqueue_plugin_files`.
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

/// Build the composite dedup key for a plugin. Mirror of
/// `FomodService::make_plugin_key`: `to_lower(step) + "\x1f" + to_lower(group) +
/// "\x1f" + to_lower(plugin)`.
///
/// The separator is ASCII Unit Separator (0x1F), which cannot appear in XML text
/// content, so names carrying the separator cannot collide.
fn make_plugin_key(step: &str, group: &str, plugin: &str) -> String {
    format!(
        "{}\x1f{}\x1f{}",
        to_lower(step),
        to_lower(group),
        to_lower(plugin)
    )
}

/// Copy a plugin's `<conditionFlags>` into the flag map, skipping EMPTY flag
/// names. Mirror of the identical three-line loop the C++ repeats at
/// `FomodService.cpp:478-482`, `:514-518` and `:555-559`.
fn apply_condition_flags(plugin: &FomodPlugin, flags: &mut HashMap<String, String>) {
    for (name, value) in &plugin.condition_flags {
        if !name.is_empty() {
            flags.insert(name.clone(), value.clone());
        }
    }
}

/// True when a step carries a visibility condition that does NOT hold. Mirror of
/// the C++ `step.visible && !evaluate_condition(*step.visible, ...)` guard: a
/// step with no condition is always visible.
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

/// Post-increment the occurrence counter for `name`, returning the value BEFORE
/// the increment. Mirror of the C++ `occurrence[name]++` idiom (a missing key
/// default-constructs to 0 first).
fn post_increment(counters: &mut HashMap<String, usize>, name: &str) -> usize {
    let slot = counters.entry(name.to_string()).or_insert(0);
    let before = *slot;
    *slot += 1;
    before
}

/// Schema-tolerant plugin name reader. Mirror of the anon-namespace
/// `read_plugin_name` in `FomodService.cpp:22-33`.
///
/// Accepts BOTH the legacy schema-v1 form (`plugins: ["Name", ...]`) and
/// schema-v2 (`plugins: [{"name": "Name", ...}, ...]`), returning `""` for
/// anything else so the caller SKIPS the entry. Note this contradicts the
/// (stale) `FomodService.hpp` class doc, which still describes a
/// `get<std::string>()` that throws on a non-string entry.
fn read_plugin_name(entry: &Value) -> String {
    if let Some(s) = entry.as_str() {
        return s.to_string();
    }
    // `Value::get` already yields `None` for a non-object, so this single lookup
    // covers the C++ `is_object() && contains("name") && name.is_string()` chain.
    if let Some(name) = entry.get("name").and_then(Value::as_str) {
        return name.to_string();
    }
    String::new()
}

/// Read the `name` field the way the C++ `json.value("name", "")` does,
/// returning `None` where that call would THROW. Same tri-state as
/// `fomod_inference_service::name_field`:
///
/// - not an object                  -> `None` (type_error 306)
/// - object, no `name`              -> `Some("")` (the default is used)
/// - object, `name` is a string     -> `Some(s)`
/// - object, `name` is not a string -> `None` (type_error 302)
fn name_field(src: &Value) -> Option<&str> {
    if !src.is_object() {
        return None;
    }
    match src.get("name") {
        None => Some(""),
        Some(value) => value.as_str(),
    }
}

/// Borrow the elements of a JSON array value; an empty slice for anything else
/// (including `None`).
fn array_items(value: Option<&Value>) -> &[Value] {
    match value {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

/// Check a group's selection count against its cardinality constraint. Mirror of
/// the file-static `validate_cardinality` in `FomodService.cpp:172-188`.
fn validate_cardinality(group_type: FomodGroupType, selected: i32, total: i32) -> bool {
    match group_type {
        FomodGroupType::SelectExactlyOne => selected == 1,
        FomodGroupType::SelectAtLeastOne => selected >= 1,
        FomodGroupType::SelectAtMostOne => selected <= 1,
        FomodGroupType::SelectAll => selected == total,
        FomodGroupType::SelectAny => true,
    }
}

// ---------------------------------------------------------------------------
// execute_file_operations
// ---------------------------------------------------------------------------

/// Sort queued operations and execute every copy. Mirror of the static
/// `FomodService::execute_file_operations`.
///
/// Stable sort by ascending `priority`, then ascending `document_order`, so the
/// highest-priority operation for a destination runs LAST and wins (MO2's
/// last-write-wins rule). Individual failures are counted, never fatal; `ops` is
/// cleared afterwards regardless. Returns the number of failed operations.
///
/// That count is ALWAYS 0 in practice, in both languages:
/// [`FileOperations::copy_file`] / [`FileOperations::copy_folder`] swallow every
/// I/O error internally (the C++ statics are documented and implemented as
/// non-throwing), so the C++ `try`/`catch` this loop mirrors can never fire. The
/// counting is ported anyway because it is observable through the return value,
/// which the caller logs.
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
        // The back end cannot report a failure, exactly like the C++ statics.
        false
    })
}

/// The sort-execute-clear body of [`execute_file_operations`], with the copy back
/// end injected (returning `true` for a FAILED operation) so tests can observe
/// the executed order and the failure counting without touching disk.
fn execute_file_operations_with<F>(ops: &mut Vec<FileOperation>, mut copy: F) -> i32
where
    F: FnMut(&FileOperation) -> bool,
{
    // MO2 uses a stable sort: priority ascending, then XML document order as the
    // tiebreaker. Higher priority is processed later and overwrites.
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
            // (FS 720-723) The C++ appends `: {ex.what()}` from the caught
            // exception. The back end here reports failure as a bool and carries
            // no message, so the trailing `: reason` has no counterpart.
            Logger::instance().log_error(&format!(
                "[fomod] Failed to execute file operation: {} -> {}",
                op.source, op.destination
            ));
        }
    }

    // (FS 727-733) `ops.len()` is read BEFORE the clear, as in the C++ where the
    // warning precedes `ops.clear()`.
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

    // --- IR fixture builders ------------------------------------------------

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

    // --- JSON fixture builders ----------------------------------------------

    /// `{"name": <name>, <key>: [<items>]}`.
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

    /// schema-v2 plugin entry: `{"name": "..."}`.
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

    /// Run `process_optional_files` over a fresh service, returning the queued
    /// destinations (basename-free, just the raw joined strings) on success.
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

    // --- make_plugin_key ----------------------------------------------------

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
        // Names containing the ASCII separators FOMOD text can carry (spaces,
        // slashes) cannot collide across component boundaries.
        assert_ne!(
            make_plugin_key("a b", "c", "d"),
            make_plugin_key("a", "b c", "d")
        );
    }

    // --- enqueue_entry ------------------------------------------------------

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
        // A drive-letter destination survives normalize_path and is rejected.
        enqueue_entry(
            &entry("a.esp", "C:/windows/x"),
            "src",
            "dst",
            &mut ops,
            &mut order,
        );
        // `..` segments are stripped by normalize_path, so they normalize to a
        // path inside the mod root and are accepted (as they are in the C++).
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
        // is_safe_destination normalizes leading slashes away, so "/etc/passwd"
        // passes the guard - and the JOIN then uses the RAW destination, whose
        // root component replaces the base in BOTH std::filesystem and Rust.
        // The C++ has exactly this hole; it is reproduced, not fixed.
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

    // --- execute_file_operations -------------------------------------------

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
        // FileOperations::copy_file swallows its own I/O errors (a missing source
        // is a silent skip), so the failure count is 0 even for paths that cannot
        // possibly be copied - the same result the non-throwing C++ statics give.
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

    // --- required and conditional files ------------------------------------

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

        // No flags set: neither pattern matches.
        let mut service = FomodService::new();
        service.set_installer(ir.clone());
        let mut ops = Vec::new();
        let mut order = 0;
        service.process_conditional_files("src", "dst", None, &mut ops, &mut order);
        assert!(ops.is_empty());

        // With mode=on set by a selected plugin, only the first pattern fires.
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

    // --- module dependencies ------------------------------------------------

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

    // --- read_plugin_name / name_field --------------------------------------

    #[test]
    fn read_plugin_name_accepts_both_schemas_and_skips_the_rest() {
        assert_eq!(read_plugin_name(&Value::string("A")), "A");
        assert_eq!(read_plugin_name(&json_plugin_v2("B")), "B");
        // Anything else yields "" so the caller SKIPS it - the tolerant CODE, not
        // the stale header doc that claims a throw.
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

    // --- both selections schemas -------------------------------------------

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
        // Two IR plugins share the name "Dup". A junk entry between the two JSON
        // selections must NOT advance the occurrence counter, so the second
        // readable "Dup" still binds to the SECOND IR plugin.
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

    // --- occurrence-based matching -----------------------------------------

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
        // The first JSON step has no `groups` array: the C++ increments the step
        // occurrence counter and continues, so the SECOND JSON step binds to the
        // second IR step.
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
        // Only ONE IR step named "S". Three JSON steps named "S": the first binds
        // (occ 0), the second and third miss (occ 1 and 2) WITHOUT double
        // incrementing, which would matter if more IR steps existed.
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

    // --- visibility, dependencies, flags ------------------------------------

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
        // A second plugin whose dependency needs the flag the first one sets;
        // flags from an earlier selection are visible to later ones.
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

    // --- Required auto-install ---------------------------------------------

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
        // Effective type comes from evaluate_plugin_type, so a pattern that flips
        // an Optional plugin to Required is honoured.
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

        // A context is required for evaluate_plugin_type to use Normal-mode
        // evaluation; the flag leaf resolves the same either way here.
        let (_, ops) = run_optional(ir, &config).expect("well-formed");
        assert_eq!(sources(&ops), vec![joined("src", "pat.esp")]);
    }

    // --- alwaysInstall / installIfUsable ------------------------------------

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
                // The selected plugin's own files come from pass 1 (its
                // alwaysInstall flag is irrelevant - it is already processed).
                joined("src", "sel-always.esp"),
                joined("src", "always.esp"),
                joined("src", "usable.esp"),
                // NotUsable keeps alwaysInstall but drops installIfUsable.
                joined("src", "nu-always.esp"),
            ]
        );
    }

    #[test]
    fn required_plugins_are_excluded_from_the_auto_install_pass() {
        // A Required plugin is installed WHOLE by pass 1/2, so pass 3 must not
        // re-enqueue its alwaysInstall entry.
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
        // The C++ skips the whole optional phase (and its try/catch) when `steps`
        // is missing or is not an array.
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

    // --- error + rollback ---------------------------------------------------

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

        // Pre-existing operations (e.g. from process_required_files) must survive
        // the rollback untouched.
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

        // A non-object step element: C++ type_error 306.
        let config = selections(vec![Value::string("S")]);
        let mut service = FomodService::new();
        service.set_installer(ir.clone());
        let mut ops = Vec::new();
        let mut order = 0;
        assert_eq!(
            service.process_optional_files(&config, "src", "dst", None, &mut ops, &mut order),
            Err(SelectionsError::NameTypeError)
        );

        // A non-string group name: C++ type_error 302.
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

    // --- validate_json_selections -------------------------------------------

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
        // An empty group entry never reaches selected_plugins, so the group is
        // simply not validated - the failure needs a selection the IR does not
        // contain.
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
