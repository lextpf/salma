/*!
 * @brief evaluates compiled FOMOD dependency trees.
 * @author Alex (https://github.com/lextpf)
 *
 * normal mode reads the dependency context and filesystem. inferred mode uses external
 * overrides and performs no filesystem access. both modes share flag and composite semantics.
 *
 * evaluation is bounded by MAX_DEPENDENCY_DEPTH.
 */

use std::collections::HashMap;
use std::path::Path;

use crate::fomod_ir::{FomodCondition, FomodConditionOp, FomodConditionType, FomodPlugin};
use crate::logger::Logger;
use crate::types::{FomodDependencyContext, PluginType};
use crate::utils::{normalize_path, to_lower};

/**
 * @brief maximum nesting depth for recursive condition evaluation, a guard against malformed XML.
 * @author Alex (https://github.com/lextpf)
 *
 * both users count from depth 0 at the outermost `<dependencies>`, and both degrade rather than
 * fail when the bound is passed.
 */
pub const MAX_DEPENDENCY_DEPTH: i32 = 32;

/**
 * @enum ExternalConditionOverride
 * @brief how inferred mode answers an external dependency.
 * @author Alex (https://github.com/lextpf)
 *
 * consulted only in inferred mode; no variant reads the filesystem or the game state, because
 * inferred mode never probes either.
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ExternalConditionOverride {
    /**
     * @brief external state cannot be determined.
     * @author Alex (https://github.com/lextpf)
     */
    #[default]
    Unknown = 0,
    ForceFalse = 1,
    ForceTrue = 2,
}

// compare two integer version vectors element-wise, padding the shorter one with zeros: "1.2"
// equals "1.2.0" and is less than "1.2.1".
fn compare_version_parts(x: &[i32], y: &[i32]) -> i32 {
    for i in 0..x.len().max(y.len()) {
        let xv = x.get(i).copied().unwrap_or(0);
        let yv = y.get(i).copied().unwrap_or(0);
        if xv < yv {
            return -1;
        }
        if xv > yv {
            return 1;
        }
    }
    0
}

// parse a FOMOD version string into an integer vector: drop every character that is neither an
// ASCII digit nor '.', split on '.', parse each token, then tail-pad to length 3.
// a token that will not parse, meaning an empty one or one wider than `i32`, becomes 0 and writes a
// `[fomod]` warning to `logs/salma.log`.
fn parse_version_parts(version_string: &str) -> Vec<i32> {
    // keep ASCII digits and dots only. a non-ASCII digit is dropped like any other stray character.
    let cleaned: String = version_string
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    let mut parts: Vec<i32> = Vec::new();
    if !cleaned.is_empty() {
        let mut tokens: Vec<&str> = cleaned.split('.').collect();
        if cleaned.ends_with('.') {
            // a trailing delimiter contributes no token; see the doc comment.
            tokens.pop();
        }
        for token in tokens {
            // empty token (leading or consecutive dot) and i32 overflow both recover as 0 after a
            // warning. neither aborts the parse.
            match token.parse::<i32>() {
                Ok(value) => parts.push(value),
                Err(err) => {
                    Logger::instance().log_warning(&format!(
                        "[fomod] Malformed version component \"{token}\": {err}"
                    ));
                    parts.push(0);
                }
            }
        }
    }
    while parts.len() < 3 {
        parts.push(0);
    }
    parts
}

// filename component under MSVC std::filesystem::path rules: everything after the last '/' or '\'
// separator, and empty when the path ends with a separator.
// two root-name rules make the no-separator cases surprising.
fn cpp_path_filename(path: &str) -> &str {
    let bytes = path.as_bytes();
    let is_sep = |b: u8| b == b'/' || b == b'\\';
    // UNC root-name rule: when nothing after the two leading separators holds another separator,
    // the whole path is the root-name and the filename is empty. the byte-wise scan is safe because
    // '/' and '\' are ASCII and never occur inside a UTF-8 continuation sequence.
    if bytes.len() >= 3
        && is_sep(bytes[0])
        && is_sep(bytes[1])
        && !is_sep(bytes[2])
        && !bytes[3..].iter().any(|&b| is_sep(b))
    {
        return "";
    }
    match path.rfind(['/', '\\']) {
        Some(pos) => &path[pos + 1..],
        None => {
            if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                &path[2..]
            } else {
                path
            }
        }
    }
}

// extension under MSVC std::filesystem::path rules, including the leading dot (".esp").
fn cpp_path_extension(path: &str) -> &str {
    let filename = cpp_path_filename(path);
    if filename == "." || filename == ".." {
        return "";
    }
    match filename.rfind('.') {
        Some(0) | None => "",
        Some(pos) => &filename[pos..],
    }
}

// existence probe that never fails: any I/O error answers "does not exist".
fn safe_exists(p: &Path) -> bool {
    p.exists()
}

fn is_plugin_extension(file_path: &str) -> bool {
    let ext_lower = to_lower(cpp_path_extension(file_path));
    matches!(ext_lower.as_str(), ".esp" | ".esm" | ".esl")
}

// decide a fileDependency leaf in normal mode.
// a missing file answers true for every extension, because the plugin branch is guarded by
// `file_exists` and the fallthrough answer is `!file_exists`.
fn eval_file_dep(file_path: &str, state: &str, ctx: Option<&FomodDependencyContext>) -> bool {
    if file_path.is_empty() {
        return false;
    }

    let normalized = normalize_path(file_path);

    let mut file_exists = false;
    if let Some(ctx) = ctx {
        if ctx.installed_files.contains(&normalized) {
            file_exists = true;
        }
        // the extension and filename checks run on the original file_path, not on the normalized
        // one.
        if !file_exists && is_plugin_extension(file_path) {
            let lower_name = to_lower(cpp_path_filename(file_path));
            if ctx.installed_plugins.contains(&lower_name) {
                file_exists = true;
            }
        }
        if !file_exists && !ctx.archive_root.is_empty() {
            let full = Path::new(&ctx.archive_root).join(&normalized);
            if safe_exists(&full) {
                file_exists = true;
            }
        }
        if !file_exists && !ctx.game_path.is_empty() {
            let full = Path::new(&ctx.game_path).join(&normalized);
            if safe_exists(&full) {
                file_exists = true;
            }
        }
    }

    if state == "Missing" {
        return !file_exists;
    }

    if state == "Inactive" {
        // for a plugin file (.esp/.esm/.esl), "Inactive" means the file exists but is not in the
        // active plugin list. FOMOD gives it no meaning for a non-plugin file, so those fall back
        // to !file_exists, that is, to "Missing" semantics.
        if file_exists {
            if let Some(ctx) = ctx {
                if is_plugin_extension(file_path) {
                    let lower_name = to_lower(cpp_path_filename(file_path));
                    return !ctx.installed_plugins.contains(&lower_name);
                }
            }
        }
        return !file_exists;
    }

    // unknown states log an untagged warning and use Active semantics.
    if state != "Active" {
        Logger::instance().log_warning(&format!(
            "Unknown file dependency state: {state} for file: {file_path}, treating as Active"
        ));
    }
    file_exists
}

// decide a gameDependency leaf: true unless the context supplies both a game_path and a
// game_version and the installed version is lower than the required one.
// no context, an empty `game_path`, an empty required version or an empty `game_version` all answer
// true.
fn eval_game_dep(version: &str, ctx: Option<&FomodDependencyContext>) -> bool {
    let Some(ctx) = ctx else {
        return true; // standalone mode
    };
    if ctx.game_path.is_empty() {
        return true; // standalone mode
    }

    if !version.is_empty() && !ctx.game_version.is_empty() {
        return compare_version_parts(
            &parse_version_parts(&ctx.game_version),
            &parse_version_parts(version),
        ) >= 0;
    }

    true
}

// decide a pluginDependency leaf.
// every other value, including `"Active"` and the empty string, asks for an active plugin.
fn eval_plugin_dep(
    plugin_name: &str,
    plugin_type: &str,
    ctx: Option<&FomodDependencyContext>,
) -> bool {
    if plugin_name.is_empty() {
        return false;
    }

    let lower_name = to_lower(plugin_name);
    let is_active = ctx.is_some_and(|c| c.installed_plugins.contains(&lower_name));

    let mut file_exists = is_active;
    if !file_exists {
        if let Some(ctx) = ctx {
            if !ctx.game_path.is_empty() {
                // the join uses the raw plugin_name, not the lowered one.
                let data_path = Path::new(&ctx.game_path).join("Data").join(plugin_name);
                if safe_exists(&data_path) {
                    file_exists = true;
                }
            }
        }
    }

    if plugin_type == "Inactive" {
        file_exists && !is_active
    } else {
        is_active // "Active" (default), and any other type string
    }
}

// decide a fomodDependency leaf: a case-sensitive exact match against ctx.installed_fomods.
// an empty name is false.
fn eval_fomod_dep(fomod_name: &str, ctx: Option<&FomodDependencyContext>) -> bool {
    if fomod_name.is_empty() {
        return false;
    }
    ctx.is_some_and(|c| c.installed_fomods.contains(fomod_name))
}

// decide a fommDependency leaf against the FOMM version MO2 hardcodes, "0.13.21".
// true when the required version is empty, or is less than or equal to that.
fn eval_fomm_version(version: &str) -> bool {
    if version.is_empty() {
        return true;
    }
    let actual = parse_version_parts("0.13.21");
    let required = parse_version_parts(version);
    compare_version_parts(&required, &actual) <= 0
}

fn eval_leaf_normal(c: &FomodCondition, ctx: Option<&FomodDependencyContext>) -> bool {
    match c.r#type {
        FomodConditionType::File => eval_file_dep(&c.file_path, &c.file_state, ctx),
        FomodConditionType::Game => eval_game_dep(&c.version, ctx),
        FomodConditionType::Plugin => eval_plugin_dep(&c.plugin_name, &c.plugin_type, ctx),
        FomodConditionType::Fomod => eval_fomod_dep(&c.fomod_name, ctx),
        FomodConditionType::Fomm => eval_fomm_version(&c.version),
        FomodConditionType::Fose => true,
        // evaluate_condition_core handles Flag and Composite, so neither reaches here. the arm
        // answers true to keep the match exhaustive.
        FomodConditionType::Flag | FomodConditionType::Composite => true,
    }
}

// inferred-mode leaf dispatch: the external types (File, Plugin, Fomod) follow the override, so
// both Unknown and ForceFalse answer false.
fn eval_leaf_inferred(c: &FomodCondition, external_override: ExternalConditionOverride) -> bool {
    match c.r#type {
        FomodConditionType::File | FomodConditionType::Plugin | FomodConditionType::Fomod => {
            external_override == ExternalConditionOverride::ForceTrue
        }
        _ => true,
    }
}

fn evaluate_condition_core(
    condition: &FomodCondition,
    flags: &HashMap<String, String>,
    eval_leaf: &dyn Fn(&FomodCondition) -> bool,
    depth: i32,
) -> bool {
    match condition.r#type {
        FomodConditionType::Composite => {
            if depth > MAX_DEPENDENCY_DEPTH {
                Logger::instance().log_warning(
                    "[fomod-ir] Condition tree exceeds maximum depth, treating as unmet",
                );
                return false;
            }
            let is_and = condition.op == FomodConditionOp::And;
            // empty And is true. empty Or is false and represents a rejected over-depth subtree.
            let mut result = is_and;
            for child in &condition.children {
                let child_met = evaluate_condition_core(child, flags, eval_leaf, depth + 1);
                if is_and {
                    result = result && child_met;
                    if !result {
                        return false; // Short-circuit
                    }
                } else {
                    result = result || child_met;
                    if result {
                        return true; // Short-circuit
                    }
                }
            }
            result
        }
        // flags are answered here, ahead of leaf dispatch, which is why they evaluate identically
        // in both modes. a missing flag is true only when the expected value is empty; a present
        // flag is an exact case-sensitive comparison, with no trimming.
        FomodConditionType::Flag => match flags.get(&condition.flag_name) {
            None => condition.flag_value.is_empty(),
            Some(actual) => *actual == condition.flag_value,
        },
        _ => eval_leaf(condition),
    }
}

pub fn evaluate_condition(
    condition: &FomodCondition,
    flags: &HashMap<String, String>,
    context: Option<&FomodDependencyContext>,
) -> bool {
    evaluate_condition_core(condition, flags, &|c| eval_leaf_normal(c, context), 0)
}

pub fn evaluate_condition_inferred(
    condition: &FomodCondition,
    flags: &HashMap<String, String>,
    external_override: ExternalConditionOverride,
    _context: Option<&FomodDependencyContext>,
) -> bool {
    evaluate_condition_core(
        condition,
        flags,
        &|c| eval_leaf_inferred(c, external_override),
        0,
    )
}

// the first matching type pattern wins. absence retains the declared type.
pub fn evaluate_plugin_type(
    plugin: &FomodPlugin,
    flags: &HashMap<String, String>,
    context: Option<&FomodDependencyContext>,
) -> PluginType {
    for pattern in &plugin.type_patterns {
        let matched = if context.is_some() {
            evaluate_condition(&pattern.condition, flags, context)
        } else {
            evaluate_condition_inferred(
                &pattern.condition,
                flags,
                ExternalConditionOverride::Unknown,
                None,
            )
        };
        if matched {
            return pattern.result_type;
        }
    }
    plugin.r#type
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_ir::FomodTypePattern;
    use crate::utils::{RANDOM_HEX_DEFAULT_LEN, random_hex_string};
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    fn flags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn flag_leaf(name: &str, value: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Flag,
            flag_name: name.to_string(),
            flag_value: value.to_string(),
            ..FomodCondition::default()
        }
    }

    fn file_leaf(path: &str, state: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::File,
            file_path: path.to_string(),
            file_state: state.to_string(),
            ..FomodCondition::default()
        }
    }

    fn plugin_leaf(name: &str, ptype: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Plugin,
            plugin_name: name.to_string(),
            plugin_type: ptype.to_string(),
            ..FomodCondition::default()
        }
    }

    fn fomod_leaf(name: &str) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Fomod,
            fomod_name: name.to_string(),
            ..FomodCondition::default()
        }
    }

    fn version_leaf(kind: FomodConditionType, version: &str) -> FomodCondition {
        FomodCondition {
            r#type: kind,
            version: version.to_string(),
            ..FomodCondition::default()
        }
    }

    fn composite(op: FomodConditionOp, children: Vec<FomodCondition>) -> FomodCondition {
        FomodCondition {
            r#type: FomodConditionType::Composite,
            op,
            children,
            ..FomodCondition::default()
        }
    }

    fn nested_chain(levels: usize) -> FomodCondition {
        let mut node = composite(FomodConditionOp::And, vec![flag_leaf("missing", "")]);
        for _ in 1..levels {
            node = composite(FomodConditionOp::And, vec![node]);
        }
        node
    }

    fn empty_ctx() -> FomodDependencyContext {
        FomodDependencyContext::default()
    }

    fn with_temp_dir(f: impl FnOnce(&Path)) {
        let dir = std::env::temp_dir().join(format!(
            "salma_rs_dep_eval_{}",
            random_hex_string(RANDOM_HEX_DEFAULT_LEN)
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        f(&dir);
        fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[test]
    fn empty_and_is_true_empty_or_is_false() {
        let no_flags = HashMap::new();
        let empty_and = composite(FomodConditionOp::And, vec![]);
        let empty_or = composite(FomodConditionOp::Or, vec![]);
        assert!(evaluate_condition(&empty_and, &no_flags, None));
        assert!(!evaluate_condition(&empty_or, &no_flags, None));
        assert!(evaluate_condition_inferred(
            &empty_and,
            &no_flags,
            ExternalConditionOverride::ForceTrue,
            None
        ));
        assert!(!evaluate_condition_inferred(
            &empty_or,
            &no_flags,
            ExternalConditionOverride::ForceTrue,
            None
        ));
    }

    #[test]
    fn and_short_circuits_on_first_false_or_on_first_true() {
        let no_flags = HashMap::new();
        let f = flag_leaf("f", "set"); // missing flag, non-empty value -> false
        let t = flag_leaf("f", ""); // missing flag, empty value -> true
        let and = composite(FomodConditionOp::And, vec![f.clone(), t.clone()]);
        let or = composite(FomodConditionOp::Or, vec![t, f]);
        assert!(!evaluate_condition(&and, &no_flags, None));
        assert!(evaluate_condition(&or, &no_flags, None));
    }

    #[test]
    fn depth_32_composite_evaluates_depth_33_is_unmet() {
        let no_flags = HashMap::new();
        assert!(evaluate_condition(&nested_chain(33), &no_flags, None));
        assert!(!evaluate_condition(&nested_chain(34), &no_flags, None));
    }

    #[test]
    fn missing_flag_with_empty_expected_value_is_true() {
        let no_flags = HashMap::new();
        assert!(evaluate_condition(&flag_leaf("f", ""), &no_flags, None));
    }

    #[test]
    fn missing_flag_with_non_empty_expected_value_is_false() {
        let no_flags = HashMap::new();
        assert!(!evaluate_condition(&flag_leaf("f", "On"), &no_flags, None));
    }

    #[test]
    fn flag_comparison_is_exact_and_case_sensitive() {
        let state = flags(&[("f", "On")]);
        assert!(evaluate_condition(&flag_leaf("f", "On"), &state, None));
        assert!(!evaluate_condition(&flag_leaf("f", "on"), &state, None));
        assert!(!evaluate_condition(&flag_leaf("f", "On "), &state, None));
        assert!(!evaluate_condition(&flag_leaf("f", ""), &state, None));
    }

    #[test]
    fn flags_evaluate_identically_in_inferred_mode() {
        let state = flags(&[("f", "On")]);
        for ov in [
            ExternalConditionOverride::Unknown,
            ExternalConditionOverride::ForceFalse,
            ExternalConditionOverride::ForceTrue,
        ] {
            assert!(evaluate_condition_inferred(
                &flag_leaf("f", "On"),
                &state,
                ov,
                None
            ));
            assert!(!evaluate_condition_inferred(
                &flag_leaf("f", "Off"),
                &state,
                ov,
                None
            ));
        }
    }

    #[test]
    fn file_dep_empty_path_is_false_for_every_state() {
        let no_flags = HashMap::new();
        let ctx = empty_ctx();
        for state in ["Active", "Missing", "Inactive", ""] {
            assert!(
                !evaluate_condition(&file_leaf("", state), &no_flags, Some(&ctx)),
                "state {state:?}"
            );
        }
    }

    #[test]
    fn file_dep_installed_files_hit_uses_normalized_path() {
        let no_flags = HashMap::new();
        let ctx = FomodDependencyContext {
            installed_files: set(&["textures/armor/file.dds"]),
            ..empty_ctx()
        };
        let cond = file_leaf("Textures\\Armor\\File.DDS", "Active");
        assert!(evaluate_condition(&cond, &no_flags, Some(&ctx)));
        assert!(!evaluate_condition(
            &file_leaf("Textures\\Armor\\Other.DDS", "Active"),
            &no_flags,
            Some(&ctx)
        ));
    }

    #[test]
    fn file_dep_plugin_extension_falls_back_to_installed_plugins() {
        let no_flags = HashMap::new();
        let ctx = FomodDependencyContext {
            installed_plugins: set(&["skyui.esp"]),
            ..empty_ctx()
        };
        assert!(evaluate_condition(
            &file_leaf("Data/SkyUI.esp", "Active"),
            &no_flags,
            Some(&ctx)
        ));
        let ctx2 = FomodDependencyContext {
            installed_plugins: set(&["skyui.txt"]),
            ..empty_ctx()
        };
        assert!(!evaluate_condition(
            &file_leaf("Data/SkyUI.txt", "Active"),
            &no_flags,
            Some(&ctx2)
        ));
        let ctx3 = FomodDependencyContext {
            installed_plugins: set(&["a.esm", "b.esl"]),
            ..empty_ctx()
        };
        assert!(evaluate_condition(
            &file_leaf("A.esm", "Active"),
            &no_flags,
            Some(&ctx3)
        ));
        assert!(evaluate_condition(
            &file_leaf("B.esl", "Active"),
            &no_flags,
            Some(&ctx3)
        ));
    }

    #[test]
    fn file_dep_missing_state_inverts_existence() {
        let no_flags = HashMap::new();
        let ctx = FomodDependencyContext {
            installed_files: set(&["present.txt"]),
            ..empty_ctx()
        };
        assert!(!evaluate_condition(
            &file_leaf("present.txt", "Missing"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(evaluate_condition(
            &file_leaf("absent.txt", "Missing"),
            &no_flags,
            Some(&ctx)
        ));
    }

    #[test]
    fn file_dep_inactive_on_plugin_file_checks_active_plugin_list() {
        let no_flags = HashMap::new();
        let ctx = FomodDependencyContext {
            installed_files: set(&["mod.esp"]),
            ..empty_ctx()
        };
        assert!(evaluate_condition(
            &file_leaf("Mod.esp", "Inactive"),
            &no_flags,
            Some(&ctx)
        ));
        let ctx2 = FomodDependencyContext {
            installed_files: set(&["mod.esp"]),
            installed_plugins: set(&["mod.esp"]),
            ..empty_ctx()
        };
        assert!(!evaluate_condition(
            &file_leaf("Mod.esp", "Inactive"),
            &no_flags,
            Some(&ctx2)
        ));
    }

    #[test]
    fn file_dep_inactive_on_non_plugin_file_means_missing() {
        let no_flags = HashMap::new();
        let ctx = FomodDependencyContext {
            installed_files: set(&["readme.txt"]),
            ..empty_ctx()
        };
        assert!(!evaluate_condition(
            &file_leaf("readme.txt", "Inactive"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(evaluate_condition(
            &file_leaf("gone.txt", "Inactive"),
            &no_flags,
            Some(&ctx)
        ));
    }

    #[test]
    fn file_dep_unknown_state_is_treated_as_active() {
        let no_flags = HashMap::new();
        let ctx = FomodDependencyContext {
            installed_files: set(&["present.txt"]),
            ..empty_ctx()
        };
        assert!(evaluate_condition(
            &file_leaf("present.txt", "Enabled"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(!evaluate_condition(
            &file_leaf("absent.txt", "Enabled"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(evaluate_condition(
            &file_leaf("present.txt", "missing"),
            &no_flags,
            Some(&ctx)
        ));
    }

    #[test]
    fn file_dep_without_context_never_exists() {
        let no_flags = HashMap::new();
        assert!(!evaluate_condition(
            &file_leaf("anything.esp", "Active"),
            &no_flags,
            None
        ));
        assert!(evaluate_condition(
            &file_leaf("anything.esp", "Missing"),
            &no_flags,
            None
        ));
        assert!(evaluate_condition(
            &file_leaf("anything.esp", "Inactive"),
            &no_flags,
            None
        ));
    }

    #[test]
    fn file_dep_probes_archive_root_then_game_path_on_disk() {
        with_temp_dir(|dir| {
            let no_flags = HashMap::new();
            let archive_root = dir.join("archive");
            let game_path = dir.join("game");
            fs::create_dir_all(archive_root.join("sub")).unwrap();
            fs::create_dir_all(game_path.join("sub")).unwrap();
            fs::write(archive_root.join("sub/in_archive.txt"), b"x").unwrap();
            fs::write(game_path.join("sub/in_game.txt"), b"x").unwrap();

            let ctx = FomodDependencyContext {
                archive_root: archive_root.to_string_lossy().into_owned(),
                game_path: game_path.to_string_lossy().into_owned(),
                ..empty_ctx()
            };
            assert!(evaluate_condition(
                &file_leaf("Sub\\In_Archive.txt", "Active"),
                &no_flags,
                Some(&ctx)
            ));
            assert!(evaluate_condition(
                &file_leaf("sub/in_game.txt", "Active"),
                &no_flags,
                Some(&ctx)
            ));
            assert!(!evaluate_condition(
                &file_leaf("sub/nowhere.txt", "Active"),
                &no_flags,
                Some(&ctx)
            ));
            let ctx_no_probe = FomodDependencyContext { ..empty_ctx() };
            assert!(!evaluate_condition(
                &file_leaf("sub/in_archive.txt", "Active"),
                &no_flags,
                Some(&ctx_no_probe)
            ));
        });
    }

    #[test]
    fn cpp_path_helpers_mirror_msvc_fs_path() {
        assert_eq!(cpp_path_extension("mod.esp"), ".esp");
        assert_eq!(cpp_path_extension("dir/mod.esp"), ".esp");
        assert_eq!(cpp_path_extension("dir\\mod.esp"), ".esp");
        assert_eq!(cpp_path_extension("mod"), "");
        assert_eq!(cpp_path_extension(".gitignore"), "");
        assert_eq!(cpp_path_extension("dir/.gitignore"), "");
        assert_eq!(cpp_path_extension(".profile.txt"), ".txt");
        assert_eq!(cpp_path_extension("file."), ".");
        assert_eq!(cpp_path_extension("."), "");
        assert_eq!(cpp_path_extension(".."), "");
        assert_eq!(cpp_path_extension("dir/.."), "");
        assert_eq!(cpp_path_extension("dir.d/file"), "");

        assert_eq!(cpp_path_filename("a/b/c.esp"), "c.esp");
        assert_eq!(cpp_path_filename("a\\b\\c.esp"), "c.esp");
        assert_eq!(cpp_path_filename("a/b\\c.esp"), "c.esp");
        assert_eq!(cpp_path_filename("c.esp"), "c.esp");
        assert_eq!(cpp_path_filename("a/"), "");

        // drive root-name decomposition without a separator: MSVC excludes the root-name ("C:")
        // from filename().
        assert_eq!(cpp_path_filename("C:foo.esp"), "foo.esp");
        assert_eq!(cpp_path_filename("c:foo.esp"), "foo.esp");
        assert_eq!(cpp_path_filename("C:"), "");
        assert_eq!(cpp_path_filename("C:/foo.esp"), "foo.esp");
        assert_eq!(cpp_path_filename("CC:foo"), "CC:foo");
        assert_eq!(cpp_path_filename("1:foo"), "1:foo");
        assert_eq!(cpp_path_filename(":foo"), ":foo");
        assert_eq!(cpp_path_filename("dir/C:bar"), "C:bar");
        assert_eq!(cpp_path_extension("C:foo.esp"), ".esp");
        assert_eq!(cpp_path_extension("C:.esp"), "");
        assert_eq!(cpp_path_extension("C:.gitignore"), "");

        assert_eq!(cpp_path_filename("//server.esp"), "");
        assert_eq!(cpp_path_extension("//server.esp"), "");
        assert_eq!(cpp_path_filename("//server"), "");
        assert_eq!(cpp_path_filename("\\\\server"), "");
        assert_eq!(cpp_path_filename("//C:"), "");
        assert_eq!(cpp_path_filename("\\\\?"), "");
        assert_eq!(cpp_path_filename("//server/share.esp"), "share.esp");
        assert_eq!(cpp_path_extension("//server/share.esp"), ".esp");
        assert_eq!(cpp_path_filename("//server.esp/x.esp"), "x.esp");
        assert_eq!(cpp_path_filename("//server/"), "");
        assert_eq!(cpp_path_filename("\\\\?\\foo"), "foo");
        assert_eq!(cpp_path_filename("\\??\\foo"), "foo");
        // three or more leading separators form no root-name, and "\??" is not a device prefix
        // without its trailing separator.
        assert_eq!(cpp_path_filename("///foo"), "foo");
        assert_eq!(cpp_path_filename("//"), "");
        assert_eq!(cpp_path_filename("\\??"), "??");
    }

    #[test]
    fn file_dep_unc_root_name_is_not_a_plugin() {
        let no_flags = HashMap::new();
        let mut ctx = empty_ctx();
        ctx.installed_plugins.insert("server.esp".to_string());
        assert!(!evaluate_condition(
            &file_leaf("//server.esp", "Active"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(evaluate_condition(
            &file_leaf("//server.esp", "Inactive"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(evaluate_condition(
            &file_leaf("//host/server.esp", "Active"),
            &no_flags,
            Some(&ctx)
        ));
    }

    #[test]
    fn game_dep_standalone_mode_is_true() {
        let no_flags = HashMap::new();
        let leaf = version_leaf(FomodConditionType::Game, "1.5.97");
        assert!(evaluate_condition(&leaf, &no_flags, None));
        let ctx = empty_ctx();
        assert!(evaluate_condition(&leaf, &no_flags, Some(&ctx)));
    }

    #[test]
    fn game_dep_compares_versions_when_both_present() {
        let no_flags = HashMap::new();
        let ctx = |v: &str| FomodDependencyContext {
            game_path: "C:/Games/Skyrim".to_string(),
            game_version: v.to_string(),
            ..empty_ctx()
        };
        let leaf = version_leaf(FomodConditionType::Game, "1.5.97");
        assert!(evaluate_condition(&leaf, &no_flags, Some(&ctx("1.5.97"))));
        assert!(evaluate_condition(&leaf, &no_flags, Some(&ctx("1.6.1170"))));
        assert!(!evaluate_condition(&leaf, &no_flags, Some(&ctx("1.5.80"))));
        let empty_req = version_leaf(FomodConditionType::Game, "");
        assert!(evaluate_condition(&empty_req, &no_flags, Some(&ctx("1.0"))));
        assert!(evaluate_condition(&leaf, &no_flags, Some(&ctx(""))));
    }

    #[test]
    fn plugin_dep_empty_name_is_false() {
        let no_flags = HashMap::new();
        let ctx = empty_ctx();
        assert!(!evaluate_condition(
            &plugin_leaf("", "Active"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(!evaluate_condition(
            &plugin_leaf("", "Inactive"),
            &no_flags,
            Some(&ctx)
        ));
    }

    #[test]
    fn plugin_dep_active_matches_lowercased_installed_plugins() {
        let no_flags = HashMap::new();
        let ctx = FomodDependencyContext {
            installed_plugins: set(&["skyui.esp"]),
            ..empty_ctx()
        };
        assert!(evaluate_condition(
            &plugin_leaf("SkyUI.esp", "Active"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(evaluate_condition(
            &plugin_leaf("SkyUI.esp", "Enabled"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(evaluate_condition(
            &plugin_leaf("SkyUI.esp", "inactive"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(!evaluate_condition(
            &plugin_leaf("Other.esp", "Active"),
            &no_flags,
            Some(&ctx)
        ));
        // active plugin fails the Inactive check.
        assert!(!evaluate_condition(
            &plugin_leaf("SkyUI.esp", "Inactive"),
            &no_flags,
            Some(&ctx)
        ));
    }

    #[test]
    fn plugin_dep_inactive_uses_game_data_dir_with_raw_name() {
        with_temp_dir(|dir| {
            let no_flags = HashMap::new();
            let data = dir.join("Data");
            fs::create_dir_all(&data).unwrap();
            fs::write(data.join("OnDisk.esp"), b"x").unwrap();

            let ctx = FomodDependencyContext {
                game_path: dir.to_string_lossy().into_owned(),
                ..empty_ctx()
            };
            assert!(evaluate_condition(
                &plugin_leaf("OnDisk.esp", "Inactive"),
                &no_flags,
                Some(&ctx)
            ));
            assert!(!evaluate_condition(
                &plugin_leaf("OnDisk.esp", "Active"),
                &no_flags,
                Some(&ctx)
            ));
            assert!(!evaluate_condition(
                &plugin_leaf("Nowhere.esp", "Inactive"),
                &no_flags,
                Some(&ctx)
            ));
        });
    }

    #[test]
    fn plugin_dep_without_context_is_never_active() {
        let no_flags = HashMap::new();
        assert!(!evaluate_condition(
            &plugin_leaf("SkyUI.esp", "Active"),
            &no_flags,
            None
        ));
        assert!(!evaluate_condition(
            &plugin_leaf("SkyUI.esp", "Inactive"),
            &no_flags,
            None
        ));
    }

    #[test]
    fn fomod_dep_matches_exactly_without_lowercasing() {
        let no_flags = HashMap::new();
        let ctx = FomodDependencyContext {
            installed_fomods: set(&["SkyUI"]),
            ..empty_ctx()
        };
        assert!(evaluate_condition(
            &fomod_leaf("SkyUI"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(!evaluate_condition(
            &fomod_leaf("skyui"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(!evaluate_condition(&fomod_leaf(""), &no_flags, Some(&ctx)));
        assert!(!evaluate_condition(&fomod_leaf("SkyUI"), &no_flags, None));
    }

    #[test]
    fn fomm_dep_boundary_at_hardcoded_version() {
        let no_flags = HashMap::new();
        let fomm = |v: &str| version_leaf(FomodConditionType::Fomm, v);
        assert!(evaluate_condition(&fomm("0.13.21"), &no_flags, None));
        assert!(evaluate_condition(&fomm("0.13.20"), &no_flags, None));
        assert!(evaluate_condition(&fomm("0.12"), &no_flags, None));
        assert!(!evaluate_condition(&fomm("0.13.22"), &no_flags, None));
        assert!(!evaluate_condition(&fomm("1.0"), &no_flags, None));
        assert!(evaluate_condition(&fomm(""), &no_flags, None));
        assert!(evaluate_condition(&fomm("0.13.21.0"), &no_flags, None));
        assert!(!evaluate_condition(&fomm("0.13.21.1"), &no_flags, None));
    }

    #[test]
    fn fose_dep_is_always_true_in_both_modes() {
        let no_flags = HashMap::new();
        let fose = version_leaf(FomodConditionType::Fose, "99.0");
        assert!(evaluate_condition(&fose, &no_flags, None));
        let ctx = empty_ctx();
        assert!(evaluate_condition(&fose, &no_flags, Some(&ctx)));
        for ov in [
            ExternalConditionOverride::Unknown,
            ExternalConditionOverride::ForceFalse,
            ExternalConditionOverride::ForceTrue,
        ] {
            assert!(evaluate_condition_inferred(&fose, &no_flags, ov, None));
        }
    }

    #[test]
    fn inferred_mode_override_matrix() {
        let no_flags = HashMap::new();
        let external: Vec<FomodCondition> = vec![
            file_leaf("a.esp", "Active"),
            plugin_leaf("a.esp", "Active"),
            fomod_leaf("SomeMod"),
        ];
        let infra: Vec<FomodCondition> = vec![
            version_leaf(FomodConditionType::Game, "1.0"),
            version_leaf(FomodConditionType::Fomm, "99.0"),
            version_leaf(FomodConditionType::Fose, "99.0"),
        ];
        for (ov, expected_external) in [
            (ExternalConditionOverride::Unknown, false),
            (ExternalConditionOverride::ForceFalse, false),
            (ExternalConditionOverride::ForceTrue, true),
        ] {
            for leaf in &external {
                assert_eq!(
                    evaluate_condition_inferred(leaf, &no_flags, ov, None),
                    expected_external,
                    "override {ov:?}, leaf {:?}",
                    leaf.r#type
                );
            }
            for leaf in &infra {
                assert!(
                    evaluate_condition_inferred(leaf, &no_flags, ov, None),
                    "override {ov:?}, leaf {:?}",
                    leaf.r#type
                );
            }
        }
    }

    #[test]
    fn inferred_mode_ignores_context_entirely() {
        // a context that would make the File leaf true in normal mode has no effect in inferred
        // mode, which never reads it.
        let no_flags = HashMap::new();
        let ctx = FomodDependencyContext {
            installed_files: set(&["a.esp"]),
            ..empty_ctx()
        };
        let leaf = file_leaf("a.esp", "Active");
        assert!(evaluate_condition(&leaf, &no_flags, Some(&ctx)));
        assert!(!evaluate_condition_inferred(
            &leaf,
            &no_flags,
            ExternalConditionOverride::Unknown,
            Some(&ctx)
        ));
    }

    #[test]
    fn parse_version_parts_table() {
        let cases: &[(&str, &[i32])] = &[
            ("1.2.3", &[1, 2, 3]),
            ("v1.2.3-beta", &[1, 2, 3]),
            ("1.2.3b", &[1, 2, 3]),
            ("1.2.3.99 (custom)", &[1, 2, 3, 99]),
            ("1..2", &[1, 0, 2]),
            (".1", &[0, 1, 0]),
            ("1.2.", &[1, 2, 0]),
            ("1.", &[1, 0, 0]),
            ("", &[0, 0, 0]),
            ("beta", &[0, 0, 0]),
            (".", &[0, 0, 0]),
            ("..", &[0, 0, 0]),
            // i32 overflow token -> 0.
            ("99999999999999999999", &[0, 0, 0]),
            ("1.99999999999999999999.2", &[1, 0, 2]),
            ("7", &[7, 0, 0]),
            ("1.2", &[1, 2, 0]),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_version_parts(input),
                expected.to_vec(),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn compare_version_parts_pads_missing_with_zero() {
        assert_eq!(
            compare_version_parts(&parse_version_parts("1.2"), &parse_version_parts("1.2.0")),
            0
        );
        assert_eq!(
            compare_version_parts(&parse_version_parts("1.2"), &parse_version_parts("1.2.1")),
            -1
        );
        assert_eq!(
            compare_version_parts(&parse_version_parts("1.2.1"), &parse_version_parts("1.2")),
            1
        );
        assert_eq!(compare_version_parts(&[1, 2, 3, 4], &[1, 2, 3]), 1);
        assert_eq!(compare_version_parts(&[1, 2, 3], &[1, 2, 3, 0]), 0);
    }

    fn plugin_with_patterns(
        base: PluginType,
        patterns: Vec<(FomodCondition, PluginType)>,
    ) -> FomodPlugin {
        FomodPlugin {
            name: "P".to_string(),
            r#type: base,
            type_patterns: patterns
                .into_iter()
                .map(|(condition, result_type)| FomodTypePattern {
                    condition,
                    result_type,
                })
                .collect(),
            ..FomodPlugin::default()
        }
    }

    #[test]
    fn plugin_type_first_matching_pattern_wins() {
        let state = flags(&[("f", "On")]);
        let plugin = plugin_with_patterns(
            PluginType::Optional,
            vec![
                (flag_leaf("f", "On"), PluginType::Required),
                (flag_leaf("f", "On"), PluginType::NotUsable),
            ],
        );
        assert_eq!(
            evaluate_plugin_type(&plugin, &state, None),
            PluginType::Required
        );
    }

    #[test]
    fn plugin_type_falls_back_to_declared_type_when_nothing_matches() {
        let no_flags = HashMap::new();
        let plugin = plugin_with_patterns(
            PluginType::Recommended,
            vec![(flag_leaf("f", "On"), PluginType::Required)],
        );
        assert_eq!(
            evaluate_plugin_type(&plugin, &no_flags, None),
            PluginType::Recommended
        );
        let no_patterns = plugin_with_patterns(PluginType::CouldBeUsable, vec![]);
        assert_eq!(
            evaluate_plugin_type(&no_patterns, &no_flags, None),
            PluginType::CouldBeUsable
        );
    }

    #[test]
    fn plugin_type_ctx_presence_selects_normal_vs_inferred_unknown() {
        let no_flags = HashMap::new();
        let plugin = plugin_with_patterns(
            PluginType::Optional,
            vec![(file_leaf("marker.esp", "Active"), PluginType::Required)],
        );
        let ctx = FomodDependencyContext {
            installed_files: set(&["marker.esp"]),
            ..empty_ctx()
        };
        assert_eq!(
            evaluate_plugin_type(&plugin, &no_flags, Some(&ctx)),
            PluginType::Required
        );
        assert_eq!(
            evaluate_plugin_type(&plugin, &no_flags, None),
            PluginType::Optional
        );
    }

    #[test]
    fn plugin_type_none_ctx_dispatches_inferred_not_normal_with_null_ctx() {
        // distinguisher: a File leaf with state "Missing" or "Inactive" is true in normal mode
        // without a context, because file_exists is false, but false under inferred-Unknown, where
        // File leaves follow the override whatever their state. only the inferred dispatch is
        // correct for the None branch, and that is the hot path: the CSP solver always passes no
        // context. the ctx-presence test above cannot catch a regression to evaluate_condition(...,
        // None), because its "Active" leaf is false in both modes without a context.
        let no_flags = HashMap::new();
        for state in ["Missing", "Inactive"] {
            let plugin = plugin_with_patterns(
                PluginType::Optional,
                vec![(file_leaf("x.esp", state), PluginType::Required)],
            );
            assert_eq!(
                evaluate_plugin_type(&plugin, &no_flags, None),
                PluginType::Optional,
                "state {state:?}"
            );
            let ctx = empty_ctx();
            assert_eq!(
                evaluate_plugin_type(&plugin, &no_flags, Some(&ctx)),
                PluginType::Required,
                "state {state:?}"
            );
        }
    }

    #[test]
    fn external_condition_override_repr_matches_cpp() {
        assert_eq!(ExternalConditionOverride::Unknown as u8, 0);
        assert_eq!(ExternalConditionOverride::ForceFalse as u8, 1);
        assert_eq!(ExternalConditionOverride::ForceTrue as u8, 2);
        assert_eq!(
            ExternalConditionOverride::default(),
            ExternalConditionOverride::Unknown
        );
    }

    #[test]
    fn filesystem_probes_swallow_errors_as_not_exists() {
        let no_flags = HashMap::new();
        // invalid path characters on windows: the probe must return false, never panic.
        let ctx = FomodDependencyContext {
            archive_root: "Z:/definitely/not/a/real\u{0}/root".to_string(),
            game_path: "??invalid<>path".to_string(),
            ..empty_ctx()
        };
        assert!(!evaluate_condition(
            &file_leaf("some/file.txt", "Active"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(!evaluate_condition(
            &plugin_leaf("Some.esp", "Inactive"),
            &no_flags,
            Some(&ctx)
        ));
    }

    #[test]
    fn nested_or_inside_and_evaluates_with_short_circuit() {
        let state = flags(&[("a", "1")]);
        let tree = composite(
            FomodConditionOp::And,
            vec![
                composite(
                    FomodConditionOp::Or,
                    vec![flag_leaf("a", "2"), flag_leaf("a", "1")],
                ),
                version_leaf(FomodConditionType::Fose, ""),
            ],
        );
        assert!(evaluate_condition(&tree, &state, None));
        let tree2 = composite(
            FomodConditionOp::And,
            vec![
                composite(
                    FomodConditionOp::Or,
                    vec![flag_leaf("a", "2"), flag_leaf("a", "3")],
                ),
                version_leaf(FomodConditionType::Fose, ""),
            ],
        );
        assert!(!evaluate_condition(&tree2, &state, None));
    }

    #[test]
    fn temp_dir_helper_cleans_up() {
        let mut seen: Option<PathBuf> = None;
        with_temp_dir(|dir| {
            seen = Some(dir.to_path_buf());
            assert!(dir.is_dir());
        });
        assert!(!seen.expect("dir path captured").exists());
    }
}
