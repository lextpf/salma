//! FOMOD dependency evaluation - Rust port of `src/FomodDependencyEvaluator.hpp`
//! / `.cpp`.
//!
//! Free functions that evaluate pre-compiled [`FomodCondition`] IR trees; the
//! single source of truth for FOMOD dependency semantics. Callers (mirroring
//! the C++ call sites):
//!
//! - `FomodService` (forward installation, Task 14)
//! - `FomodForwardSimulator` (offline simulation, Task 6)
//! - `FomodCSPSolver` (constraint solving, Tasks 8-9)
//!
//! The C++ `LeafEvaluator<Mode>` compile-time template dispatch becomes two
//! plain leaf functions selected by the public entry points; the observable
//! behavior is identical. C++ log_warning call sites (depth exceeded, unknown
//! file-dependency state, malformed version component) emit nothing until the
//! Task 17 logger lands; each is marked with a comment.

use std::collections::HashMap;
use std::path::Path;

use crate::fomod_ir::{FomodCondition, FomodConditionOp, FomodConditionType, FomodPlugin};
use crate::types::{FomodDependencyContext, PluginType};
use crate::utils::{normalize_path, to_lower};

/// Maximum depth for recursive condition evaluation (guards against malformed
/// XML). Mirror of `MAX_DEPENDENCY_DEPTH` in `src/FomodDependencyEvaluator.hpp`.
/// Shared with [`crate::fomod_ir_parser`]'s condition compiler, exactly as the
/// C++ parser includes the evaluator header for it.
pub const MAX_DEPENDENCY_DEPTH: i32 = 32;

/// External dependency override mode used during inference. Mirror of
/// `mo2core::ExternalConditionOverride` (`uint8_t` in C++).
///
/// The two `Force*` modes pin the answer regardless of any actual filesystem
/// or game state. `Unknown` is the default for inference runs that do not
/// have access to a [`FomodDependencyContext`]; the solver still has to
/// decide each branch, so the enum picks a conservative answer per category:
///
/// - **File / Plugin / Fomod** -> `false`. These reference user-installed
///   content that may or may not be present; defaulting to `false` keeps the
///   solver from speculatively activating optional files that depend on
///   packages the user might not have.
/// - **Game / FOMM / FOSE** -> `true`. These reference engine /
///   script-extender / loader version checks. An installed mod almost always
///   satisfies them on the machine it was installed on, so defaulting to
///   `true` matches the common real-world case during inference and avoids
///   spurious gating.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ExternalConditionOverride {
    /// External state cannot be determined; see enum-level doc for the
    /// per-category defaults applied.
    #[default]
    Unknown = 0,
    /// Override forces external dependency to evaluate as unmet (false).
    ForceFalse = 1,
    /// Override forces external dependency to evaluate as met (true).
    ForceTrue = 2,
}

// ---------------------------------------------------------------------------
// Version helpers (private, like the C++ file-scoped statics).
// ---------------------------------------------------------------------------

/// Lex-order comparison of two integer version vectors with shorter-pads-zero
/// semantics: "1.2" compares EQUAL to "1.2.0", and "1.2" compares LESS than
/// "1.2.1". Mirror of the C++ `compare_version_parts`.
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

/// Parse a FOMOD version string into an integer vector. Mirror of the C++
/// `parse_version_parts`: strip to ASCII digits and '.', tokenize with C++
/// `std::getline`-on-'.' semantics, `std::stoi` failures (empty token from
/// consecutive/leading dots, or i32 overflow) become 0, tail-pad to length 3.
///
/// The getline quirk, replicated exactly: a TRAILING '.' produces NO trailing
/// empty token ("1.2." -> [1, 2]), but consecutive dots and a leading dot DO
/// produce empty tokens ("1..2" -> [1, 0, 2], ".1" -> [0, 1]). A fully empty
/// cleaned string yields no tokens ([0, 0, 0] after padding).
fn parse_version_parts(version_string: &str) -> Vec<i32> {
    // Remove non-numeric/non-dot characters. C++ uses isdigit under the "C"
    // locale, which is ASCII-only; is_ascii_digit matches.
    let cleaned: String = version_string
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    let mut parts: Vec<i32> = Vec::new();
    if !cleaned.is_empty() {
        let mut tokens: Vec<&str> = cleaned.split('.').collect();
        if cleaned.ends_with('.') {
            // getline never yields the empty token after a trailing delimiter
            // (the next read hits EOF before extracting anything).
            tokens.pop();
        }
        for token in tokens {
            // std::stoi throws on empty tokens (invalid_argument) and on i32
            // overflow (out_of_range); C++ catches both, logs a warning (no
            // logging until Task 17), and pushes 0.
            parts.push(token.parse::<i32>().unwrap_or(0));
        }
    }
    while parts.len() < 3 {
        parts.push(0);
    }
    parts
}

// ---------------------------------------------------------------------------
// C++ std::filesystem::path semantics helpers.
//
// Rust's std::path::Path has different extension/filename edge rules (no
// leading dot in extension(), ".gitignore"/"file." handled differently), so
// these tiny helpers mirror the MSVC fs::path behavior the C++ evaluator
// observes. Both '/' and '\\' are separators, as on Windows.
// ---------------------------------------------------------------------------

/// Filename component per C++ `fs::path::filename()`: everything after the
/// last '/' or '\\' separator; empty when the path ends with a separator.
/// With no separator, MSVC still decomposes a leading drive root-name away:
/// `fs::path("C:foo.esp").filename()` is "foo.esp" (root-name "C:" excluded)
/// and `fs::path("C:").filename()` is "". The root-name only exists at the
/// START of the path ("dir/C:bar" has filename "C:bar"), so the strip applies
/// only in the no-separator branch. MSVC additionally parses a UNC root-name:
/// EXACTLY two leading separators followed by a non-separator extend the
/// root-name to the next separator, so when no further separator follows,
/// the whole path is the root-name and filename() is "" ("//server.esp",
/// "\\\\server", "\\\\?"). With a further separator the generic
/// after-the-last-separator rule already agrees with MSVC
/// ("//server/share.esp" -> "share.esp", "\\\\?\\foo" -> "foo"), and three-or
/// -more leading separators form no root-name ("///foo" -> "foo"). All cases
/// verified against MSVC 2022; see PARITY-NOTES.md.
fn cpp_path_filename(path: &str) -> &str {
    let bytes = path.as_bytes();
    let is_sep = |b: u8| b == b'/' || b == b'\\';
    // MSVC UNC root-name rule ("\\server"): when nothing after the two
    // leading separators contains another separator, the entire path is the
    // root-name and the filename is empty. Byte-wise scan is safe: '/' and
    // '\\' are ASCII and never occur inside a UTF-8 continuation sequence.
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

/// Extension per C++ `fs::path::extension()`: INCLUDES the leading dot
/// (".esp"). A filename that is "." or "..", has no dot, or starts with its
/// only dot (".gitignore") has NO extension; "file." has extension ".".
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

// ---------------------------------------------------------------------------
// Shared helpers: core evaluation logic for dependency types (private, like
// the C++ anonymous namespace).
// ---------------------------------------------------------------------------

/// Non-throwing existence probe. The C++ `safe_exists` wraps `fs::exists`
/// with an error_code so I/O failures yield "does not exist";
/// `Path::exists()` has identical semantics (any error -> false).
fn safe_exists(p: &Path) -> bool {
    p.exists()
}

/// True when the lowercased C++-style extension names a game plugin file.
fn is_plugin_extension(file_path: &str) -> bool {
    let ext_lower = to_lower(cpp_path_extension(file_path));
    matches!(ext_lower.as_str(), ".esp" | ".esm" | ".esl")
}

/// Mirror of the C++ `eval_file_dep` (Normal mode only).
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
        // Extension/filename checks run on the ORIGINAL file_path, exactly as
        // the C++ constructs fs::path(file_path) fresh.
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
        // "Inactive" semantics: for plugin files (.esp/.esm/.esl), check
        // whether the file exists but is NOT in the active plugin list.
        // For non-plugin files, FOMOD has no standard "Inactive" meaning,
        // so conservatively return !file_exists (treat as "Missing").
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

    // "Active" (default). C++ logs a warning for any other state string and
    // treats it as Active; no logging until Task 17.
    file_exists
}

/// Mirror of the C++ `eval_game_dep`.
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

/// Mirror of the C++ `eval_plugin_dep`. The C++ returns a `PluginDepResult`
/// struct whose `is_active`/`file_exists` members no caller reads; only the
/// `met` flag is returned here.
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
                // The join uses the RAW plugin_name, not the lowered one.
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

/// Mirror of the C++ `eval_fomod_dep`. Case-sensitive exact match -
/// deliberately asymmetric with plugin names, which are lowercased.
fn eval_fomod_dep(fomod_name: &str, ctx: Option<&FomodDependencyContext>) -> bool {
    if fomod_name.is_empty() {
        return false;
    }
    ctx.is_some_and(|c| c.installed_fomods.contains(fomod_name))
}

/// FOMM version comparison (MO2 hardcodes "0.13.21"). Mirror of the C++
/// `LeafEvaluator::eval_fomm_version`: required <= actual.
fn eval_fomm_version(version: &str) -> bool {
    if version.is_empty() {
        return true;
    }
    let actual = parse_version_parts("0.13.21");
    let required = parse_version_parts(version);
    compare_version_parts(&required, &actual) <= 0
}

// ---------------------------------------------------------------------------
// Leaf dispatch: the C++ LeafEvaluator<Mode> template as two functions.
// ---------------------------------------------------------------------------

/// Normal-mode leaf dispatch (`LeafEvaluator<EvalMode::Normal>`).
fn eval_leaf_normal(c: &FomodCondition, ctx: Option<&FomodDependencyContext>) -> bool {
    match c.r#type {
        FomodConditionType::File => eval_file_dep(&c.file_path, &c.file_state, ctx),
        FomodConditionType::Game => eval_game_dep(&c.version, ctx),
        FomodConditionType::Plugin => eval_plugin_dep(&c.plugin_name, &c.plugin_type, ctx),
        FomodConditionType::Fomod => eval_fomod_dep(&c.fomod_name, ctx),
        FomodConditionType::Fomm => eval_fomm_version(&c.version),
        FomodConditionType::Fose => true,
        // Flag and Composite never reach the leaf dispatch (handled in
        // evaluate_condition_core); the C++ default switch arm returns true.
        FomodConditionType::Flag | FomodConditionType::Composite => true,
    }
}

/// Inferred-mode leaf dispatch (`LeafEvaluator<EvalMode::Inferred>`): the
/// external categories (File/Plugin/Fomod) follow the override - both
/// `Unknown` and `ForceFalse` yield false - and everything else (Game, Fomm,
/// Fose, plus the unreachable Flag/Composite default arm) is true. The C++
/// evaluator's ctx member is never read in this mode.
fn eval_leaf_inferred(c: &FomodCondition, external_override: ExternalConditionOverride) -> bool {
    match c.r#type {
        FomodConditionType::File | FomodConditionType::Plugin | FomodConditionType::Fomod => {
            external_override == ExternalConditionOverride::ForceTrue
        }
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// evaluate_condition_core: shared Composite/Flag handling, delegates leaf
// types to the supplied strategy. Mirror of the C++ template function.
// ---------------------------------------------------------------------------

fn evaluate_condition_core(
    condition: &FomodCondition,
    flags: &HashMap<String, String>,
    eval_leaf: &dyn Fn(&FomodCondition) -> bool,
    depth: i32,
) -> bool {
    match condition.r#type {
        FomodConditionType::Composite => {
            if depth > MAX_DEPENDENCY_DEPTH {
                // C++ logs "[fomod-ir] Condition tree exceeds maximum depth,
                // treating as unmet"; no logging until Task 17.
                return false;
            }
            let is_and = condition.op == FomodConditionOp::And;
            // And starts true (empty And -> true), Or starts false (empty Or
            // -> false). This is how the parser's depth-truncation bail
            // (empty Or) becomes always-false and pattern-without-
            // dependencies (empty And) becomes always-true.
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
        // Flags are handled here, BEFORE leaf dispatch, so they evaluate
        // identically in Normal and Inferred modes. Missing flag -> true iff
        // the expected value is empty; present -> exact case-sensitive
        // equality, no trimming.
        FomodConditionType::Flag => match flags.get(&condition.flag_name) {
            None => condition.flag_value.is_empty(),
            Some(actual) => *actual == condition.flag_value,
        },
        _ => eval_leaf(condition),
    }
}

// ---------------------------------------------------------------------------
// Public entry points, mirroring the three MO2_API free functions.
// ---------------------------------------------------------------------------

/// Evaluate a [`FomodCondition`] IR node against a flag map and optional
/// context. Mirror of `mo2core::evaluate_condition`.
///
/// Never panics for well-formed IR; filesystem errors during file-dependency
/// checks are swallowed (probe answers "does not exist"), matching the C++
/// non-throwing `std::error_code` overloads.
pub fn evaluate_condition(
    condition: &FomodCondition,
    flags: &HashMap<String, String>,
    context: Option<&FomodDependencyContext>,
) -> bool {
    evaluate_condition_core(condition, flags, &|c| eval_leaf_normal(c, context), 0)
}

/// Evaluate a condition for inference: flag conditions evaluated normally,
/// all external conditions (File, Plugin, Fomod) follow the override mode.
/// Mirror of `mo2core::evaluate_condition_inferred`.
///
/// - `ForceTrue`: external conditions evaluate to true
/// - `ForceFalse`: external conditions evaluate to false
/// - `Unknown`: external conditions (File, Plugin, Fomod) conservatively
///   return false; infrastructure conditions (Game, Fomm, Fose) return true
///
/// `_context` is accepted for signature parity with the C++ function but is
/// never read in inferred mode (the C++ `LeafEvaluator<Inferred>` stores the
/// ctx member and never touches it).
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

/// Determine a plugin's effective type given the current flag state. Mirror
/// of `mo2core::evaluate_plugin_type`: checks `type_patterns` in order, first
/// match wins, falls back to the plugin's declared type.
///
/// The Normal-vs-Inferred asymmetry on context presence is deliberate and
/// mirrors the C++ exactly: with a context the patterns are evaluated in
/// Normal mode; without one they are evaluated in Inferred mode with the
/// `Unknown` override (external leaves -> false). The CSP solver always
/// passes no context and thus always gets inferred-Unknown semantics.
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

    // --- helpers -----------------------------------------------------------

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

    /// Composite chain: `levels` nested composites; the INNERMOST composite
    /// sits at evaluation depth `levels - 1` (the root is evaluated at 0) and
    /// carries one always-true Flag leaf.
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

    /// Temp dir scoped helper: creates a unique dir under std temp, runs the
    /// closure, removes the dir.
    fn with_temp_dir(f: impl FnOnce(&Path)) {
        let dir = std::env::temp_dir().join(format!(
            "salma_rs_dep_eval_{}",
            random_hex_string(RANDOM_HEX_DEFAULT_LEN)
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        f(&dir);
        fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    // --- trap (a): composite depth guard + And/Or semantics ----------------

    #[test]
    fn empty_and_is_true_empty_or_is_false() {
        let no_flags = HashMap::new();
        let empty_and = composite(FomodConditionOp::And, vec![]);
        let empty_or = composite(FomodConditionOp::Or, vec![]);
        assert!(evaluate_condition(&empty_and, &no_flags, None));
        assert!(!evaluate_condition(&empty_or, &no_flags, None));
        // Same in inferred mode: Composite handling is shared.
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
        // And: [false, true] -> false; Or: [true, false] -> true.
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
        // 33 nested composites: innermost at depth 32 == MAX, guard is
        // `depth > MAX` so it still evaluates (to true via the flag leaf).
        assert!(evaluate_condition(&nested_chain(33), &no_flags, None));
        // 34 nested composites: innermost at depth 33 > MAX -> that node is
        // false, collapsing the whole And chain to false.
        assert!(!evaluate_condition(&nested_chain(34), &no_flags, None));
    }

    // --- trap (b): flag semantics, shared across modes ---------------------

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

    // --- trap (c): file dependency states ----------------------------------

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
        // Original path is un-normalized; the lookup normalizes it.
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
        // Not in installed_files, but .esp filename is in installed_plugins.
        assert!(evaluate_condition(
            &file_leaf("Data/SkyUI.esp", "Active"),
            &no_flags,
            Some(&ctx)
        ));
        // Non-plugin extension gets no fallback.
        let ctx2 = FomodDependencyContext {
            installed_plugins: set(&["skyui.txt"]),
            ..empty_ctx()
        };
        assert!(!evaluate_condition(
            &file_leaf("Data/SkyUI.txt", "Active"),
            &no_flags,
            Some(&ctx2)
        ));
        // .esm and .esl take the fallback too.
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
        // Exists (installed_files) but not active (installed_plugins) ->
        // Inactive is true.
        let ctx = FomodDependencyContext {
            installed_files: set(&["mod.esp"]),
            ..empty_ctx()
        };
        assert!(evaluate_condition(
            &file_leaf("Mod.esp", "Inactive"),
            &no_flags,
            Some(&ctx)
        ));
        // Exists AND active -> Inactive is false.
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
        // Exists, non-plugin extension -> !file_exists -> false.
        assert!(!evaluate_condition(
            &file_leaf("readme.txt", "Inactive"),
            &no_flags,
            Some(&ctx)
        ));
        // Absent, non-plugin -> true.
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
        // C++ logs a warning and falls through to the Active branch.
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
        // Lowercase state literals do not match ("missing" != "Missing").
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
            // Hit via archive_root (normalized relative path join).
            assert!(evaluate_condition(
                &file_leaf("Sub\\In_Archive.txt", "Active"),
                &no_flags,
                Some(&ctx)
            ));
            // Hit via game_path only.
            assert!(evaluate_condition(
                &file_leaf("sub/in_game.txt", "Active"),
                &no_flags,
                Some(&ctx)
            ));
            // Miss everywhere.
            assert!(!evaluate_condition(
                &file_leaf("sub/nowhere.txt", "Active"),
                &no_flags,
                Some(&ctx)
            ));
            // Empty archive_root/game_path disable the respective probe.
            let ctx_no_probe = FomodDependencyContext { ..empty_ctx() };
            assert!(!evaluate_condition(
                &file_leaf("sub/in_archive.txt", "Active"),
                &no_flags,
                Some(&ctx_no_probe)
            ));
        });
    }

    // --- trap (d): C++ path extension/filename semantics -------------------

    #[test]
    fn cpp_path_helpers_mirror_msvc_fs_path() {
        // extension() INCLUDES the dot.
        assert_eq!(cpp_path_extension("mod.esp"), ".esp");
        assert_eq!(cpp_path_extension("dir/mod.esp"), ".esp");
        assert_eq!(cpp_path_extension("dir\\mod.esp"), ".esp");
        // No dot -> no extension.
        assert_eq!(cpp_path_extension("mod"), "");
        // Leading dot with no other dot -> NO extension (".gitignore").
        assert_eq!(cpp_path_extension(".gitignore"), "");
        assert_eq!(cpp_path_extension("dir/.gitignore"), "");
        // Leading dot plus another dot -> extension from the last dot.
        assert_eq!(cpp_path_extension(".profile.txt"), ".txt");
        // Trailing dot -> extension ".".
        assert_eq!(cpp_path_extension("file."), ".");
        // "." and ".." have no extension.
        assert_eq!(cpp_path_extension("."), "");
        assert_eq!(cpp_path_extension(".."), "");
        assert_eq!(cpp_path_extension("dir/.."), "");
        // Dot in a directory component does not count.
        assert_eq!(cpp_path_extension("dir.d/file"), "");

        // filename(): after the last separator of either kind.
        assert_eq!(cpp_path_filename("a/b/c.esp"), "c.esp");
        assert_eq!(cpp_path_filename("a\\b\\c.esp"), "c.esp");
        assert_eq!(cpp_path_filename("a/b\\c.esp"), "c.esp");
        assert_eq!(cpp_path_filename("c.esp"), "c.esp");
        assert_eq!(cpp_path_filename("a/"), "");

        // Drive root-name decomposition without a separator: MSVC excludes
        // the root-name ("C:") from filename().
        assert_eq!(cpp_path_filename("C:foo.esp"), "foo.esp");
        assert_eq!(cpp_path_filename("c:foo.esp"), "foo.esp");
        assert_eq!(cpp_path_filename("C:"), "");
        assert_eq!(cpp_path_filename("C:/foo.esp"), "foo.esp");
        // Not a drive prefix: two letters, or a non-letter, before ':'.
        assert_eq!(cpp_path_filename("CC:foo"), "CC:foo");
        assert_eq!(cpp_path_filename("1:foo"), "1:foo");
        assert_eq!(cpp_path_filename(":foo"), ":foo");
        // Root-names only exist at the start of the path; after a separator
        // the colon component is kept whole.
        assert_eq!(cpp_path_filename("dir/C:bar"), "C:bar");
        // extension() through the fixed filename(): "C:.esp" has filename
        // ".esp" whose only dot is leading -> NO extension (MSVC agrees).
        assert_eq!(cpp_path_extension("C:foo.esp"), ".esp");
        assert_eq!(cpp_path_extension("C:.esp"), "");
        assert_eq!(cpp_path_extension("C:.gitignore"), "");

        // UNC root-name decomposition: exactly two leading separators plus a
        // non-separator start a root-name that runs to the next separator.
        // With no further separator the WHOLE path is the root-name, so
        // filename() and extension() are empty even for a plugin-shaped tail.
        // Every expectation here was verified against MSVC 2022 fs::path.
        assert_eq!(cpp_path_filename("//server.esp"), "");
        assert_eq!(cpp_path_extension("//server.esp"), "");
        assert_eq!(cpp_path_filename("//server"), "");
        assert_eq!(cpp_path_filename("\\\\server"), "");
        assert_eq!(cpp_path_filename("//C:"), "");
        assert_eq!(cpp_path_filename("\\\\?"), "");
        // A separator after the root-name resumes the generic rule.
        assert_eq!(cpp_path_filename("//server/share.esp"), "share.esp");
        assert_eq!(cpp_path_extension("//server/share.esp"), ".esp");
        assert_eq!(cpp_path_filename("//server.esp/x.esp"), "x.esp");
        assert_eq!(cpp_path_filename("//server/"), "");
        assert_eq!(cpp_path_filename("\\\\?\\foo"), "foo");
        assert_eq!(cpp_path_filename("\\??\\foo"), "foo");
        // Three or more leading separators form NO root-name, and "\\??" is
        // not a device prefix without its trailing separator.
        assert_eq!(cpp_path_filename("///foo"), "foo");
        assert_eq!(cpp_path_filename("//"), "");
        assert_eq!(cpp_path_filename("\\??"), "??");
    }

    /// A UNC-root-name-only path with a plugin-shaped tail must NOT take the
    /// installed_plugins fallback: MSVC sees filename "" / extension "", so
    /// the C++ evaluator treats "//server.esp" as a non-plugin file that does
    /// not exist ("Active" -> false even when server.esp is an active plugin,
    /// "Inactive" -> !exists -> true).
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
        // Control: the same tail below a UNC share IS a plugin filename.
        assert!(evaluate_condition(
            &file_leaf("//host/server.esp", "Active"),
            &no_flags,
            Some(&ctx)
        ));
    }

    // --- trap (e): game dependency -----------------------------------------

    #[test]
    fn game_dep_standalone_mode_is_true() {
        let no_flags = HashMap::new();
        let leaf = version_leaf(FomodConditionType::Game, "1.5.97");
        // No ctx.
        assert!(evaluate_condition(&leaf, &no_flags, None));
        // Ctx with empty game_path.
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
        // game_version >= required -> true.
        assert!(evaluate_condition(&leaf, &no_flags, Some(&ctx("1.5.97"))));
        assert!(evaluate_condition(&leaf, &no_flags, Some(&ctx("1.6.1170"))));
        // game_version < required -> false.
        assert!(!evaluate_condition(&leaf, &no_flags, Some(&ctx("1.5.80"))));
        // Empty required version or empty game_version -> true.
        let empty_req = version_leaf(FomodConditionType::Game, "");
        assert!(evaluate_condition(&empty_req, &no_flags, Some(&ctx("1.0"))));
        assert!(evaluate_condition(&leaf, &no_flags, Some(&ctx(""))));
    }

    // --- trap (f): plugin dependency ----------------------------------------

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
        // Any type string other than "Inactive" behaves as Active,
        // including garbage and lowercase "inactive".
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
        // Active plugin fails the Inactive check.
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
            // Exists in Data/, not active -> Inactive true.
            assert!(evaluate_condition(
                &plugin_leaf("OnDisk.esp", "Inactive"),
                &no_flags,
                Some(&ctx)
            ));
            // Exists in Data/, not active -> Active false.
            assert!(!evaluate_condition(
                &plugin_leaf("OnDisk.esp", "Active"),
                &no_flags,
                Some(&ctx)
            ));
            // Absent everywhere -> Inactive false.
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

    // --- trap (g): fomod dependency -----------------------------------------

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
        // Asymmetric with plugins: NO lowercasing.
        assert!(!evaluate_condition(
            &fomod_leaf("skyui"),
            &no_flags,
            Some(&ctx)
        ));
        assert!(!evaluate_condition(&fomod_leaf(""), &no_flags, Some(&ctx)));
        assert!(!evaluate_condition(&fomod_leaf("SkyUI"), &no_flags, None));
    }

    // --- trap (h): fomm version ---------------------------------------------

    #[test]
    fn fomm_dep_boundary_at_hardcoded_version() {
        let no_flags = HashMap::new();
        let fomm = |v: &str| version_leaf(FomodConditionType::Fomm, v);
        // Exactly the hardcoded actual -> true (required <= actual).
        assert!(evaluate_condition(&fomm("0.13.21"), &no_flags, None));
        // Below -> true.
        assert!(evaluate_condition(&fomm("0.13.20"), &no_flags, None));
        assert!(evaluate_condition(&fomm("0.12"), &no_flags, None));
        // Above -> false.
        assert!(!evaluate_condition(&fomm("0.13.22"), &no_flags, None));
        assert!(!evaluate_condition(&fomm("1.0"), &no_flags, None));
        // Empty required -> true.
        assert!(evaluate_condition(&fomm(""), &no_flags, None));
        // Pad-zero equality: "0.13.21.0" == "0.13.21".
        assert!(evaluate_condition(&fomm("0.13.21.0"), &no_flags, None));
        assert!(!evaluate_condition(&fomm("0.13.21.1"), &no_flags, None));
    }

    // --- trap (i): fose always true -----------------------------------------

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

    // --- trap (j): inferred-mode override matrix -----------------------------

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
        // A context that would make the File leaf true in Normal mode has no
        // effect in Inferred mode (the C++ evaluator never reads ctx there).
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

    // --- traps (k), (l): version parsing and comparison ---------------------

    #[test]
    fn parse_version_parts_table() {
        let cases: &[(&str, &[i32])] = &[
            ("1.2.3", &[1, 2, 3]),
            ("v1.2.3-beta", &[1, 2, 3]),
            ("1.2.3b", &[1, 2, 3]),
            ("1.2.3.99 (custom)", &[1, 2, 3, 99]),
            // Consecutive dots: empty token -> 0.
            ("1..2", &[1, 0, 2]),
            // Leading dot: empty first token -> 0.
            (".1", &[0, 1, 0]),
            // Trailing dot: getline yields NO trailing empty token.
            ("1.2.", &[1, 2, 0]),
            ("1.", &[1, 0, 0]),
            // Fully empty and non-numeric inputs.
            ("", &[0, 0, 0]),
            ("beta", &[0, 0, 0]),
            // Only dots: "." -> [""], ".." -> ["", ""] after the trailing pop.
            (".", &[0, 0, 0]),
            ("..", &[0, 0, 0]),
            // i32 overflow token -> 0.
            ("99999999999999999999", &[0, 0, 0]),
            ("1.99999999999999999999.2", &[1, 0, 2]),
            // Pad to 3.
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
        // Longer-than-3 vectors still compare element-wise.
        assert_eq!(compare_version_parts(&[1, 2, 3, 4], &[1, 2, 3]), 1);
        assert_eq!(compare_version_parts(&[1, 2, 3], &[1, 2, 3, 0]), 0);
    }

    // --- trap (m): evaluate_plugin_type --------------------------------------

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
        // Pattern with a File condition that the ctx satisfies but that
        // inferred-Unknown evaluates to false.
        let no_flags = HashMap::new();
        let plugin = plugin_with_patterns(
            PluginType::Optional,
            vec![(file_leaf("marker.esp", "Active"), PluginType::Required)],
        );
        let ctx = FomodDependencyContext {
            installed_files: set(&["marker.esp"]),
            ..empty_ctx()
        };
        // Some(ctx): Normal mode, File leaf true -> pattern matches.
        assert_eq!(
            evaluate_plugin_type(&plugin, &no_flags, Some(&ctx)),
            PluginType::Required
        );
        // None: Inferred/Unknown, File leaf false -> fallback type.
        assert_eq!(
            evaluate_plugin_type(&plugin, &no_flags, None),
            PluginType::Optional
        );
    }

    #[test]
    fn plugin_type_none_ctx_dispatches_inferred_not_normal_with_null_ctx() {
        // Distinguisher: a File leaf with state "Missing" or "Inactive" is
        // TRUE in Normal mode without a context (file_exists = false), but
        // FALSE in Inferred/Unknown mode (File leaves follow the override
        // regardless of state). Only the inferred dispatch matches the C++
        // None branch (FomodDependencyEvaluator.cpp:399-408); this is the hot
        // inference path, since the CSP solver always passes no context. The
        // ctx-presence test above cannot catch a regression to
        // evaluate_condition(..., None) because its "Active" leaf is false in
        // BOTH modes without a context.
        let no_flags = HashMap::new();
        for state in ["Missing", "Inactive"] {
            let plugin = plugin_with_patterns(
                PluginType::Optional,
                vec![(file_leaf("x.esp", state), PluginType::Required)],
            );
            // None: inferred-Unknown -> File leaf false -> fallback type.
            assert_eq!(
                evaluate_plugin_type(&plugin, &no_flags, None),
                PluginType::Optional,
                "state {state:?}"
            );
            // Some(empty ctx): Normal mode -> file does not exist -> leaf
            // true -> pattern type.
            let ctx = empty_ctx();
            assert_eq!(
                evaluate_plugin_type(&plugin, &no_flags, Some(&ctx)),
                PluginType::Required,
                "state {state:?}"
            );
        }
    }

    // --- trap (n): enum representation ---------------------------------------

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

    // --- trap (o): non-throwing filesystem probes ----------------------------

    #[test]
    fn filesystem_probes_swallow_errors_as_not_exists() {
        let no_flags = HashMap::new();
        // Invalid path characters on Windows: the probe must return false,
        // never panic (mirrors the fs::exists error_code overload).
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

    // --- integration: composite trees over mixed leaves ----------------------

    #[test]
    fn nested_or_inside_and_evaluates_with_short_circuit() {
        let state = flags(&[("a", "1")]);
        // And( Or(flag a==2 [false], flag a==1 [true]) [true],
        //      Fose [true] ) -> true
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
        // Same tree with the Or unsatisfiable -> false.
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
