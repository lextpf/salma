//! FOMOD dependency evaluation: the single source of truth for what a
//! `<dependencies>` tree means.
//!
//! Free functions that evaluate pre-compiled [`FomodCondition`] IR trees. Every
//! stage that decides what a FOMOD installs answers its questions here, so
//! changing the semantics in this module changes all of them at once:
//!
//! - [`crate::fomod_service`] - the real install replay
//! - [`crate::fomod_forward_simulator`] - the offline simulation the solver
//!   scores candidates against
//! - [`crate::fomod_propagator`] - the deterministic fixpoint pre-pass
//! - [`crate::fomod_csp_solver`] and [`crate::fomod_csp_options`] - constraint
//!   solving and per-group option enumeration
//!
//! Sharing one implementation is the point: a change moves the install replay
//! and the simulator it is scored against together, so the two cannot drift.
//!
//! [`crate::fomod_ir_parser`] imports [`MAX_DEPENDENCY_DEPTH`] from here but
//! calls no evaluation function.
//!
//! Two evaluation modes share one tree walk. Normal mode answers external
//! conditions from a [`FomodDependencyContext`] and the filesystem. Inferred
//! mode answers them from an [`ExternalConditionOverride`] and touches neither.
//! Flags and composites evaluate identically in both.
//!
//! Three warnings leave this module and their subsystem tags are not uniform:
//! the depth warning is tagged `[fomod-ir]`, the malformed-version warning
//! `[fomod]`, and the unknown-file-state warning carries no tag at all. The
//! spellings are deliberate; read PARITY-NOTES.md before normalizing them.

use std::collections::HashMap;
use std::path::Path;

use crate::fomod_ir::{FomodCondition, FomodConditionOp, FomodConditionType, FomodPlugin};
use crate::logger::Logger;
use crate::types::{FomodDependencyContext, PluginType};
use crate::utils::{normalize_path, to_lower};

/// Maximum nesting depth for recursive condition evaluation, a guard against
/// malformed XML. Shared with [`crate::fomod_ir_parser`]'s condition compiler.
///
/// Both users count from depth 0 at the outermost `<dependencies>`, and both
/// degrade rather than fail when the bound is passed. The parser replaces the
/// over-deep subtree with an empty `Or`, which is always false. The evaluator
/// answers false for the over-deep subtree. Neither reports an error, so an
/// over-deep condition reads as unmet, never as invalid.
pub const MAX_DEPENDENCY_DEPTH: i32 = 32;

/// How inferred mode answers an external dependency.
///
/// Consulted only in inferred mode; no variant reads the filesystem or the game
/// state, because inferred mode never probes either. `Unknown` is the default
/// for an inference run with no [`FomodDependencyContext`].
///
/// Inferred mode splits the condition types into two categories, and the split
/// applies under every override, not only under `Unknown`:
///
/// - **File, Plugin, Fomod** are external and follow the override. They
///   reference user-installed content that may or may not be present, so
///   answering false unless forced keeps the solver from speculatively
///   activating optional files that depend on packages the user might not have.
/// - **Game, Fomm, Fose** are infrastructure and are true under every override.
///   They reference engine, script-extender and loader version checks, which an
///   installed mod almost always satisfies on the machine it was installed on,
///   so true matches the common case and avoids spurious gating.
///
/// Three overrides, two distinct results: `Unknown` and `ForceFalse` are
/// indistinguishable for every condition type.
///
/// ```text
/// inferred-mode leaf result by condition type and override:
///
///               File   Plugin  Fomod | Game  Fomm  Fose
///   Unknown     false  false   false | true  true  true
///   ForceFalse  false  false   false | true  true  true
///   ForceTrue   true   true    true  | true  true  true
///
/// Flag and Composite never reach leaf dispatch; they are handled earlier.
/// ```
///
/// The two variants stay separate because diagnostics have to tell "not
/// determined" from "determined absent": [`crate::fomod_inference_service`]
/// maps each override onto its own step-visibility reason code. Its
/// `compute_overrides` emits only `ForceTrue` and `Unknown`, so the `ForceFalse`
/// reason code is structurally unreachable and only the tests exercise it.
/// Collapsing the variants would erase the distinction from the output while
/// changing no result. The matrix is pinned by the test
/// `inferred_mode_override_matrix`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ExternalConditionOverride {
    /// External state cannot be determined. Evaluates exactly as `ForceFalse`;
    /// the separate variant exists so callers and diagnostics can report "not
    /// determined" rather than "determined absent".
    #[default]
    Unknown = 0,
    /// External dependencies evaluate as unmet. Same results as `Unknown`.
    ForceFalse = 1,
    /// External dependencies evaluate as met.
    ForceTrue = 2,
}

/// Compare two integer version vectors element-wise, padding the shorter one
/// with zeros: "1.2" equals "1.2.0" and is less than "1.2.1".
///
/// Returns -1 when `x < y`, 0 when they are equal, 1 when `x > y`. The sign is
/// the whole contract, and the two call sites pass their arguments in opposite
/// orders, so check which vector is `x` before reading a comparison here.
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

/// Parse a FOMOD version string into an integer vector: drop every character
/// that is neither an ASCII digit nor '.', split on '.', parse each token, then
/// tail-pad to length 3.
///
/// A token that will not parse, meaning an empty one or one wider than `i32`,
/// becomes 0 and writes a `[fomod]` warning to `logs/salma.log`.
///
/// A trailing '.' yields no trailing empty token, while a leading dot and
/// consecutive dots do. The trailing-dot case is special-cased on purpose:
/// without it, every version string ending in a dot would log a spurious
/// malformed-component warning. The token column is taken before the tail-pad
/// step, so it is not what the function returns:
///
/// | input    | tokens      | returned    |
/// |----------|-------------|-------------|
/// | `"1.2."` | `[1, 2]`    | `[1, 2, 0]` |
/// | `"1..2"` | `[1, 0, 2]` | `[1, 0, 2]` |
/// | `".1"`   | `[0, 1]`    | `[0, 1, 0]` |
/// | `""`     | none        | `[0, 0, 0]` |
///
/// The return value is always at least 3 elements long, and longer when the
/// input has more than three components.
fn parse_version_parts(version_string: &str) -> Vec<i32> {
    // Keep ASCII digits and dots only. A non-ASCII digit is dropped like any
    // other stray character.
    let cleaned: String = version_string
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    let mut parts: Vec<i32> = Vec::new();
    if !cleaned.is_empty() {
        let mut tokens: Vec<&str> = cleaned.split('.').collect();
        if cleaned.ends_with('.') {
            // A trailing delimiter contributes no token; see the doc comment.
            tokens.pop();
        }
        for token in tokens {
            // Empty token (leading or consecutive dot) and i32 overflow both
            // recover as 0 after a warning. Neither aborts the parse.
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

// ---------------------------------------------------------------------------
// Windows path decomposition.
//
// Plugin detection has to split a path the way MSVC std::filesystem::path does.
// Rust's std::path::Path uses different rules (extension() drops the leading
// dot, and ".gitignore" and "file." decompose differently), so these helpers
// implement the MSVC rules directly. Both '/' and '\' are separators.
// ---------------------------------------------------------------------------

/// Filename component under MSVC `std::filesystem::path` rules: everything after
/// the last '/' or '\' separator, and empty when the path ends with a separator.
///
/// Two root-name rules make the no-separator cases surprising. Both are verified
/// against MSVC 2022; see PARITY-NOTES.md.
///
/// - A leading drive root-name is excluded, so `C:foo.esp` has filename
///   `foo.esp` and `C:` has filename "". Root-names exist only at the start of a
///   path, so `dir/C:bar` has filename `C:bar`, and the strip applies only in
///   the no-separator branch.
/// - Exactly two leading separators followed by a non-separator start a UNC
///   root-name that runs to the next separator. With no further separator the
///   whole path is the root-name and the filename is "": `//server.esp`,
///   `\\server`, `\\?`. With a further separator the generic
///   after-the-last-separator rule already agrees (`//server/share.esp` ->
///   `share.esp`, `\\?\foo` -> `foo`), and three or more leading separators form
///   no root-name at all (`///foo` -> `foo`).
fn cpp_path_filename(path: &str) -> &str {
    let bytes = path.as_bytes();
    let is_sep = |b: u8| b == b'/' || b == b'\\';
    // UNC root-name rule: when nothing after the two leading separators holds
    // another separator, the whole path is the root-name and the filename is
    // empty. The byte-wise scan is safe because '/' and '\' are ASCII and never
    // occur inside a UTF-8 continuation sequence.
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

/// Extension under MSVC `std::filesystem::path` rules, including the leading dot
/// (".esp"). A filename that is "." or "..", that has no dot, or whose only dot
/// is the first character (".gitignore") has no extension; "file." has extension
/// ".".
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

/// Existence probe that never fails: any I/O error answers "does not exist".
fn safe_exists(p: &Path) -> bool {
    p.exists()
}

/// True when the path's extension, lowercased, names a game plugin file.
fn is_plugin_extension(file_path: &str) -> bool {
    let ext_lower = to_lower(cpp_path_extension(file_path));
    matches!(ext_lower.as_str(), ".esp" | ".esm" | ".esl")
}

/// Decide a `<fileDependency>` leaf in normal mode.
///
/// Returns false at once when `file_path` is empty. Otherwise it establishes
/// whether the file exists, then answers according to `state`:
///
/// ```text
/// existence probes, in order, first hit wins:
///   1 ctx.installed_files contains normalize_path(file_path)
///   2 .esp/.esm/.esl only: ctx.installed_plugins contains the lowercased filename
///   3 ctx.archive_root joined with the normalized path exists on disk
///   4 ctx.game_path    joined with the normalized path exists on disk
///   no ctx -> exists = false, no probe runs
///
/// answer by state:
///   state      .esp/.esm/.esl                      any other extension
///   Active     exists                              exists
///   Missing    not exists                          not exists
///   Inactive   not exists, or exists and the       same as Missing:
///              lowercased filename is not in       not exists
///              ctx.installed_plugins
///   any other  warn, then answer as Active         warn, then as Active
/// ```
///
/// The `Inactive` row is surprising in two ways. A missing file answers true for
/// every extension, because the plugin branch is guarded by `file_exists` and
/// the fallthrough answer is `!file_exists`. An existing non-plugin file answers
/// false, because FOMOD gives `Inactive` no meaning outside .esp/.esm/.esl and
/// the code falls back to `Missing` semantics.
///
/// Probes 3 and 4 are blocking filesystem calls, up to two per leaf, and run
/// only when the context sets `archive_root` or `game_path`. Any I/O error
/// answers "does not exist"; nothing is thrown or returned as an error.
///
/// Probes 1 and 2 handle case differently: probe 1 compares normalized paths,
/// which are already lowercase, and probe 2 lowercases the filename itself.
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
        // The extension and filename checks run on the original file_path, not
        // on the normalized one.
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
        // For a plugin file (.esp/.esm/.esl), "Inactive" means the file exists
        // but is not in the active plugin list. FOMOD gives it no meaning for a
        // non-plugin file, so those fall back to !file_exists, that is, to
        // "Missing" semantics.
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

    // "Active" is the default: any other state string warns and is then treated
    // as Active. This warning is the one engine message with no subsystem tag.
    // Leave it untagged; see the module doc.
    if state != "Active" {
        Logger::instance().log_warning(&format!(
            "Unknown file dependency state: {state} for file: {file_path}, treating as Active"
        ));
    }
    file_exists
}

/// Decide a `<gameDependency>` leaf: true unless the context supplies both a
/// `game_path` and a `game_version` and the installed version is lower than the
/// required one.
///
/// No context, an empty `game_path`, an empty required version or an empty
/// `game_version` all answer true. That is standalone mode: with no game to
/// check against, a game version requirement cannot fail the install.
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

/// Decide a `<pluginDependency>` leaf.
///
/// Returns false at once when `plugin_name` is empty. "Active" means the
/// lowercased name is in `ctx.installed_plugins`. When it is not, and the context
/// sets `game_path`, one blocking disk probe tests
/// `<game_path>/Data/<plugin_name>` using the raw name, not the lowercased one.
///
/// `plugin_type == "Inactive"` asks for a file that exists but is not active.
/// Every other value, including `"Active"` and the empty string, asks for an
/// active plugin.
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
                // The join uses the raw plugin_name, not the lowered one.
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

/// Decide a `<fomodDependency>` leaf: a case-sensitive exact match against
/// `ctx.installed_fomods`. An empty name is false.
///
/// The case sensitivity is deliberate and asymmetric with plugin names, which
/// are lowercased before lookup. Do not make the two symmetric: callers are
/// required to store FOMOD names exactly as written, so lowercasing one side
/// here would stop every mixed-case name matching.
fn eval_fomod_dep(fomod_name: &str, ctx: Option<&FomodDependencyContext>) -> bool {
    if fomod_name.is_empty() {
        return false;
    }
    ctx.is_some_and(|c| c.installed_fomods.contains(fomod_name))
}

/// Decide a `<fommDependency>` leaf against the FOMM version MO2 hardcodes,
/// "0.13.21". True when the required version is empty, or is less than or equal
/// to that.
fn eval_fomm_version(version: &str) -> bool {
    if version.is_empty() {
        return true;
    }
    let actual = parse_version_parts("0.13.21");
    let required = parse_version_parts(version);
    compare_version_parts(&required, &actual) <= 0
}

/// Normal-mode leaf dispatch: each type answers from the context and the disk.
fn eval_leaf_normal(c: &FomodCondition, ctx: Option<&FomodDependencyContext>) -> bool {
    match c.r#type {
        FomodConditionType::File => eval_file_dep(&c.file_path, &c.file_state, ctx),
        FomodConditionType::Game => eval_game_dep(&c.version, ctx),
        FomodConditionType::Plugin => eval_plugin_dep(&c.plugin_name, &c.plugin_type, ctx),
        FomodConditionType::Fomod => eval_fomod_dep(&c.fomod_name, ctx),
        FomodConditionType::Fomm => eval_fomm_version(&c.version),
        FomodConditionType::Fose => true,
        // evaluate_condition_core handles Flag and Composite, so neither reaches
        // here. The arm answers true to keep the match exhaustive.
        FomodConditionType::Flag | FomodConditionType::Composite => true,
    }
}

/// Inferred-mode leaf dispatch: the external types (File, Plugin, Fomod) follow
/// the override, so both `Unknown` and `ForceFalse` answer false. Everything
/// else, including the unreachable Flag and Composite arm, answers true. No
/// context is consulted and no filesystem probe runs.
fn eval_leaf_inferred(c: &FomodCondition, external_override: ExternalConditionOverride) -> bool {
    match c.r#type {
        FomodConditionType::File | FomodConditionType::Plugin | FomodConditionType::Fomod => {
            external_override == ExternalConditionOverride::ForceTrue
        }
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// evaluate_condition_core: the shared tree walk. It owns Composite and Flag
// handling, which is why those two evaluate identically in both modes, and
// hands every other type to the supplied leaf function.
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
                // Tagged [fomod-ir], not the [fomod] the rest of this module
                // uses. Keep the spelling; see the module doc.
                Logger::instance().log_warning(
                    "[fomod-ir] Condition tree exceeds maximum depth, treating as unmet",
                );
                return false;
            }
            let is_and = condition.op == FomodConditionOp::And;
            // And seeds true, Or seeds false, so an empty And is true and an
            // empty Or is false. Both empties are load-bearing: the parser's
            // depth-truncation bail emits an empty Or and must read as
            // always-false, and a pattern with no dependencies is an empty And
            // and must read as always-true.
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
        // Flags are answered here, ahead of leaf dispatch, which is why they
        // evaluate identically in both modes. A missing flag is true only when
        // the expected value is empty; a present flag is an exact
        // case-sensitive comparison, with no trimming.
        FomodConditionType::Flag => match flags.get(&condition.flag_name) {
            None => condition.flag_value.is_empty(),
            Some(actual) => *actual == condition.flag_value,
        },
        _ => eval_leaf(condition),
    }
}

/// Evaluate a [`FomodCondition`] tree in normal mode, against a flag map and an
/// optional context.
///
/// Never panics for well-formed IR. Filesystem errors during file-dependency
/// checks are swallowed: the probe answers "does not exist".
///
/// **Depth.** Recursion starts at depth 0. A Composite subtree nested deeper
/// than [`MAX_DEPENDENCY_DEPTH`] evaluates as unmet and writes a `[fomod-ir]`
/// warning to `logs/salma.log`. There is no error return, so an over-deep tree
/// silently reads as unsatisfied.
///
/// **Blocking I/O.** With a `Some(context)` that sets `archive_root` or
/// `game_path`, each File leaf can issue up to two existence probes and each
/// Plugin leaf one. With `None`, or with a context whose two path fields are
/// empty, no probe runs.
///
/// The only caller that supplies a context is the install replay:
/// `installation_service::handle_fomod_install` builds one with `archive_root`
/// always set and `game_path` set only when the config JSON carries `gamePath`.
/// Inference passes `None` everywhere - the propagator, the CSP solver and every
/// `simulate()` call site - so a solve issues no probe at all.
pub fn evaluate_condition(
    condition: &FomodCondition,
    flags: &HashMap<String, String>,
    context: Option<&FomodDependencyContext>,
) -> bool {
    evaluate_condition_core(condition, flags, &|c| eval_leaf_normal(c, context), 0)
}

/// Evaluate a [`FomodCondition`] tree for inference. Flags evaluate exactly as
/// in [`evaluate_condition`]; the external conditions (File, Plugin, Fomod)
/// follow `external_override`.
///
/// - `ForceTrue`: external conditions are true
/// - `ForceFalse`: external conditions are false
/// - `Unknown`: same results as `ForceFalse` (see
///   [`ExternalConditionOverride`] for the full matrix and for why the two are
///   kept apart)
///
/// Infrastructure conditions (Game, Fomm, Fose) are true under all three
/// overrides.
///
/// **Depth.** Same guard as [`evaluate_condition`]: recursion starts at depth 0,
/// and a Composite subtree nested deeper than [`MAX_DEPENDENCY_DEPTH`] evaluates
/// as unmet and logs a `[fomod-ir]` warning.
///
/// **No I/O.** Inferred mode issues no filesystem probe, whatever `_context`
/// holds. The parameter is never read; it exists so both entry points take the
/// same call shape.
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

/// Determine a plugin's effective type under the current flag state: test
/// `type_patterns` in order, first match wins, otherwise fall back to the
/// plugin's declared type.
///
/// Context presence selects the evaluation mode, and that asymmetry is
/// deliberate. With a context the patterns evaluate in normal mode; without one
/// they evaluate in inferred mode under the `Unknown` override, so external
/// leaves answer false. The CSP solver always passes no context and therefore
/// always gets inferred-Unknown semantics.
///
/// Do not rewrite the `None` branch as `evaluate_condition(..., None)`. It looks
/// equivalent and is not: a File leaf with state `Missing` or `Inactive` is true
/// in normal mode without a context, and false under inferred-Unknown.
///
/// With a context this function can block on disk: up to two probes per File
/// leaf, `archive_root` then `game_path`, and one per Plugin leaf, for every
/// pattern it tests up to the first match. With `None` it does no I/O.
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

    /// Composite chain of `levels` nested composites. The root evaluates at
    /// depth 0, so the innermost composite sits at depth `levels - 1` and
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
        // 33 nested composites put the innermost at depth 32, equal to
        // MAX_DEPENDENCY_DEPTH. The guard is `depth > max`, so it still
        // evaluates, and the flag leaf makes it true.
        assert!(evaluate_condition(&nested_chain(33), &no_flags, None));
        // 34 nested composites put the innermost at depth 33, past the bound.
        // That node is false, collapsing the whole And chain to false.
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
        // Exists and is active -> Inactive is false.
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
        // An unrecognized state warns and falls through to the Active branch.
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

    // --- trap (d): MSVC path extension/filename semantics ------------------

    #[test]
    fn cpp_path_helpers_mirror_msvc_fs_path() {
        // The extension includes the dot.
        assert_eq!(cpp_path_extension("mod.esp"), ".esp");
        assert_eq!(cpp_path_extension("dir/mod.esp"), ".esp");
        assert_eq!(cpp_path_extension("dir\\mod.esp"), ".esp");
        // No dot -> no extension.
        assert_eq!(cpp_path_extension("mod"), "");
        // Leading dot with no other dot -> no extension (".gitignore").
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
        // The extension follows the corrected filename: "C:.esp" has filename
        // ".esp", whose only dot is leading, so there is no extension.
        assert_eq!(cpp_path_extension("C:foo.esp"), ".esp");
        assert_eq!(cpp_path_extension("C:.esp"), "");
        assert_eq!(cpp_path_extension("C:.gitignore"), "");

        // UNC root-name decomposition: exactly two leading separators plus a
        // non-separator start a root-name that runs to the next separator.
        // With no further separator the whole path is the root-name, so the
        // filename and extension are empty even for a plugin-shaped tail.
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
        // Three or more leading separators form no root-name, and "\??" is not
        // a device prefix without its trailing separator.
        assert_eq!(cpp_path_filename("///foo"), "foo");
        assert_eq!(cpp_path_filename("//"), "");
        assert_eq!(cpp_path_filename("\\??"), "??");
    }

    /// A UNC-root-name-only path with a plugin-shaped tail must not take the
    /// installed_plugins fallback. Its filename and extension are both empty, so
    /// the evaluator treats "//server.esp" as a non-plugin file that does not
    /// exist: "Active" is false even when server.esp is an active plugin, and
    /// "Inactive" is true.
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
        // Control: the same tail below a UNC share is a plugin filename.
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
        // Asymmetric with plugins: no lowercasing.
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
        // A context that would make the File leaf true in normal mode has no
        // effect in inferred mode, which never reads it.
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
            // Trailing dot yields no trailing empty token.
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
        // Some(ctx): normal mode, File leaf true -> pattern matches.
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
        // Distinguisher: a File leaf with state "Missing" or "Inactive" is true
        // in normal mode without a context, because file_exists is false, but
        // false under inferred-Unknown, where File leaves follow the override
        // whatever their state. Only the inferred dispatch is correct for the
        // None branch, and that is the hot path: the CSP solver always passes no
        // context. The ctx-presence test above cannot catch a regression to
        // evaluate_condition(..., None), because its "Active" leaf is false in
        // both modes without a context.
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
            // Some(empty ctx): normal mode -> file does not exist -> leaf
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
        // never panic.
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
