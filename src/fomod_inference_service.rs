/*!
 * @brief coordinates FOMOD selection inference.
 * @author Alex (https://github.com/lextpf)
 *
 * ### :material-transit-connection-variant: inference flow
 *
 * @verbatim
 * archive list -> XML parse -> atom expansion -> installed scan -> hash
 *              -> cache validation -> propagation -> CSP -> schema-v2 JSON
 * @endverbatim
 *
 * ### :material-alert-circle-outline: failure handling
 *
 * every operational failure returns an empty string. tier-1 metadata is only a candidate and
 * must reproduce the target exactly before it can bypass the solver.
 */

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Instant, UNIX_EPOCH};

use crate::archive_service::ArchiveService;
use crate::fomod_atom::{AtomIndex, ExpandedAtoms, Origin, TargetTree};
use crate::fomod_csp_solver::solve_fomod_csp;
use crate::fomod_csp_types::InferenceOverrides;
use crate::fomod_dependency_evaluator::ExternalConditionOverride;
use crate::fomod_forward_simulator::{SimulatedTree, classify_dests, compare_trees, simulate};
use crate::fomod_inference_atoms::{
    add_output_tree, add_repro_detail, assemble_json, build_atom_index, build_target_tree,
    compute_excluded_dests, expand_all_atoms,
};
use crate::fomod_ir::FomodInstaller;
use crate::fomod_ir_parser::parse_module_config;
use crate::fomod_propagator::propagate;
use crate::inference_diagnostics::{
    ConfidenceComponents, ConfidenceScore, DiagnosticCacheInfo, DiagnosticGroupCounts,
    DiagnosticTimings, InferenceDiagnosticsBuilder, Reason, ReasonCode, RunDiagnostics,
    serialize_confidence, serialize_reason, serialize_run_diagnostics,
};
use crate::json::{self, Value};
use crate::logger::Logger;
use crate::utils::{fnv1a_hash, normalize_path, to_lower};

/**
 * @brief entry count above which the whole hash cache is cleared.
 * @author Alex (https://github.com/lextpf)
 */
pub const K_MAX_CACHE_ENTRIES: usize = 100_000;

// largest installed file (256 MiB) read into memory for hashing.
const K_MAX_HASH_FILE_SIZE: u64 = 256 * 1024 * 1024;

/**
 * @struct CachedHash
 * @brief cached FNV-1a content hash and uncompressed size for one archive entry.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Copy, Default)]
pub struct CachedHash {
    pub hash: u64,
    /**
     * @brief uncompressed size of the archive entry in bytes.
     * @author Alex (https://github.com/lextpf)
     */
    pub size: u64,
}

/**
 * @struct FomodInferenceService
 * @brief reverse-engineers which FOMOD options were originally selected.
 * @author Alex (https://github.com/lextpf)
 *
 * `capi::inferFomodSelections` builds a fresh instance per call, so on the DLL path the hash cache
 * always starts empty. one C ABI call hashes one archive, so the cap does not clear that cache.
 */
#[derive(Debug, Default)]
pub struct FomodInferenceService {
    // instance-scoped hash cache for contested archive entries, keyed by
    // "archive_signature\nentry_path".
    // the lock is taken up to three times per call and is never held across archive I/O.
    cache: Mutex<HashMap<String, CachedHash>>,
}

impl FomodInferenceService {
    pub fn new() -> Self {
        FomodInferenceService::default()
    }

    /**
     * @fn infer_selections(&self, &str, &str) -> String
     * @brief infer FOMOD selections from an archive and its installed files.
     * @author Alex (https://github.com/lextpf)
     *
     * the guarantee covers error conditions, not panics; `capi::guard` contains those.
     * @return schema-v2 JSON (`dump(2)`) on success, or an empty string on any of six failures:
     * archive not found, mod not found, not a FOMOD, the XML entry unreadable, an XML parse error,
     * or a malformed tier-1 cache blob.
     */
    pub fn infer_selections(&self, archive_path: &str, mod_path: &str) -> String {
        let t_total = Instant::now();
        let logger = Logger::instance();

        // the banner reports the archive extension with its dot, and the size in MB to one decimal.
        // an unreadable size is reported as 0.0 rather than failing the call.
        let archive_ext = Path::new(archive_path)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let archive_size_mb = fs::metadata(archive_path)
            .map(|m| m.len() as f64 / (1024.0 * 1024.0))
            .unwrap_or(0.0);

        logger.log("[infer] ========================================");
        logger.log(&format!(
            "[infer] Archive: \"{archive_path}\" ({archive_size_mb:.1} MB, {archive_ext})"
        ));
        logger.log(&format!("[infer] Mod path: \"{mod_path}\""));
        logger.log("[infer] 0/9 Starting inference");

        // both paths must exist. either miss ends the call with an empty string.
        if !Path::new(archive_path).exists() {
            logger.log_error(&format!("[infer] Archive not found: {archive_path}"));
            return String::new();
        }
        if !Path::new(mod_path).exists() {
            logger.log_error(&format!("[infer] Mod path not found: {mod_path}"));
            return String::new();
        }

        // read the tier-1 fomod-plus blob only as a candidate. reading never fails the
        // call: its own errors collapse to `None`.
        let t_step = Instant::now();
        let fomod_plus = try_fomod_plus_json(Path::new(mod_path));
        if fomod_plus.is_some() {
            logger.log(&format!(
                "[infer] Tier 1 candidate: fomod-plus JSON found, will validate ({}ms)",
                t_step.elapsed().as_millis()
            ));
        } else {
            logger.log(&format!(
                "[infer] Tier 1 miss: no fomod-plus data ({}ms)",
                t_step.elapsed().as_millis()
            ));
        }

        // every failure below returns an empty string early; no Result crosses the FFI boundary.
        let archive_service = ArchiveService::new();

        // step 1: list archive entries with sizes. each `t_*` below measures its own stage only;
        // measuring from `t_total` would report cumulative elapsed time in
        // `diagnostics.timings_ms`.
        logger.log("[infer] 1/9 Listing archive entries");
        let t_step = Instant::now();
        let listing = archive_service.list_entries_with_sizes(archive_path);
        let t_list = t_step.elapsed().as_millis() as i64;
        logger.log(&format!(
            "[infer] Step 1 list_entries: {} entries, {} sizes ({t_list}ms)",
            listing.paths.len(),
            listing.sizes.len()
        ));

        // build the sorted normalized entry index and the normalized sizes map. the size lookup
        // uses the original entry path against a map keyed by the normalized path, so a size
        // propagates only for an entry whose raw path is already lowercase and forward-slashed;
        // every other atom keeps file_size 0.
        let mut sorted_norm_entries: Vec<String> = Vec::with_capacity(listing.paths.len());
        let mut norm_entry_sizes: HashMap<String, u64> = HashMap::new();
        for entry in &listing.paths {
            if entry.ends_with('/') || entry.ends_with('\\') {
                continue;
            }
            let norm = normalize_path(entry);
            if let Some(&sz) = listing.sizes.get(entry.as_str()) {
                norm_entry_sizes.insert(norm.clone(), sz);
            }
            sorted_norm_entries.push(norm);
        }
        sorted_norm_entries.sort();

        // step 2: find the FOMOD ModuleConfig entry (prefer shallowest path).
        logger.log("[infer] 2/9 Finding FOMOD config");
        const MODULE_CFG_SUFFIX: &str = "fomod/moduleconfig.xml";
        let mut xml_entry_norm = String::new();
        let mut best_depth = usize::MAX;
        for entry in &listing.paths {
            let norm = normalize_path(entry);
            let is_candidate = norm == MODULE_CFG_SUFFIX
                || (norm.len() > MODULE_CFG_SUFFIX.len()
                    && norm.ends_with(&format!("/{MODULE_CFG_SUFFIX}")));
            if !is_candidate {
                continue;
            }
            let depth = norm.matches('/').count();
            if depth < best_depth
                || (depth == best_depth
                    && (xml_entry_norm.is_empty() || norm.len() < xml_entry_norm.len()))
            {
                xml_entry_norm = norm;
                best_depth = depth;
            }
        }

        if xml_entry_norm.is_empty() {
            logger.log(&format!(
                "[infer] Not a FOMOD mod, total: {}ms",
                t_total.elapsed().as_millis()
            ));
            return String::new();
        }

        let mut suffix_pos = xml_entry_norm.len() - MODULE_CFG_SUFFIX.len();
        if suffix_pos > 0 && xml_entry_norm.as_bytes()[suffix_pos - 1] == b'/' {
            suffix_pos -= 1;
        }
        let fomod_prefix = if suffix_pos > 0 {
            xml_entry_norm[..suffix_pos].to_string()
        } else {
            String::new()
        };

        logger.log(&format!(
            "[infer] Step 2 found XML: \"{xml_entry_norm}\" (prefix: \"{fomod_prefix}\")"
        ));

        // step 3: read ModuleConfig.xml into memory.
        logger.log("[infer] 3/9 Reading ModuleConfig.xml");
        let t_step = Instant::now();
        let mut xml_set: HashSet<String> = HashSet::new();
        xml_set.insert(xml_entry_norm.clone());
        let xml_data = archive_service.read_entries_batch(archive_path, &xml_set);
        let Some(xml_bytes) = xml_data.get(&xml_entry_norm) else {
            logger.log_error("[infer] Failed to read ModuleConfig.xml from archive");
            return String::new();
        };
        logger.log(&format!(
            "[infer] Step 3 read XML: {} bytes ({}ms)",
            xml_bytes.len(),
            t_step.elapsed().as_millis()
        ));

        // step 4: Parse XML and build the IR.
        logger.log("[infer] 4/9 Parsing FOMOD XML");
        let t_step = Instant::now();
        let installer = match parse_module_config(xml_bytes, &fomod_prefix) {
            Ok(installer) => installer,
            Err(err) => {
                logger.log_error(&format!("[infer] XML parse failed: {err}"));
                return String::new();
            }
        };
        logger.log(&format!(
            "[infer] Step 4 parse IR: {} steps, {} cond patterns ({}ms)",
            installer.steps.len(),
            installer.conditional_patterns.len(),
            t_step.elapsed().as_millis()
        ));

        let mut diag_builder = InferenceDiagnosticsBuilder::new(&installer);

        // step 5: expand atoms.
        logger.log("[infer] 5/9 Expanding file atoms");
        let t_step = Instant::now();
        let mut atoms = expand_all_atoms(&installer, &sorted_norm_entries, &norm_entry_sizes);
        let mut total_atoms = 0;
        atoms.for_each(|_| total_atoms += 1);
        let mut atom_index = build_atom_index(&atoms);
        let excluded = compute_excluded_dests(&atom_index);
        logger.log(&format!(
            "[infer] Step 5 expand atoms: {total_atoms} total, {} dests, {} excluded ({}ms)",
            atom_index.len(),
            excluded.len(),
            t_step.elapsed().as_millis()
        ));

        // step 6: build the target tree from the installed-file scan. `t_scan` covers the walk and
        // the tree build, not the walk alone.
        logger.log("[infer] 6/9 Scanning installed files");
        let t_step = Instant::now();
        let installed = scan_installed_files(Path::new(mod_path));
        let mut target = build_target_tree(&installed);
        let t_scan = t_step.elapsed().as_millis() as i64;
        logger.log(&format!(
            "[infer] Step 6 target tree: {} files ({t_scan}ms)",
            target.len()
        ));

        // step 7: hash contested files for disambiguation (mutates target + atoms).
        logger.log("[infer] 7/9 Hashing contested files");
        let t_step = Instant::now();
        self.hash_contested_files(
            &mut target,
            &mut atoms,
            &mut atom_index,
            Path::new(mod_path),
            archive_path,
            &excluded,
        );
        logger.log(&format!(
            "[infer] Step 7 hash contested ({}ms)",
            t_step.elapsed().as_millis()
        ));

        // step 7b: precompute conditional + step-visibility overrides.
        let overrides = compute_overrides(&installer, &atoms, &atom_index, &target, &excluded);

        // feed step-visibility overrides into the diagnostics chain.
        for (si, mode) in overrides.step_visible.iter().enumerate() {
            match mode {
                ExternalConditionOverride::ForceTrue => diag_builder.set_step_visibility(
                    si as i32,
                    true,
                    ReasonCode::StepVisibilityForced,
                ),
                // unreachable: compute_overrides produces only ForceTrue and Unknown, so no step is
                // ever reported as not visible and its closing tally logs a `false` count that is
                // structurally 0. the arm exists for match exhaustiveness.
                ExternalConditionOverride::ForceFalse => {
                    diag_builder.set_step_visibility(si as i32, false, ReasonCode::StepNotVisible)
                }
                ExternalConditionOverride::Unknown => diag_builder.set_step_visibility(
                    si as i32,
                    true,
                    ReasonCode::StepVisibilityUnknown,
                ),
            }
        }

        // tier-1 validation runs here, between the 7b overrides and the 7c propagate: it needs the
        // overrides to simulate with, and short-circuiting before the expensive propagate and solve
        // is the point. it carries no step letter because 7b and 7c are fixed by the log lines that
        // emit them. short-circuit only on an exact reproduction.
        if let Some(fp) = &fomod_plus {
            let total_ms = t_total.elapsed().as_millis() as i64;
            match try_tier1_cache(
                fp, &installer, &atoms, &target, &excluded, &overrides, total_ms,
            ) {
                Tier1Outcome::Hit(out) => return out.dump(2),
                // a malformed step or group `name` fails the whole call.
                Tier1Outcome::Abort => return String::new(),
                Tier1Outcome::Miss => {}
            }
        }

        // step 7c: Constraint propagation pre-pass.
        let t_step = Instant::now();
        let propagation = propagate(
            &installer,
            &atoms,
            &atom_index,
            &target,
            &excluded,
            &overrides,
            None,
        );
        let total_groups: usize = installer.steps.iter().map(|s| s.groups.len()).sum();
        logger.log(&format!(
            "[infer] Step 7c propagate: resolved={}/{total_groups} groups, fully_resolved={} ({}ms)",
            propagation.resolved_groups.len(),
            propagation.fully_resolved,
            t_step.elapsed().as_millis()
        ));

        diag_builder.absorb_propagation(&propagation);

        // step 8: CSP solve (with propagation-narrowed domains).
        logger.log("[infer] 8/9 Solving (this may take a while)");
        let t_solve_start = Instant::now();
        let result = solve_fomod_csp(
            &installer,
            &atoms,
            &atom_index,
            &target,
            &excluded,
            Some(&overrides),
            if propagation.resolved_groups.is_empty() {
                None
            } else {
                Some(&propagation)
            },
        );
        let t_solve = t_solve_start.elapsed().as_millis() as i64;
        logger.log(&format!(
            "[infer] Step 8 solve: {} nodes, exact={} ({t_solve}ms)",
            result.nodes_explored, result.exact_match
        ));
        diag_builder.absorb_solver(&result);

        // step 9: assemble JSON.
        logger.log("[infer] 9/9 Assembling result");
        let t_step = Instant::now();
        let total_ms = t_total.elapsed().as_millis() as i64;
        diag_builder.set_run_timings(t_list, t_scan, t_solve, total_ms);
        diag_builder.set_target_file_count(target.len() as i32);
        diag_builder.finalize(&result, &propagation, &installer);
        let mut json_result = assemble_json(&installer, &result, diag_builder.diagnostics());
        let out_sim = simulate(
            &installer,
            &atoms,
            &result.selections,
            None,
            Some(&overrides),
        );
        add_output_tree(&mut json_result, &out_sim);
        // which dests diverged, beside the counts the diagnostics already carry.
        add_repro_detail(
            &mut json_result,
            &classify_dests(&out_sim, &target, &excluded),
        );
        let result_str = json_result.dump(2);
        logger.log(&format!(
            "[infer] Step 9 assemble JSON: {} bytes ({}ms)",
            result_str.len(),
            t_step.elapsed().as_millis()
        ));

        logger.log(&format!(
            "[infer] DONE total={total_ms}ms | list={t_list}ms scan={t_scan}ms solve={t_solve}ms"
        ));
        logger.log("[infer] ========================================");

        result_str
    }

    // hash only contested destinations so equal-size candidates can be distinguished.
    pub fn hash_contested_files(
        &self,
        target: &mut TargetTree,
        atoms: &mut ExpandedAtoms,
        atom_index: &mut AtomIndex,
        mod_path: &Path,
        archive_path: &str,
        excluded: &HashSet<String>,
    ) {
        // bounded hash cache: clear-all when the cap is exceeded (both the check and the clear
        // under one lock, so two threads cannot double-clear).
        {
            let mut cache = self.cache.lock().unwrap();
            if cache.len() > K_MAX_CACHE_ENTRIES {
                Logger::instance().log(&format!(
                    "[infer] Hash cache exceeded {K_MAX_CACHE_ENTRIES} entries, clearing"
                ));
                cache.clear();
            }
        }

        // phase 1: find contested destinations.
        let (contested_dests, entries_to_read) = find_contested_dests(target, atom_index, excluded);
        if contested_dests.is_empty() {
            return;
        }

        Logger::instance().log(&format!(
            "[infer] Hashing {} contested dests ({} archive entries)",
            contested_dests.len(),
            entries_to_read.len()
        ));

        // phase 2: fetch entry hashes (cache lookup + archive read).
        let hashes = self.fetch_entry_hashes(archive_path, &entries_to_read);

        // phase 3: apply hashes to atoms and target files.
        apply_entry_hashes(
            atom_index,
            atoms,
            target,
            &contested_dests,
            &hashes,
            mod_path,
        );
    }

    fn fetch_entry_hashes(
        &self,
        archive_path: &str,
        entries_to_read: &HashSet<String>,
    ) -> HashResult {
        let mut result = HashResult::default();
        let mut missing_entries: HashSet<String> = HashSet::new();

        let archive_sig = build_archive_signature(archive_path);
        let mut cache_hits = 0usize;
        {
            let cache = self.cache.lock().unwrap();
            for entry_path in entries_to_read {
                let key = format!("{archive_sig}\n{entry_path}");
                if let Some(hit) = cache.get(&key) {
                    cache_hits += 1;
                    result.source_hashes.insert(entry_path.clone(), hit.hash);
                    result.source_sizes.insert(entry_path.clone(), hit.size);
                } else {
                    missing_entries.insert(entry_path.clone());
                }
            }
        }

        if !missing_entries.is_empty() {
            let archive_service = ArchiveService::new();
            let batch_data = archive_service.read_entries_batch(archive_path, &missing_entries);

            let mut new_entries: Vec<(String, CachedHash)> = Vec::new();
            for (entry_path, data) in batch_data {
                let h = fnv1a_hash(&data);
                let sz = data.len() as u64;
                result.source_hashes.insert(entry_path.clone(), h);
                result.source_sizes.insert(entry_path.clone(), sz);
                let key = format!("{archive_sig}\n{entry_path}");
                new_entries.push((key, CachedHash { hash: h, size: sz }));
            }
            let mut cache = self.cache.lock().unwrap();
            for (key, value) in new_entries {
                cache.insert(key, value);
            }
        }

        // the batch read borrows `missing_entries`, so it is still the miss count here.
        Logger::instance().log(&format!(
            "[infer] Contested hash cache: hits={cache_hits}, misses={}",
            missing_entries.len()
        ));

        result
    }
}

/**
 * @fn try_fomod_plus_json(&Path) -> Option<Value>
 * @brief read any cached fomod-plus JSON from mod/meta.ini.
 * @author Alex (https://github.com/lextpf)
 *
 * read and parse errors collapse to `None`, so this never fails the inference call.
 * @return `Some(json)` only when the `[Settings]` key `fomod plus/fomod` (case-insensitive) holds a
 * JSON object with a non-empty `steps` array; otherwise `None`.
 */
pub fn try_fomod_plus_json(mod_path: &Path) -> Option<Value> {
    let meta_ini = mod_path.join("meta.ini");
    if !meta_ini.exists() {
        return None;
    }
    let bytes = fs::read(&meta_ini).ok()?;

    let mut in_settings = false;
    for (idx, line) in getline_split(&bytes).enumerate() {
        if idx + 1 > 10000 {
            Logger::instance().log_warning("[infer] meta.ini exceeds 10000 lines, aborting parse");
            return None;
        }
        let trimmed = trim_bytes(line, b" \t\r\n");
        if trimmed.is_empty() {
            continue;
        }
        if trimmed[0] == b'[' {
            // ASCII comparison; a non-UTF-8 header can never equal "[settings]".
            in_settings = to_lower(&String::from_utf8_lossy(trimmed)) == "[settings]";
            continue;
        }
        if !in_settings {
            continue;
        }

        let Some(eq_pos) = trimmed.iter().position(|&b| b == b'=') else {
            continue;
        };

        let key = trim_bytes(&trimmed[..eq_pos], b" \t");
        if to_lower(&String::from_utf8_lossy(key)) != "fomod plus/fomod" {
            continue;
        }

        let mut value = trim_bytes(&trimmed[eq_pos + 1..], b" \t");
        // peel one outer quote pair (once).
        if value.len() >= 2 && value[0] == b'"' && value[value.len() - 1] == b'"' {
            value = &value[1..value.len() - 1];
        }

        if value.is_empty() || value == b"{}" || value == b"\"{}\"" {
            Logger::instance().log("[infer] fomod-plus JSON found but empty - rejecting");
            return None;
        }

        // ill-formed UTF-8 is a parse failure like any other: log it as one and take the tier-1
        // miss.
        let Ok(value) = std::str::from_utf8(value) else {
            Logger::instance()
                .log("[infer] Failed to parse fomod-plus JSON: invalid UTF-8 in value");
            return None;
        };

        // first matching key wins whether it parses/validates or not.
        match json::parse(value) {
            Ok(j) => {
                let has_steps = j
                    .get("steps")
                    .map(|s| s.is_array() && !s.is_empty())
                    .unwrap_or(false);
                if has_steps {
                    Logger::instance().log("[infer] Using fomod-plus JSON from meta.ini (Tier 1)");
                    return Some(j);
                }
                Logger::instance().log("[infer] fomod-plus JSON has no steps - rejecting");
                return None;
            }
            Err(err) => {
                Logger::instance().log(&format!("[infer] Failed to parse fomod-plus JSON: {err}"));
                return None;
            }
        }
    }
    None
}

// split raw file bytes into lines: one line per \n-terminated segment, plus a final unterminated
// segment only when it is non-empty.
// an empty file yields no lines at all.
fn getline_split(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let trimmed = match bytes.last() {
        Some(b'\n') => &bytes[..bytes.len() - 1],
        _ => bytes,
    };
    let empty = trimmed.is_empty() && bytes.is_empty();
    trimmed.split(|&b| b == b'\n').skip(usize::from(empty))
}

// trim every leading and trailing byte contained in set.
// works on bytes, not code points, and takes an explicit set.
fn trim_bytes<'a>(bytes: &'a [u8], set: &[u8]) -> &'a [u8] {
    let mut start = 0;
    while start < bytes.len() && set.contains(&bytes[start]) {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && set.contains(&bytes[end - 1]) {
        end -= 1;
    }
    &bytes[start..end]
}

// infer true only when one conditional uniquely produces an in-target destination; otherwise use
// Unknown.
pub fn compute_overrides(
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    atom_index: &AtomIndex,
    target: &TargetTree,
    excluded: &HashSet<String>,
) -> InferenceOverrides {
    let mut overrides = InferenceOverrides {
        conditional_active: vec![
            ExternalConditionOverride::Unknown;
            installer.conditional_patterns.len()
        ],
        step_visible: vec![ExternalConditionOverride::Unknown; installer.steps.len()],
    };

    // cond_only_dest_patterns[dest] = the conditional-index producer set for dests reached only by
    // conditional atoms.
    let mut cond_only_dest_patterns: HashMap<String, HashSet<i32>> = HashMap::new();
    for (dest, atoms_for_dest) in atom_index {
        if excluded.contains(dest) || !target.contains_key(dest) {
            continue;
        }
        let only_conditional = atoms_for_dest
            .iter()
            .all(|a| a.origin == Origin::Conditional);
        if !only_conditional {
            continue;
        }
        let producers = cond_only_dest_patterns.entry(dest.clone()).or_default();
        for atom in atoms_for_dest {
            if atom.conditional_index >= 0 {
                producers.insert(atom.conditional_index);
            }
        }
    }

    // both counters exist only to be logged below.
    let mut cond_unique_forced = 0;
    let mut cond_ambiguous_skipped = 0;
    for ci in 0..atoms.per_conditional.len() {
        let mut has_unique_cond_only_hit = false;
        for atom in &atoms.per_conditional[ci] {
            if excluded.contains(&atom.dest_path) {
                continue;
            }
            if !target.contains_key(&atom.dest_path) {
                continue;
            }
            let Some(producers) = cond_only_dest_patterns.get(&atom.dest_path) else {
                continue;
            };
            if producers.is_empty() {
                continue;
            }
            if producers.len() == 1 && producers.contains(&(ci as i32)) {
                has_unique_cond_only_hit = true;
                break;
            }
            cond_ambiguous_skipped += 1;
        }
        if has_unique_cond_only_hit {
            overrides.conditional_active[ci] = ExternalConditionOverride::ForceTrue;
            cond_unique_forced += 1;
        }
    }
    Logger::instance().log(&format!(
        "[infer] Step 7b conditional evidence: unique_forced={cond_unique_forced}, ambiguous_skipped={cond_ambiguous_skipped}"
    ));

    // step visibility via step-unique evidence: ForceTrue if and only if a step has at least one
    // target-hit dest reached by its own plugins that no other step reaches.
    let mut step_dests: Vec<HashSet<String>> = vec![HashSet::new(); installer.steps.len()];
    let mut flat_idx: usize = 0;
    for (si, step) in installer.steps.iter().enumerate() {
        for group in &step.groups {
            for _pi in 0..group.plugins.len() {
                if flat_idx < atoms.per_plugin.len() {
                    for atom in &atoms.per_plugin[flat_idx] {
                        if !excluded.contains(&atom.dest_path)
                            && target.contains_key(&atom.dest_path)
                        {
                            step_dests[si].insert(atom.dest_path.clone());
                        }
                    }
                } else {
                    // IR/atom desync: skip rather than panic.
                    Logger::instance().log_warning(&format!(
                        "[infer] compute_overrides: flat_idx {flat_idx} out of range ({}), possible IR/atom desync",
                        atoms.per_plugin.len()
                    ));
                }
                flat_idx += 1;
            }
        }
    }

    let mut dest_step_count: HashMap<&str, i32> = HashMap::new();
    for s in &step_dests {
        for d in s {
            *dest_step_count.entry(d.as_str()).or_insert(0) += 1;
        }
    }

    for (si, dests) in step_dests.iter().enumerate() {
        for d in dests {
            if dest_step_count.get(d.as_str()) == Some(&1) {
                overrides.step_visible[si] = ExternalConditionOverride::ForceTrue;
                break;
            }
        }
    }

    // closing tallies over both override vectors; log-only.
    let tally = |modes: &[ExternalConditionOverride]| {
        let mut t = 0;
        let mut f = 0;
        let mut u = 0;
        for mode in modes {
            match mode {
                ExternalConditionOverride::ForceTrue => t += 1,
                ExternalConditionOverride::ForceFalse => f += 1,
                ExternalConditionOverride::Unknown => u += 1,
            }
        }
        (t, f, u)
    };
    let (cond_true, cond_false, cond_unknown) = tally(&overrides.conditional_active);
    let (step_true, step_false, step_unknown) = tally(&overrides.step_visible);
    Logger::instance().log(&format!(
        "[infer] Step 7b overrides: cond true/false/unknown={cond_true}/{cond_false}/{cond_unknown}, steps true/false/unknown={step_true}/{step_false}/{step_unknown}"
    ));

    overrides
}

/**
 * @fn scan_installed_files(&Path) -> HashMap<String, u64>
 * @brief skip unreadable subtrees and record metadata failures with size zero.
 * @author Alex (https://github.com/lextpf)
 *
 */
pub fn scan_installed_files(mod_path: &Path) -> HashMap<String, u64> {
    let mut files: HashMap<String, u64> = HashMap::new();
    if !mod_path.exists() {
        return files;
    }

    let mut stack: Vec<std::path::PathBuf> = vec![mod_path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read_dir = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(err) => {
                // tolerate permission-denied and transient errors. the walk is per-directory, so
                // the warning names the directory that failed rather than the mod root.
                Logger::instance()
                    .log_warning(&format!("[infer] Error iterating {}: {err}", dir.display()));
                continue;
            }
        };
        for entry in read_dir.flatten() {
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(err) => {
                    Logger::instance().log_warning(&format!(
                        "[infer] Error processing entry in {}: {err}",
                        mod_path.display()
                    ));
                    continue;
                }
            };
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                let Ok(rel) = path.strip_prefix(mod_path) else {
                    continue;
                };
                let rel_str = rel.to_string_lossy();
                let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                files.insert(normalize_path(&rel_str), size);
            }
        }
    }
    files
}

#[derive(Debug, Default)]
struct HashResult {
    source_hashes: HashMap<String, u64>,
    source_sizes: HashMap<String, u64>,
}

// phase 1 of contested hashing: find the dests that more than one distinct size-compatible source
// can produce.
fn find_contested_dests(
    target: &TargetTree,
    atom_index: &AtomIndex,
    excluded: &HashSet<String>,
) -> (HashSet<String>, HashSet<String>) {
    let mut contested_dests: HashSet<String> = HashSet::new();
    let mut entries_to_read: HashSet<String> = HashSet::new();

    for (dest, target_file) in target {
        if excluded.contains(dest) {
            continue;
        }
        let Some(atoms_for_dest) = atom_index.get(dest) else {
            continue;
        };

        let mut candidate_sources: HashSet<&str> = HashSet::new();
        for a in atoms_for_dest {
            if !a.source_path.is_empty()
                && (a.file_size == 0 || target_file.size == 0 || a.file_size == target_file.size)
            {
                candidate_sources.insert(a.source_path.as_str());
            }
        }
        if candidate_sources.len() > 1 {
            contested_dests.insert(dest.clone());
            for src in candidate_sources {
                entries_to_read.insert(src.to_string());
            }
        }
    }

    (contested_dests, entries_to_read)
}

// build the hash-cache key prefix canonical_path|size|mtime for one archive.
// the exact encoding is free to change: the signature never leaves this process and never reaches
// the output document.
fn build_archive_signature(archive_path: &str) -> String {
    let path = Path::new(archive_path);
    let normalized_path = match fs::canonicalize(path) {
        Ok(canon) => normalize_path(&canon.to_string_lossy()),
        Err(_) => normalize_path(archive_path),
    };
    let meta = fs::metadata(path).ok();
    let sz = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{normalized_path}|{sz}|{mtime}")
}

// phase 3 of contested hashing: write the fetched hashes onto the atoms, then hash the installed
// file at each contested dest into the target.
// that matters because the two views have different readers.
fn apply_entry_hashes(
    atom_index: &mut AtomIndex,
    atoms: &mut ExpandedAtoms,
    target: &mut TargetTree,
    contested_dests: &HashSet<String>,
    hashes: &HashResult,
    mod_path: &Path,
) {
    // update atoms in the index with hashes.
    for (dest, atom_vec) in atom_index.iter_mut() {
        if !contested_dests.contains(dest) {
            continue;
        }
        for a in atom_vec.iter_mut() {
            if let Some(&h) = hashes.source_hashes.get(&a.source_path) {
                a.content_hash = h;
                if let Some(&sz) = hashes.source_sizes.get(&a.source_path) {
                    a.file_size = sz;
                }
            }
        }
    }

    // update atoms in the ExpandedAtoms struct.
    atoms.for_each_mut(|a| {
        if let Some(&h) = hashes.source_hashes.get(&a.source_path) {
            a.content_hash = h;
            if let Some(&sz) = hashes.source_sizes.get(&a.source_path) {
                a.file_size = sz;
            }
        }
    });

    // hash installed files at contested dests.
    for dest in contested_dests {
        let Some(tf) = target.get_mut(dest) else {
            continue;
        };
        let full_path = mod_path.join(dest);
        let Ok(meta) = fs::metadata(&full_path) else {
            continue;
        };
        let sz = meta.len();
        if sz > K_MAX_HASH_FILE_SIZE {
            Logger::instance().log_warning(&format!(
                "[infer] Skipping oversized file for hashing ({sz} bytes): {dest}"
            ));
            continue;
        }
        // the whole file is read into memory to hash it, which is why the size cap above exists. a
        // read error leaves the target entry untouched.
        let Ok(buf) = fs::read(&full_path) else {
            continue;
        };
        tf.hash = fnv1a_hash(&buf);
        tf.size = buf.len() as u64;
    }
}

/**
 * @enum Tier1Outcome
 * @brief outcome of validating the tier-1 fomod-plus candidate.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug)]
pub enum Tier1Outcome {
    Hit(Box<Value>),
    Miss,
    /**
     * @brief stop when a cached step, group or present name has the wrong JSON type.
     * @author Alex (https://github.com/lextpf)
     *
     * this fails the whole inference call, which returns `""`.
     */
    Abort,
}

impl Tier1Outcome {
    pub fn is_hit(&self) -> bool {
        matches!(self, Tier1Outcome::Hit(_))
    }

    pub fn is_miss(&self) -> bool {
        matches!(self, Tier1Outcome::Miss)
    }

    pub fn hit(self) -> Option<Value> {
        match self {
            Tier1Outcome::Hit(value) => Some(*value),
            _ => None,
        }
    }
}

// read a name field, keeping "absent" and "wrong type" apart.
// `Some("")` is an ordinary miss, because no step or group in the IR is named `""`.
fn name_field(src: &Value) -> Option<&str> {
    if !src.is_object() {
        return None;
    }
    match src.get("name") {
        None => Some(""),
        Some(v) => v.as_str(),
    }
}

// extract a plugin name from a cached JSON entry: the string itself, or the string name of an
// object.
fn plugin_name_of(src: &Value) -> String {
    if let Some(s) = src.as_str() {
        return s.to_string();
    }
    if let Some(name) = src.get("name").and_then(Value::as_str) {
        return name.to_string();
    }
    String::new()
}

// malformed names abort. resolution or reproduction misses fall through. only exact reproduction
// hits.
pub fn try_tier1_cache(
    fomod_plus: &Value,
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    target: &TargetTree,
    excluded: &HashSet<String>,
    overrides: &InferenceOverrides,
    total_ms: i64,
) -> Tier1Outcome {
    let mut grid: Vec<Vec<Vec<bool>>> = installer
        .steps
        .iter()
        .map(|step| {
            step.groups
                .iter()
                .map(|group| vec![false; group.plugins.len()])
                .collect()
        })
        .collect();

    let mut cache_resolved = true;
    // names the first unresolvable entry, for the "cache stale" warning only.
    let mut stale_what = String::new();
    'steps: for src_step in fomod_plus.get("steps").into_iter().flat_map(array_iter) {
        let Some(step_name) = name_field(src_step) else {
            return Tier1Outcome::Abort;
        };
        let Some(si) = installer.steps.iter().position(|s| s.name == step_name) else {
            cache_resolved = false;
            stale_what = format!("step \"{step_name}\"");
            break;
        };
        let step = &installer.steps[si];

        let Some(groups) = src_step.get("groups").filter(|g| g.is_array()) else {
            continue;
        };
        for src_group in array_iter(groups) {
            let Some(group_name) = name_field(src_group) else {
                return Tier1Outcome::Abort;
            };
            let Some(gi) = step.groups.iter().position(|g| g.name == group_name) else {
                cache_resolved = false;
                stale_what = format!("group \"{group_name}\" in step \"{step_name}\"");
                break 'steps;
            };
            let group = &step.groups[gi];

            let Some(plugins) = src_group.get("plugins").filter(|p| p.is_array()) else {
                continue;
            };
            for src_plugin in array_iter(plugins) {
                let plugin_name = plugin_name_of(src_plugin);
                let Some(pi) = group.plugins.iter().position(|p| p.name == plugin_name) else {
                    cache_resolved = false;
                    stale_what = format!(
                        "plugin \"{plugin_name}\" in group \"{group_name}\" of step \"{step_name}\""
                    );
                    break 'steps;
                };
                grid[si][gi][pi] = true;
            }
        }
    }

    if !cache_resolved {
        Logger::instance().log_warning(&format!(
            "[infer] Tier 1 cache stale: {stale_what} not found in installer - falling through"
        ));
        return Tier1Outcome::Miss;
    }

    // forward-simulate the cached selection with the same atoms + overrides the solver path uses,
    // then diff against the target.
    let t_step = Instant::now();
    let sim = simulate(installer, atoms, &grid, None, Some(overrides));
    let repro = compare_trees(&sim, target, excluded);
    Logger::instance().log(&format!(
        "[infer] Tier 1 validate: missing={} extra={} size_mismatch={} hash_mismatch={} ({}ms)",
        repro.missing,
        repro.extra,
        repro.size_mismatch,
        repro.hash_mismatch,
        t_step.elapsed().as_millis()
    ));

    if !repro.exact() {
        Logger::instance().log_warning(&format!(
            "[infer] Tier 1 cache did not reproduce target (missing={} extra={} size_mismatch={} hash_mismatch={}) - falling through to normal inference",
            repro.missing, repro.extra, repro.size_mismatch, repro.hash_mismatch
        ));
        return Tier1Outcome::Miss;
    }

    // `total_ms` was measured by the caller immediately before this call.
    Logger::instance().log(&format!(
        "[infer] Tier 1 hit: cache reproduces target exactly, total: {total_ms}ms"
    ));
    Tier1Outcome::Hit(Box::new(build_tier1_json(fomod_plus, &sim, total_ms)))
}

// iterate the elements of a JSON array value; empty for non-arrays.
fn array_iter(value: &Value) -> std::slice::Iter<'_, Value> {
    match value {
        Value::Array(items) => items.iter(),
        _ => [].iter(),
    }
}

// build the bespoke tier-1 schema-v2 document from the cached blob and the validation simulation.
fn build_tier1_json(fomod_plus: &Value, sim: &SimulatedTree, total_ms: i64) -> Value {
    let cache_confidence = || {
        serialize_confidence(&ConfidenceScore {
            composite: 1.0,
            band: "high".to_string(),
            components: ConfidenceComponents {
                evidence: 1.0,
                propagation: 1.0,
                repro: 1.0,
                ambiguity: 1.0,
            },
        })
    };
    let cache_reason = || {
        let mut arr = Value::array();
        arr.push(serialize_reason(&Reason {
            code: ReasonCode::FomodPlusCache,
            message: "Cached selection from meta.ini".to_string(),
            detail: None,
        }));
        arr
    };

    let convert_plugin = |src: &Value, selected: bool| -> Value {
        let mut p = Value::object();
        if let Some(s) = src.as_str() {
            p.insert("name", Value::string(s));
        } else if let Some(name) = src.get("name") {
            p.insert("name", name.clone());
        } else {
            p.insert("name", Value::string(""));
        }
        p.insert("selected", Value::Bool(selected));
        p.insert("confidence", cache_confidence());
        p.insert("reasons", cache_reason());
        p
    };

    let mut out = Value::object();
    out.insert("schema_version", Value::Int(2));

    let mut out_steps = Value::array();
    let mut cache_total_groups = 0i32;
    for src_step in fomod_plus.get("steps").into_iter().flat_map(array_iter) {
        let mut out_step = Value::object();
        // `name_field` cannot be None here: this emitter only runs after try_tier1_cache validated
        // every step and group it walks.
        out_step.insert("name", Value::string(name_field(src_step).unwrap_or("")));
        out_step.insert("confidence", cache_confidence());
        out_step.insert("visible", Value::Bool(true));
        out_step.insert("reasons", Value::array());

        let mut out_groups = Value::array();
        if let Some(groups) = src_step.get("groups").filter(|g| g.is_array()) {
            for src_group in array_iter(groups) {
                cache_total_groups += 1;
                let mut out_group = Value::object();
                out_group.insert("name", Value::string(name_field(src_group).unwrap_or("")));
                out_group.insert("confidence", cache_confidence());
                out_group.insert("resolved_by", Value::string("cache.fomod_plus"));
                out_group.insert("reasons", Value::array());

                let mut sel_arr = Value::array();
                let mut desel_arr = Value::array();
                if let Some(plugins) = src_group.get("plugins").filter(|p| p.is_array()) {
                    for p in array_iter(plugins) {
                        sel_arr.push(convert_plugin(p, true));
                    }
                }
                if let Some(deselected) = src_group.get("deselected").filter(|p| p.is_array()) {
                    for p in array_iter(deselected) {
                        desel_arr.push(convert_plugin(p, false));
                    }
                }
                out_group.insert("plugins", sel_arr);
                out_group.insert("deselected", desel_arr);
                out_groups.push(out_group);
            }
        }
        out_step.insert("groups", out_groups);
        out_steps.push(out_step);
    }
    out.insert("steps", out_steps);

    let run = RunDiagnostics {
        confidence: ConfidenceScore {
            composite: 1.0,
            band: "high".to_string(),
            components: ConfidenceComponents {
                evidence: 1.0,
                propagation: 1.0,
                repro: 1.0,
                ambiguity: 1.0,
            },
        },
        exact_match: true,
        phase_reached: "tier1_cache".to_string(),
        nodes_explored: 0,
        groups: DiagnosticGroupCounts {
            total: cache_total_groups,
            resolved_by_propagation: cache_total_groups,
            resolved_by_csp: 0,
        },
        repro: Default::default(),
        timings: DiagnosticTimings {
            list_ms: 0,
            scan_ms: 0,
            solve_ms: 0,
            total_ms,
        },
        cache: DiagnosticCacheInfo {
            hit: true,
            source: "fomod-plus".to_string(),
        },
    };
    out.insert("diagnostics", serialize_run_diagnostics(&run));

    add_output_tree(&mut out, sim);
    // a hit means the cached selection reproduced the installed tree exactly, so there is nothing
    // to mark. the key is still emitted, empty, so a consumer never has to special-case the cache
    // path.
    add_repro_detail(&mut out, &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_atom::{FomodAtom, TargetFile};
    use crate::fomod_ir::{FomodConditionalPattern, FomodGroup, FomodPlugin, FomodStep};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn plugin(name: &str) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            ..FomodPlugin::default()
        }
    }

    fn step_with_plugins(name: &str, plugin_names: &[&str]) -> FomodStep {
        FomodStep {
            name: name.to_string(),
            groups: plugin_names
                .iter()
                .map(|p| FomodGroup {
                    plugins: vec![plugin(p)],
                    ..FomodGroup::default()
                })
                .collect(),
            ..FomodStep::default()
        }
    }

    fn cond_atom(dest: &str, ci: i32) -> FomodAtom {
        FomodAtom {
            source_path: format!("src/{dest}"),
            dest_path: dest.to_string(),
            origin: Origin::Conditional,
            conditional_index: ci,
            ..FomodAtom::default()
        }
    }

    fn plugin_atom(dest: &str, flat_idx: i32) -> FomodAtom {
        FomodAtom {
            source_path: format!("src/{dest}"),
            dest_path: dest.to_string(),
            origin: Origin::Plugin,
            plugin_index: flat_idx,
            ..FomodAtom::default()
        }
    }

    fn target_of(dests: &[&str]) -> TargetTree {
        dests
            .iter()
            .map(|d| (d.to_string(), TargetFile { size: 1, hash: 0 }))
            .collect()
    }

    fn index_of(atoms: &[FomodAtom]) -> AtomIndex {
        let mut idx = AtomIndex::new();
        for a in atoms {
            idx.entry(a.dest_path.clone()).or_default().push(a.clone());
        }
        idx
    }

    #[test]
    fn compute_overrides_conditional_unique_dest_is_force_true() {
        let installer = FomodInstaller {
            conditional_patterns: vec![
                FomodConditionalPattern::default(),
                FomodConditionalPattern::default(),
            ],
            ..FomodInstaller::default()
        };
        let c0_uniq = cond_atom("uniq0", 0);
        let c0_shared = cond_atom("shared", 0);
        let c1_shared = cond_atom("shared", 1);
        let atoms = ExpandedAtoms {
            per_conditional: vec![
                vec![c0_uniq.clone(), c0_shared.clone()],
                vec![c1_shared.clone()],
            ],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[c0_uniq, c0_shared, c1_shared]);
        let target = target_of(&["uniq0", "shared"]);
        let excluded = HashSet::new();

        let ov = compute_overrides(&installer, &atoms, &index, &target, &excluded);
        assert_eq!(
            ov.conditional_active[0],
            ExternalConditionOverride::ForceTrue
        );
        assert_eq!(ov.conditional_active[1], ExternalConditionOverride::Unknown);
    }

    #[test]
    fn compute_overrides_conditional_dest_shared_with_plugin_is_unknown() {
        // "shared" is produced by a conditional and a plugin -> not conditional-only -> the
        // conditional is never forced.
        let installer = FomodInstaller {
            conditional_patterns: vec![FomodConditionalPattern::default()],
            steps: vec![step_with_plugins("S", &["P"])],
            ..FomodInstaller::default()
        };
        let c0 = cond_atom("shared", 0);
        let p0 = plugin_atom("shared", 0);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![p0.clone()]],
            per_conditional: vec![vec![c0.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[c0, p0]);
        let target = target_of(&["shared"]);
        let excluded = HashSet::new();

        let ov = compute_overrides(&installer, &atoms, &index, &target, &excluded);
        assert_eq!(ov.conditional_active[0], ExternalConditionOverride::Unknown);
    }

    #[test]
    fn compute_overrides_step_unique_dest_is_force_true_shared_is_unknown() {
        let installer = FomodInstaller {
            steps: vec![
                step_with_plugins("S0", &["P0"]),
                step_with_plugins("S1", &["P1"]),
            ],
            ..FomodInstaller::default()
        };
        let s0_uniq = plugin_atom("s0uniq", 0);
        let s0_shared = plugin_atom("shared_step", 0);
        let s1_shared = plugin_atom("shared_step", 1);
        let atoms = ExpandedAtoms {
            per_plugin: vec![
                vec![s0_uniq.clone(), s0_shared.clone()],
                vec![s1_shared.clone()],
            ],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[s0_uniq, s0_shared, s1_shared]);
        let target = target_of(&["s0uniq", "shared_step"]);
        let excluded = HashSet::new();

        let ov = compute_overrides(&installer, &atoms, &index, &target, &excluded);
        assert_eq!(ov.step_visible[0], ExternalConditionOverride::ForceTrue);
        assert_eq!(ov.step_visible[1], ExternalConditionOverride::Unknown);
    }

    #[test]
    fn compute_overrides_excluded_and_absent_dests_do_not_force() {
        let installer = FomodInstaller {
            conditional_patterns: vec![FomodConditionalPattern::default()],
            steps: vec![step_with_plugins("S", &["P"])],
            ..FomodInstaller::default()
        };
        let p0 = plugin_atom("ex", 0);
        let c0 = cond_atom("absent", 0);
        let atoms = ExpandedAtoms {
            per_plugin: vec![vec![p0.clone()]],
            per_conditional: vec![vec![c0.clone()]],
            ..ExpandedAtoms::default()
        };
        let index = index_of(&[p0, c0]);
        let target = target_of(&["ex"]);
        let mut excluded = HashSet::new();
        excluded.insert("ex".to_string());

        let ov = compute_overrides(&installer, &atoms, &index, &target, &excluded);
        assert_eq!(ov.step_visible[0], ExternalConditionOverride::Unknown);
        assert_eq!(ov.conditional_active[0], ExternalConditionOverride::Unknown);
    }

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn run_meta_ini(contents: &str) -> Option<Value> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("salma_t12_meta_{}_{seq}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        fs::write(dir.join("meta.ini"), contents).expect("write meta.ini");
        let result = try_fomod_plus_json(&dir);
        let _ = fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn fomod_plus_valid_under_settings_is_accepted() {
        let out = run_meta_ini(
            "[General]\nfoo=bar\n[Settings]\nfomod plus/fomod={\"steps\":[{\"name\":\"x\"}]}\n",
        );
        let v = out.expect("valid fomod-plus JSON accepted");
        assert_eq!(v.get("steps").and_then(Value::array_len), Some(1));
    }

    #[test]
    fn fomod_plus_key_outside_settings_is_ignored() {
        let out = run_meta_ini("[General]\nfomod plus/fomod={\"steps\":[{\"name\":\"x\"}]}\n");
        assert!(out.is_none());
    }

    #[test]
    fn fomod_plus_non_settings_header_turns_tracking_off() {
        let out = run_meta_ini(
            "[Settings]\n[General]\nfomod plus/fomod={\"steps\":[{\"name\":\"x\"}]}\n",
        );
        assert!(out.is_none());
    }

    #[test]
    fn fomod_plus_settings_and_key_are_case_insensitive() {
        let out = run_meta_ini("[SETTINGS]\nFoMoD PLUS/FOMOD={\"steps\":[{\"name\":\"x\"}]}\n");
        assert!(out.is_some());
    }

    #[test]
    fn fomod_plus_single_outer_quote_peel() {
        let out = run_meta_ini("[Settings]\nfomod plus/fomod=\"{\"steps\":[{\"name\":\"x\"}]}\"\n");
        assert!(out.is_some());
    }

    #[test]
    fn fomod_plus_rejects_empty_and_brace_forms() {
        assert!(run_meta_ini("[Settings]\nfomod plus/fomod=\n").is_none());
        assert!(run_meta_ini("[Settings]\nfomod plus/fomod={}\n").is_none());
        assert!(run_meta_ini("[Settings]\nfomod plus/fomod=\"{}\"\n").is_none());
    }

    #[test]
    fn fomod_plus_requires_non_empty_steps_array() {
        assert!(run_meta_ini("[Settings]\nfomod plus/fomod={\"steps\":[]}\n").is_none());
        assert!(run_meta_ini("[Settings]\nfomod plus/fomod={\"foo\":1}\n").is_none());
    }

    #[test]
    fn fomod_plus_first_matching_key_wins() {
        // the first key matches but has no steps -> reject without considering the second, valid
        // key.
        let out = run_meta_ini(
            "[Settings]\nfomod plus/fomod={\"foo\":1}\nfomod plus/fomod={\"steps\":[{\"name\":\"x\"}]}\n",
        );
        assert!(out.is_none());
    }

    #[test]
    fn fomod_plus_10000_line_cap_boundary() {
        let mut accepted = String::from("[Settings]\n");
        for _ in 0..9998 {
            accepted.push_str("other=1\n");
        }
        accepted.push_str("fomod plus/fomod={\"steps\":[{\"name\":\"x\"}]}\n");
        assert!(
            run_meta_ini(&accepted).is_some(),
            "key on line 10000 must be parsed"
        );

        let mut rejected = String::from("[Settings]\n");
        for _ in 0..9999 {
            rejected.push_str("other=1\n");
        }
        rejected.push_str("fomod plus/fomod={\"steps\":[{\"name\":\"x\"}]}\n");
        assert!(
            run_meta_ini(&rejected).is_none(),
            "key on line 10001 must be past the cap"
        );
    }

    fn run_meta_ini_bytes(contents: &[u8]) -> Option<Value> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("salma_t12_metab_{}_{seq}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        fs::write(dir.join("meta.ini"), contents).expect("write meta.ini");
        let result = try_fomod_plus_json(&dir);
        let _ = fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn fomod_plus_ill_formed_utf8_value_is_rejected() {
        // ill-formed UTF-8 in the value is a parse failure, so a tier-1 miss. decoding the file
        // lossily would replace the bad byte with U+FFFD and accept the blob, flipping the miss
        // into a hit.
        let mut bytes = b"[Settings]\nfomod plus/fomod={\"steps\":[{\"name\":\"caf".to_vec();
        bytes.push(0xe9); // lone Latin-1 'e-acute', invalid UTF-8
        bytes.extend_from_slice(b"\"}]}\n");
        assert!(run_meta_ini_bytes(&bytes).is_none());

        let ok = "[Settings]\nfomod plus/fomod={\"steps\":[{\"name\":\"caf\u{e9}\"}]}\n";
        assert!(run_meta_ini(ok).is_some());
    }

    #[test]
    fn fomod_plus_blob_rejected_on_strict_json_grammar() {
        // the JSON grammar is strict: each of these must be rejected, taking the tier-1 miss path.
        for bad in [
            "[Settings]\nfomod plus/fomod={\"steps\":[{\"name\":\"x\"}],\"i\":01}\n",
            "[Settings]\nfomod plus/fomod={\"steps\":[{\"name\":\"a\tb\"}]}\n",
        ] {
            assert!(run_meta_ini(bad).is_none(), "expected reject for {bad:?}");
        }
    }

    #[test]
    fn fomod_plus_deeply_nested_blob_is_rejected_not_a_stack_overflow() {
        // without the parser depth cap this input kills the host process: a windows stack overflow
        // is an SEH exception, not a rust panic, so capi's catch_unwind cannot contain it.
        let deep = format!(
            "[Settings]\nfomod plus/fomod={{\"steps\":[{}1{}]}}\n",
            "[".repeat(50_000),
            "]".repeat(50_000)
        );
        assert!(run_meta_ini(&deep).is_none());
    }

    #[test]
    fn try_fomod_plus_json_missing_meta_ini_is_none() {
        let dir = std::env::temp_dir().join(format!(
            "salma_t12_nometa_{}_{}",
            std::process::id(),
            TEMP_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        assert!(try_fomod_plus_json(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    fn run_tier1(blob: &Value) -> Tier1Outcome {
        let installer = FomodInstaller {
            steps: vec![step_with_plugins("S", &["P"])],
            ..FomodInstaller::default()
        };
        let overrides = InferenceOverrides {
            conditional_active: Vec::new(),
            step_visible: vec![ExternalConditionOverride::Unknown],
        };
        try_tier1_cache(
            blob,
            &installer,
            &ExpandedAtoms::default(),
            &TargetTree::new(),
            &HashSet::new(),
            &overrides,
            0,
        )
    }

    fn blob_with_step(step: Value) -> Value {
        let mut steps = Value::array();
        steps.push(step);
        let mut blob = Value::object();
        blob.insert("steps", steps);
        blob
    }

    #[test]
    fn tier1_non_string_step_name_aborts_the_whole_call() {
        // a `name` key present but not a string is malformed, and malformed fails the whole call:
        // infer_selections returns "". substituting "" and falling through would emit a full
        // inference document instead.
        let mut step = Value::object();
        step.insert("name", Value::Int(123));
        assert!(matches!(
            run_tier1(&blob_with_step(step)),
            Tier1Outcome::Abort
        ));

        let mut null_named = Value::object();
        null_named.insert("name", Value::Null);
        assert!(matches!(
            run_tier1(&blob_with_step(null_named)),
            Tier1Outcome::Abort
        ));
    }

    #[test]
    fn tier1_non_object_step_aborts_the_whole_call() {
        assert!(matches!(
            run_tier1(&blob_with_step(Value::string("S"))),
            Tier1Outcome::Abort
        ));
        assert!(matches!(
            run_tier1(&blob_with_step(Value::Int(5))),
            Tier1Outcome::Abort
        ));
    }

    #[test]
    fn tier1_non_string_group_name_aborts_the_whole_call() {
        let mut group = Value::object();
        group.insert("name", Value::Bool(true));
        let mut groups = Value::array();
        groups.push(group);
        let mut step = Value::object();
        step.insert("name", Value::string("S"));
        step.insert("groups", groups);
        assert!(matches!(
            run_tier1(&blob_with_step(step)),
            Tier1Outcome::Abort
        ));
    }

    #[test]
    fn tier1_missing_name_key_is_a_plain_miss_not_an_abort() {
        // a missing key is not malformed: it reads as "", which is an ordinary stale-cache miss
        // because no installer step is named "".
        let mut step = Value::object();
        step.insert("groups", Value::array());
        assert!(run_tier1(&blob_with_step(step)).is_miss());

        let mut stale = Value::object();
        stale.insert("name", Value::string("NoSuchStep"));
        assert!(run_tier1(&blob_with_step(stale)).is_miss());
    }

    #[test]
    fn scan_installed_files_recurses_and_normalizes() {
        let dir = std::env::temp_dir().join(format!(
            "salma_t12_scan_{}_{}",
            std::process::id(),
            TEMP_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(dir.join("Sub/Deep")).expect("mkdir");
        fs::write(dir.join("Top.esp"), b"abc").expect("write");
        fs::write(dir.join("Sub/Mid.pex"), b"de").expect("write");
        fs::write(dir.join("Sub/Deep/Leaf.dds"), b"f").expect("write");

        let files = scan_installed_files(&dir);
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(files.get("top.esp"), Some(&3));
        assert_eq!(files.get("sub/mid.pex"), Some(&2));
        assert_eq!(files.get("sub/deep/leaf.dds"), Some(&1));
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn scan_installed_files_missing_dir_is_empty() {
        let dir = std::env::temp_dir().join(format!(
            "salma_t12_absent_{}_{}",
            std::process::id(),
            TEMP_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        assert!(scan_installed_files(&dir).is_empty());
    }
}
