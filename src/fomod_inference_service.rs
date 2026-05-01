//! Inference orchestration: the single entry point that recovers which FOMOD
//! options were originally selected.
//!
//! [`FomodInferenceService::infer_selections`] compares an archive's FOMOD
//! options against an already-installed mod and returns schema-v2 JSON. Every
//! other inference module is a stage it drives. Four stages have no other home
//! and live here: the installed-file scan ([`scan_installed_files`]), lazy
//! hashing of contested files over an instance-scoped cache
//! ([`FomodInferenceService::hash_contested_files`]), the Tier-1 `meta.ini`
//! fomod-plus shortcut ([`try_fomod_plus_json`] and [`try_tier1_cache`]), and
//! the pre-solve override computation ([`compute_overrides`]).
//!
//! ## Pipeline, and every empty-string exit
//!
//! ```text
//!   0/9  banner
//!          |-- archive path missing --------------------------> ""
//!          |-- mod path missing ------------------------------> ""
//!        try_fomod_plus_json(meta.ini) -> Option<Value>   candidate only
//!   1/9  list_entries_with_sizes                          (t_list)
//!   2/9  find fomod/moduleconfig.xml, shallowest path wins
//!          |-- no candidate entry ----------------------------> ""
//!   3/9  read_entries_batch(xml)
//!          |-- entry not readable ----------------------------> ""
//!   4/9  parse_module_config -> FomodInstaller
//!          |-- XML parse error -------------------------------> ""
//!   5/9  expand_all_atoms -> build_atom_index -> excluded dests
//!   6/9  scan_installed_files -> build_target_tree         (t_scan)
//!   7/9  hash_contested_files    mutates target + atoms + atom_index
//!   7b   compute_overrides -> diag_builder.set_step_visibility
//!        try_tier1_cache(candidate), only when a candidate exists
//!          |-- Hit   -> build_tier1_json -------------------> dump(2)
//!          |-- Abort -> malformed cached name --------------> ""
//!          '-- Miss  -> fall through
//!   7c   propagate -> diag_builder.absorb_propagation
//!   8/9  solve_fomod_csp                                   (t_solve)
//!   9/9  assemble_json + add_output_tree + add_repro_detail -> dump(2)
//! ```
//!
//! The numbered labels are the ones the log lines emit. Tier-1 validation
//! carries no letter on purpose: it runs between the 7b overrides and the 7c
//! propagate, and a letter there would put the log vocabulary out of order. It
//! sits at that point because it needs the overrides to simulate with, and
//! because short-circuiting before the expensive propagate and solve is the
//! whole point of the shortcut.
//!
//! Tier 1 is a candidate, never a result. The cached blob is name-resolved
//! against the parsed IR, forward-simulated with the same atoms and overrides
//! the solver path uses, and discarded unless it reproduces the installed tree
//! exactly.
//!
//! ## Any failure returns an empty string
//!
//! Six points return `String::new()`, each marked in the diagram above. No
//! `Result` crosses the FFI boundary, and the contract covers error conditions
//! only: `capi::inferFomodSelections` supplies the panic firewall via `guard()`.
//!
//! The [`Tier1Outcome::Abort`] exit is the least obvious of the six, because
//! Tier 1 otherwise reads like an optional fast path. A cached fomod-plus blob
//! whose step or group is not an object, or whose `name` key is present but not
//! a string, fails the whole call: it yields `""` rather than falling through to
//! propagate and solve. A Tier-1 miss is a different outcome and does fall
//! through ([`Tier1Outcome::Miss`]).
//!
//! ## Propagation feeds the solver; `fully_resolved` is informational
//!
//! The service never branches on `propagation.fully_resolved`. It always calls
//! `solve_fomod_csp`. The propagation result is read at three service-level
//! sites:
//!
//! 1. The solver argument: `Some(&propagation)`, or `None` when
//!    `propagation.resolved_groups.is_empty()`.
//! 2. The Step-7c log line, which reads `resolved_groups.len()` and
//!    `fully_resolved`.
//! 3. `diag_builder.absorb_propagation(&propagation)`, which copies per-group
//!    `resolved_by` plus per-plugin reason codes and details into the
//!    diagnostics, and therefore into the emitted schema-v2 JSON. Removing the
//!    propagate call would change the output document, not just the solver seed.
//!
//! `diag_builder.finalize(&result, &propagation, &installer)` is not a fourth
//! use: its parameter is named `_propagation` and the body never reads it.
//!
//! Inside the solver the propagation result is consumed twice, and neither
//! consumer is a skip. [`crate::fomod_csp_options::get_options_for_group`] drops
//! any raw option that selects a plugin pruned from `narrowed_domains`, which is
//! how a fully-resolved group collapses to a single option instead of being
//! bypassed by a branch. `solve_fomod_csp` writes an empty `phase_per_group`
//! entry for every group in `resolved_groups`, so diagnostics attribute those
//! groups to propagation rather than to a CSP phase.
//!
//! `PropagationResult::fully_resolved` itself has no consumer anywhere in the
//! crate: the propagator computes it and it appears only in log lines.

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

/// Entry count above which the whole hash cache is cleared.
pub const K_MAX_CACHE_ENTRIES: usize = 100_000;

/// Largest installed file (256 MiB) read into memory for hashing. Anything
/// bigger keeps its scanned size and a zero hash.
const K_MAX_HASH_FILE_SIZE: u64 = 256 * 1024 * 1024;

/// Cached FNV-1a content hash and uncompressed size for one archive entry.
#[derive(Debug, Clone, Copy, Default)]
pub struct CachedHash {
    /// FNV-1a content hash of the archive entry's data.
    pub hash: u64,
    /// Uncompressed size of the archive entry in bytes.
    pub size: u64,
}

/// Reverse-engineers which FOMOD options were originally selected.
///
/// `capi::inferFomodSelections` builds a fresh instance per call, so on the DLL
/// path the hash cache always starts empty and the [`K_MAX_CACHE_ENTRIES`] cap
/// never fires there. Only a caller that reuses one instance across calls, such
/// as a test or direct library use, sees the cache retain anything.
#[derive(Debug, Default)]
pub struct FomodInferenceService {
    /// Instance-scoped hash cache for contested archive entries, keyed by
    /// `"<archive_signature>\n<entry_path>"`.
    ///
    /// Soft cap only. At the top of every
    /// [`FomodInferenceService::hash_contested_files`] call the map is cleared
    /// wholesale if it already holds more than [`K_MAX_CACHE_ENTRIES`] entries.
    /// Inserts made later in that same call are not capped, so the map can end a
    /// call above the cap. There is no insertion-time eviction and no LRU.
    ///
    /// The lock is taken up to three times per call and is never held across
    /// archive I/O.
    cache: Mutex<HashMap<String, CachedHash>>,
}

impl FomodInferenceService {
    /// Construct a service with an empty hash cache.
    pub fn new() -> Self {
        FomodInferenceService::default()
    }

    /// Infer FOMOD selections by comparing the archive's FOMOD XML against the
    /// installed files.
    ///
    /// Returns schema-v2 JSON (`dump(2)`) on success, or an empty string on any
    /// of six failures: archive not found, mod not found, not a FOMOD, the XML
    /// entry unreadable, an XML parse error, or a malformed Tier-1 cache blob.
    /// The guarantee covers error conditions, not panics; `capi::guard` contains
    /// those.
    ///
    /// `archive_path` and `mod_path` are host filesystem paths in the platform's
    /// own separator form, and both must exist. The call logs its banner first,
    /// then checks both paths and returns `""` if either is missing.
    ///
    /// # Cost and I/O
    ///
    /// Synchronous, blocking, and the most expensive entry point in the crate.
    /// One call can take minutes. It:
    ///
    /// - lists the archive (`archive_service::list_entries_with_sizes`);
    /// - reads and parses `fomod/ModuleConfig.xml` out of the archive;
    /// - batch-reads archive entries for contested destinations and FNV-1a
    ///   hashes them in memory;
    /// - walks the whole `mod_path` tree recursively ([`scan_installed_files`]);
    /// - reads entire installed files into memory to hash them, skipping any
    ///   file larger than `K_MAX_HASH_FILE_SIZE` (256 MiB);
    /// - runs a CSP search bounded by `CONFIG.time_limit_seconds`, which
    ///   defaults to 600 seconds.
    ///
    /// It writes no file except the log, and runs entirely on the calling
    /// thread.
    pub fn infer_selections(&self, archive_path: &str, mod_path: &str) -> String {
        let t_total = Instant::now();
        let logger = Logger::instance();

        // The banner reports the archive extension with its dot, and the size in
        // MB to one decimal. An unreadable size is reported as 0.0 rather than
        // failing the call.
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

        // Both paths must exist. Either miss ends the call with an empty string.
        if !Path::new(archive_path).exists() {
            logger.log_error(&format!("[infer] Archive not found: {archive_path}"));
            return String::new();
        }
        if !Path::new(mod_path).exists() {
            logger.log_error(&format!("[infer] Mod path not found: {mod_path}"));
            return String::new();
        }

        // Read the Tier-1 fomod-plus blob now, but only as a candidate. Reading
        // never fails the call: its own errors collapse to `None`.
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

        // Every failure below returns an empty string early; no Result crosses
        // the FFI boundary.
        let archive_service = ArchiveService::new();

        // Step 1: List archive entries with sizes. Each `t_*` below measures its
        // own stage only; measuring from `t_total` would report cumulative
        // elapsed time in `diagnostics.timings_ms`.
        logger.log("[infer] 1/9 Listing archive entries");
        let t_step = Instant::now();
        let listing = archive_service.list_entries_with_sizes(archive_path);
        let t_list = t_step.elapsed().as_millis() as i64;
        logger.log(&format!(
            "[infer] Step 1 list_entries: {} entries, {} sizes ({t_list}ms)",
            listing.paths.len(),
            listing.sizes.len()
        ));

        // Build the sorted normalized entry index and the normalized sizes map.
        // The size lookup uses the original entry path against a map keyed by
        // the normalized path, so a size propagates only for an entry whose raw
        // path is already lowercase and forward-slashed; every other atom keeps
        // file_size 0. That under-population is deliberate, not a typo: sizes
        // feed contested-file detection and the solver's evidence scores, so
        // changing the lookup key changes what this function infers. See
        // PARITY-NOTES.md.
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

        // Step 2: Find the FOMOD ModuleConfig entry (prefer shallowest path).
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

        // Step 3: Read ModuleConfig.xml into memory.
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

        // Step 4: Parse XML and build the IR.
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

        // Step 5: Expand atoms.
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

        // Step 6: Build the target tree from the installed-file scan. `t_scan`
        // covers the walk and the tree build, not the walk alone.
        logger.log("[infer] 6/9 Scanning installed files");
        let t_step = Instant::now();
        let installed = scan_installed_files(Path::new(mod_path));
        let mut target = build_target_tree(&installed);
        let t_scan = t_step.elapsed().as_millis() as i64;
        logger.log(&format!(
            "[infer] Step 6 target tree: {} files ({t_scan}ms)",
            target.len()
        ));

        // Step 7: Hash contested files for disambiguation (mutates target + atoms).
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

        // Step 7b: Pre-compute conditional + step-visibility overrides.
        let overrides = compute_overrides(&installer, &atoms, &atom_index, &target, &excluded);

        // Feed step-visibility overrides into the diagnostics chain.
        for (si, mode) in overrides.step_visible.iter().enumerate() {
            match mode {
                ExternalConditionOverride::ForceTrue => diag_builder.set_step_visibility(
                    si as i32,
                    true,
                    ReasonCode::StepVisibilityForced,
                ),
                // Unreachable: compute_overrides produces only ForceTrue and
                // Unknown, so no step is ever reported as not visible and its
                // closing tally logs a `false` count that is structurally 0.
                // The arm exists for match exhaustiveness.
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

        // Tier-1 validation runs here, between the 7b overrides and the 7c
        // propagate: it needs the overrides to simulate with, and
        // short-circuiting before the expensive propagate and solve is the
        // point. It carries no step letter because 7b and 7c are fixed by the
        // log lines that emit them. Short-circuit only on an exact reproduction.
        if let Some(fp) = &fomod_plus {
            let total_ms = t_total.elapsed().as_millis() as i64;
            match try_tier1_cache(
                fp, &installer, &atoms, &target, &excluded, &overrides, total_ms,
            ) {
                Tier1Outcome::Hit(out) => return out.dump(2),
                // A malformed step or group `name` fails the whole call.
                Tier1Outcome::Abort => return String::new(),
                Tier1Outcome::Miss => {}
            }
        }

        // Step 7c: Constraint propagation pre-pass.
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

        // Step 8: CSP solve (with propagation-narrowed domains).
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

        // Step 9: Assemble JSON.
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
        // Which dests diverged, beside the counts the diagnostics already carry.
        // Classified against the same excluded set the scorer used, or the marks
        // would flag files compare_trees deliberately ignores.
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

    /// Hash contested files, in the target tree and in the atoms, so the solver
    /// can tell same-sized candidates apart.
    ///
    /// A dest is contested when more than one distinct size-compatible source
    /// can produce it. For a contested dest the archive entries are read and
    /// FNV-1a-hashed into the atoms, and the installed file is read and hashed
    /// into the target. When no dest is contested, which is the common case,
    /// nothing is read and nothing is hashed.
    ///
    /// Three phases, with what each one mutates and under which filter:
    ///
    /// ```text
    ///   phase 1  find_contested_dests(target, atom_index, excluded)
    ///              -> contested_dests   (> 1 distinct size-compatible source)
    ///              -> entries_to_read   (their archive source paths)
    ///              [empty -> return; nothing is read, nothing is hashed]
    ///
    ///   phase 2  fetch_entry_hashes(archive_path, entries_to_read)
    ///              cache hit  -> HashResult{source_hashes, source_sizes}
    ///              cache miss -> read_entries_batch -> fnv1a_hash -> cache
    ///              the cap was checked once before phase 1; these inserts
    ///              are not capped
    ///
    ///   phase 3  apply_entry_hashes
    ///              atom_index[dest] <- hash/size  only if dest is contested
    ///              ExpandedAtoms.*  <- hash/size  for every atom sharing the
    ///                                             source path, no dest filter
    ///                                             <-- asymmetry
    ///              target[dest]     <- fnv1a of the installed file, size =
    ///                                  bytes read; skipped above 256 MiB
    /// ```
    ///
    /// `atom_index` and `atoms` hold independent copies of each atom, and phase
    /// 3 filters the two differently. See [`apply_entry_hashes`] for what that
    /// asymmetry means for a reader of either copy.
    ///
    /// Mutates `target`, `atoms` and `atom_index` in place. Reads from the
    /// archive and from disk. Blocking.
    pub fn hash_contested_files(
        &self,
        target: &mut TargetTree,
        atoms: &mut ExpandedAtoms,
        atom_index: &mut AtomIndex,
        mod_path: &Path,
        archive_path: &str,
        excluded: &HashSet<String>,
    ) {
        // Bounded hash cache: clear-all when the cap is exceeded (both the check
        // and the clear under one lock, so two threads cannot double-clear).
        {
            let mut cache = self.cache.lock().unwrap();
            if cache.len() > K_MAX_CACHE_ENTRIES {
                Logger::instance().log(&format!(
                    "[infer] Hash cache exceeded {K_MAX_CACHE_ENTRIES} entries, clearing"
                ));
                cache.clear();
            }
        }

        // Phase 1: Find contested destinations.
        let (contested_dests, entries_to_read) = find_contested_dests(target, atom_index, excluded);
        if contested_dests.is_empty() {
            return;
        }

        Logger::instance().log(&format!(
            "[infer] Hashing {} contested dests ({} archive entries)",
            contested_dests.len(),
            entries_to_read.len()
        ));

        // Phase 2: Fetch entry hashes (cache lookup + archive read).
        let hashes = self.fetch_entry_hashes(archive_path, &entries_to_read);

        // Phase 3: Apply hashes to atoms and target files.
        apply_entry_hashes(
            atom_index,
            atoms,
            target,
            &contested_dests,
            &hashes,
            mod_path,
        );
    }

    /// Phase 2 of contested hashing: check the instance cache, batch-read the
    /// misses, FNV-1a-hash them, and put them in the cache.
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

        // The batch read borrows `missing_entries`, so it is still the miss
        // count here.
        Logger::instance().log(&format!(
            "[infer] Contested hash cache: hits={cache_hits}, misses={}",
            missing_entries.len()
        ));

        result
    }
}

/// Read any cached fomod-plus JSON from `<mod>/meta.ini`.
///
/// Returns `Some(json)` only when the `[Settings]` key `fomod plus/fomod`
/// (case-insensitive) holds a JSON object with a non-empty `steps` array;
/// otherwise `None`. Read and parse errors collapse to `None`, so this never
/// fails the inference call.
///
/// The INI handling has quirks the unit tests pin, and each changes which blobs
/// are accepted: a 10000-line cap; `[Settings]` gating that any later header
/// turns off again; one outer quote pair peeled, once; `""`, `{}` and `"{}"`
/// rejected; and the first matching key wins whether or not it validates.
pub fn try_fomod_plus_json(mod_path: &Path) -> Option<Value> {
    let meta_ini = mod_path.join("meta.ini");
    if !meta_ini.exists() {
        return None;
    }
    // Read raw bytes and work on them directly. The value is validated as strict
    // UTF-8 further down instead of being decoded lossily here: lossy decoding
    // turns a bad byte into U+FFFD and then parses a blob that has to be
    // rejected, which flips a Tier-1 miss into a hit.
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
        // Peel one outer quote pair (once).
        if value.len() >= 2 && value[0] == b'"' && value[value.len() - 1] == b'"' {
            value = &value[1..value.len() - 1];
        }

        if value.is_empty() || value == b"{}" || value == b"\"{}\"" {
            Logger::instance().log("[infer] fomod-plus JSON found but empty - rejecting");
            return None;
        }

        // Ill-formed UTF-8 is a parse failure like any other: log it as one and
        // take the Tier-1 miss.
        let Ok(value) = std::str::from_utf8(value) else {
            Logger::instance()
                .log("[infer] Failed to parse fomod-plus JSON: invalid UTF-8 in value");
            return None;
        };

        // First matching key wins whether it parses/validates or not.
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

/// Split raw file bytes into lines: one line per `\n`-terminated segment, plus a
/// final unterminated segment only when it is non-empty. An empty file yields no
/// lines at all.
///
/// `bytes.split(b'\n')` alone would add a spurious trailing empty line for the
/// usual newline-terminated file and shift the 10000-line cap by one. The `\r`
/// of a CRLF pair is left in place; the caller trims it.
fn getline_split(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let trimmed = match bytes.last() {
        Some(b'\n') => &bytes[..bytes.len() - 1],
        _ => bytes,
    };
    let empty = trimmed.is_empty() && bytes.is_empty();
    trimmed.split(|&b| b == b'\n').skip(usize::from(empty))
}

/// Trim every leading and trailing byte contained in `set`.
///
/// Works on bytes, not code points, and takes an explicit set. `slice::trim_ascii`
/// is not a substitute: it would also strip form feed.
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

/// Pre-compute conditional and step-visibility overrides for inference.
///
/// - `conditional_active[ci]` becomes `ForceTrue` when some non-excluded,
///   in-target dest is produced only by conditional atoms and its
///   conditional-index producer set is exactly `{ci}`; otherwise `Unknown`.
/// - `step_visible[si]` becomes `ForceTrue` when the step has a non-excluded,
///   in-target dest reached by its own plugins' atoms that no other step
///   reaches; otherwise `Unknown`. The flat per-plugin walk here must stay in
///   the same order as `expand_all_atoms`' per-plugin index.
///
/// Only `ForceTrue` and `Unknown` are ever produced, so no caller can observe a
/// `ForceFalse` step or conditional.
///
/// # Panics
///
/// Panics if `atoms.per_conditional.len() > installer.conditional_patterns.len()`.
/// The conditional pass iterates `atoms.per_conditional` but writes
/// `overrides.conditional_active`, which is sized from
/// `installer.conditional_patterns`, so `ci` must be valid in both.
/// [`crate::fomod_inference_atoms::expand_all_atoms`] derives the two from the
/// same IR, so the production path always satisfies this; only a caller that
/// builds [`ExpandedAtoms`] and [`FomodInstaller`] independently, such as a test
/// or a fixture, can trip it.
///
/// The per-plugin pass is deliberately more forgiving on the other axis: a
/// `flat_idx` beyond `atoms.per_plugin` is skipped with an "IR/atom desync"
/// warning instead of panicking.
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

    // cond_only_dest_patterns[dest] = the conditional-index producer set for
    // dests reached only by conditional atoms.
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
        // Create the entry even when it stays empty; the loop below skips an
        // empty producer set explicitly.
        let producers = cond_only_dest_patterns.entry(dest.clone()).or_default();
        for atom in atoms_for_dest {
            if atom.conditional_index >= 0 {
                producers.insert(atom.conditional_index);
            }
        }
    }

    // Both counters exist only to be logged below.
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

    // Step visibility via step-unique evidence: ForceTrue if and only if a step
    // has at least one target-hit dest reached by its own plugins that no other
    // step reaches.
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

    // Closing tallies over both override vectors; log-only.
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

/// Recursively scan a mod directory into `dest -> size`.
///
/// Tolerates permission errors, includes regular files only, keys by
/// `normalize_path(relative(path, mod_root))` (lowercase, forward slashes), and
/// records the file size, or 0 when the size cannot be read. No name is skipped
/// here; `build_target_tree` later drops the top-level `meta.ini`.
///
/// Symlinks are treated differently by kind, deliberately: the walk descends
/// real directories only (`file_type().is_dir()`, so a directory symlink is not
/// followed) but includes any entry that resolves to a regular file
/// (`path.is_file()`, so a file symlink is followed). See `PARITY-NOTES.md`.
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
                // Tolerate permission-denied and transient errors. The walk is
                // per-directory, so the warning names the directory that
                // failed rather than the mod root.
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

/// Hashes/sizes for archive entries read during contested-file resolution.
#[derive(Debug, Default)]
struct HashResult {
    source_hashes: HashMap<String, u64>,
    source_sizes: HashMap<String, u64>,
}

/// Phase 1 of contested hashing: find the dests that more than one distinct
/// size-compatible source can produce. Returns `(contested_dests,
/// entries_to_read)`, where `entries_to_read` holds their archive source paths.
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

/// Build the hash-cache key prefix `<canonical_path>|<size>|<mtime>` for one
/// archive.
///
/// `mtime` is nanoseconds since the Unix epoch, and the path falls back to the
/// raw path when it cannot be canonicalized. The only property that matters is
/// that the signature changes when the archive's size or mtime changes, so the
/// cache invalidates itself. The exact encoding is free to change: the signature
/// never leaves this process and never reaches the output document.
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

/// Phase 3 of contested hashing: write the fetched hashes onto the atoms, then
/// hash the installed file at each contested dest into the target.
///
/// The two atom writes use different filters, and the difference is observable:
///
/// - The [`AtomIndex`] copies are updated only for atoms filed under a dest in
///   `contested_dests`. Every other dest bucket is skipped.
/// - The [`ExpandedAtoms`] copies are updated for every atom in `required`,
///   `per_plugin` and `per_conditional` whose `source_path` has a fetched hash.
///   There is no dest filter: `ExpandedAtoms::for_each_mut` visits all of them.
///
/// The two structures hold independent `FomodAtom` copies, so after this call an
/// atom whose own dest is not contested but whose `source_path` was fetched for
/// some other contested dest carries the new `content_hash` and `file_size` in
/// [`ExpandedAtoms`] and the old values in [`AtomIndex`]. The old `file_size` is
/// usually 0, because the normalized size map is deliberately under-populated
/// (see the note on it in [`FomodInferenceService::infer_selections`]), so the
/// divergence is typically 0 against a real size.
///
/// That matters because the two views have different readers.
/// [`crate::fomod_csp_precompute::compute_evidence`] reads `file_size` and
/// `content_hash` out of [`AtomIndex`], while
/// [`crate::fomod_forward_simulator::simulate`] reads [`ExpandedAtoms`]. The
/// propagator is unaffected: it accepts `atom_index` and never reads it.
///
/// Do not tidy the two filters into symmetry on sight. Changing either one
/// changes the evidence scores the solver ranks plugins with, and
/// `PARITY-NOTES.md` has no entry for this asymmetry, so nothing on record says
/// which side is the intended behavior. Establish that first.
///
/// An installed file above 256 MiB (`K_MAX_HASH_FILE_SIZE`) is left unhashed and
/// logged; its target entry keeps its scanned size and a zero hash.
fn apply_entry_hashes(
    atom_index: &mut AtomIndex,
    atoms: &mut ExpandedAtoms,
    target: &mut TargetTree,
    contested_dests: &HashSet<String>,
    hashes: &HashResult,
    mod_path: &Path,
) {
    // Update atoms in the index with hashes.
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

    // Update atoms in the ExpandedAtoms struct.
    atoms.for_each_mut(|a| {
        if let Some(&h) = hashes.source_hashes.get(&a.source_path) {
            a.content_hash = h;
            if let Some(&sz) = hashes.source_sizes.get(&a.source_path) {
                a.file_size = sz;
            }
        }
    });

    // Hash installed files at contested dests.
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
        // The whole file is read into memory to hash it, which is why the size
        // cap above exists. A read error leaves the target entry untouched.
        let Ok(buf) = fs::read(&full_path) else {
            continue;
        };
        tf.hash = fnv1a_hash(&buf);
        tf.size = buf.len() as u64;
    }
}

/// Outcome of validating the Tier-1 fomod-plus candidate.
#[derive(Debug)]
pub enum Tier1Outcome {
    /// The cached selection resolved and reproduced the target tree exactly.
    /// Carries the bespoke schema-v2 document to return.
    Hit(Box<Value>),
    /// The candidate is unusable: stale names, or it does not reproduce the
    /// tree. The caller falls through to propagate and solve.
    Miss,
    /// The cached blob is malformed: a step or group that is not an object, or
    /// whose `name` key is present but not a string.
    ///
    /// This fails the whole inference call, which returns `""`. Do not soften it
    /// into a [`Tier1Outcome::Miss`]: coercing the name to `""` and falling
    /// through would emit a full inference document for a blob the contract says
    /// to reject.
    Abort,
}

impl Tier1Outcome {
    /// True when the cached selection reproduced the target tree exactly.
    pub fn is_hit(&self) -> bool {
        matches!(self, Tier1Outcome::Hit(_))
    }

    /// True when the caller should fall through to propagate + solve.
    pub fn is_miss(&self) -> bool {
        matches!(self, Tier1Outcome::Miss)
    }

    /// Consume into the emitted schema-v2 document. `None` for anything but a
    /// [`Tier1Outcome::Hit`].
    pub fn hit(self) -> Option<Value> {
        match self {
            Tier1Outcome::Hit(value) => Some(*value),
            _ => None,
        }
    }
}

/// Read a `name` field, keeping "absent" and "wrong type" apart.
///
/// - not an object                  -> `None`
/// - object, no `name`              -> `Some("")`
/// - object, `name` is a string     -> `Some(s)`
/// - object, `name` is not a string -> `None`
///
/// `None` is the malformed case and makes [`try_tier1_cache`] abort the whole
/// inference call. `Some("")` is an ordinary miss, because no step or group in
/// the IR is named `""`.
fn name_field(src: &Value) -> Option<&str> {
    if !src.is_object() {
        return None;
    }
    match src.get("name") {
        None => Some(""),
        Some(v) => v.as_str(),
    }
}

/// Extract a plugin name from a cached JSON entry: the string itself, or the
/// string `name` of an object. Every other shape yields `""`.
///
/// Deliberately more tolerant than [`name_field`]: a plugin entry of the wrong
/// shape is a stale-cache miss, never an abort.
fn plugin_name_of(src: &Value) -> String {
    if let Some(s) = src.as_str() {
        return s.to_string();
    }
    if let Some(name) = src.get("name").and_then(Value::as_str) {
        return name.to_string();
    }
    String::new()
}

/// Validate the Tier-1 fomod-plus candidate and, on an exact reproduction, emit
/// the bespoke schema-v2 selection.
///
/// Returns [`Tier1Outcome::Hit`] only when the cached blob name-resolves against
/// `installer` and its forward simulation reproduces `target` exactly. The
/// document it carries is built by [`build_tier1_json`], not by `assemble_json`.
/// Any name-resolution failure, or a non-exact reproduction, is a
/// [`Tier1Outcome::Miss`] and the caller falls through to the normal solve; a
/// malformed step or group `name` is a [`Tier1Outcome::Abort`]. `total_ms` is
/// the elapsed time to embed as `diagnostics.timings_ms.total`.
pub fn try_tier1_cache(
    fomod_plus: &Value,
    installer: &FomodInstaller,
    atoms: &ExpandedAtoms,
    target: &TargetTree,
    excluded: &HashSet<String>,
    overrides: &InferenceOverrides,
    total_ms: i64,
) -> Tier1Outcome {
    // Build the [step][group][plugin] grid by name-matching the cached blob.
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
    // Names the first unresolvable entry, for the "cache stale" warning only.
    let mut stale_what = String::new();
    'steps: for src_step in fomod_plus.get("steps").into_iter().flat_map(array_iter) {
        // Read the name before the lookup that can break the loop, so every
        // step up to and including the first unresolvable one is name-checked
        // and can still abort the call.
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

    // Forward-simulate the cached selection with the same atoms + overrides the
    // solver path uses, then diff against the target.
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

/// Iterate the elements of a JSON array value; empty for non-arrays.
fn array_iter(value: &Value) -> std::slice::Iter<'_, Value> {
    match value {
        Value::Array(items) => items.iter(),
        _ => [].iter(),
    }
}

/// Build the bespoke Tier-1 schema-v2 document from the cached blob and the
/// validation simulation.
///
/// Only reached once [`try_tier1_cache`] has proved the blob name-resolves and
/// reproduces the target exactly, so every confidence value it writes is 1.0 and
/// every group is attributed to `cache.fomod_plus`.
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
        // `name_field` cannot be None here: this emitter only runs after
        // try_tier1_cache validated every step and group it walks.
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
    // A hit means the cached selection reproduced the installed tree exactly,
    // so there is nothing to mark. The key is still emitted, empty, so a
    // consumer never has to special-case the cache path.
    add_repro_detail(&mut out, &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fomod_atom::{FomodAtom, TargetFile};
    use crate::fomod_ir::{FomodConditionalPattern, FomodGroup, FomodPlugin, FomodStep};
    use std::sync::atomic::{AtomicU64, Ordering};

    // --- test builders -----------------------------------------------------

    fn plugin(name: &str) -> FomodPlugin {
        FomodPlugin {
            name: name.to_string(),
            ..FomodPlugin::default()
        }
    }

    /// A step with `n` groups, each holding one plugin (so the flat per-plugin
    /// walk visits one index per group in order).
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

    // --- compute_overrides: conditional evidence ---------------------------

    #[test]
    fn compute_overrides_conditional_unique_dest_is_force_true() {
        // ci=0 uniquely produces "uniq0"; ci=1 shares "shared" with ci=0.
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
        // ci=0 has a conditional-only dest reached by exactly {0} -> ForceTrue.
        assert_eq!(
            ov.conditional_active[0],
            ExternalConditionOverride::ForceTrue
        );
        // ci=1 only reaches "shared" (producer set {0,1}) -> Unknown.
        assert_eq!(ov.conditional_active[1], ExternalConditionOverride::Unknown);
    }

    #[test]
    fn compute_overrides_conditional_dest_shared_with_plugin_is_unknown() {
        // "shared" is produced by a conditional and a plugin -> not
        // conditional-only -> the conditional is never forced.
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

    // --- compute_overrides: step visibility --------------------------------

    #[test]
    fn compute_overrides_step_unique_dest_is_force_true_shared_is_unknown() {
        // Step0's plugin reaches a unique dest; Step1's plugin shares its only
        // dest with Step0.
        let installer = FomodInstaller {
            steps: vec![
                step_with_plugins("S0", &["P0"]),
                step_with_plugins("S1", &["P1"]),
            ],
            ..FomodInstaller::default()
        };
        // flat_idx 0 = S0/P0, flat_idx 1 = S1/P1.
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
        // The step's only dest is excluded -> not counted -> Unknown. The
        // conditional's only dest is absent from target -> Unknown.
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
        // target has neither "ex" (excluded) nor... include "ex" so it is only
        // dropped by the excluded set, and omit "absent".
        let target = target_of(&["ex"]);
        let mut excluded = HashSet::new();
        excluded.insert("ex".to_string());

        let ov = compute_overrides(&installer, &atoms, &index, &target, &excluded);
        assert_eq!(ov.step_visible[0], ExternalConditionOverride::Unknown);
        assert_eq!(ov.conditional_active[0], ExternalConditionOverride::Unknown);
    }

    // --- try_fomod_plus_json: INI quirks -----------------------------------

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    /// Create a unique temp dir, write `meta.ini` with `contents`, run
    /// `try_fomod_plus_json`, remove the dir, and return the result.
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
        // [Settings] then [General] disables tracking before the key.
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
        // The value is wrapped in one quote pair; peeling once yields valid JSON.
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
        // The first key matches but has no steps -> reject without considering
        // the second, valid key.
        let out = run_meta_ini(
            "[Settings]\nfomod plus/fomod={\"foo\":1}\nfomod plus/fomod={\"steps\":[{\"name\":\"x\"}]}\n",
        );
        assert!(out.is_none());
    }

    #[test]
    fn fomod_plus_10000_line_cap_boundary() {
        // Key on line 10000 (line 1 = [Settings], lines 2..9999 = junk) is read.
        let mut accepted = String::from("[Settings]\n");
        for _ in 0..9998 {
            accepted.push_str("other=1\n");
        }
        accepted.push_str("fomod plus/fomod={\"steps\":[{\"name\":\"x\"}]}\n");
        assert!(
            run_meta_ini(&accepted).is_some(),
            "key on line 10000 must be parsed"
        );

        // Key on line 10001 aborts the parse (++line_count > 10000).
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

    /// `run_meta_ini` for a file whose bytes are not valid UTF-8.
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
        // Ill-formed UTF-8 in the value is a parse failure, so a Tier-1 miss.
        // Decoding the file lossily would replace the bad byte with U+FFFD and
        // accept the blob, flipping the miss into a hit.
        let mut bytes = b"[Settings]\nfomod plus/fomod={\"steps\":[{\"name\":\"caf".to_vec();
        bytes.push(0xe9); // lone Latin-1 'e-acute', invalid UTF-8
        bytes.extend_from_slice(b"\"}]}\n");
        assert!(run_meta_ini_bytes(&bytes).is_none());

        // The same text as well-formed UTF-8 is still accepted.
        let ok = "[Settings]\nfomod plus/fomod={\"steps\":[{\"name\":\"caf\u{e9}\"}]}\n";
        assert!(run_meta_ini(ok).is_some());
    }

    #[test]
    fn fomod_plus_blob_rejected_on_strict_json_grammar() {
        // The JSON grammar is strict: each of these must be rejected, taking
        // the Tier-1 miss path.
        for bad in [
            // leading zero
            "[Settings]\nfomod plus/fomod={\"steps\":[{\"name\":\"x\"}],\"i\":01}\n",
            // raw control byte (a tab) inside a string
            "[Settings]\nfomod plus/fomod={\"steps\":[{\"name\":\"a\tb\"}]}\n",
        ] {
            assert!(run_meta_ini(bad).is_none(), "expected reject for {bad:?}");
        }
    }

    #[test]
    fn fomod_plus_deeply_nested_blob_is_rejected_not_a_stack_overflow() {
        // Without the parser depth cap this input kills the host process: a
        // Windows stack overflow is an SEH exception, not a Rust panic, so
        // capi's catch_unwind cannot contain it.
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

    // --- try_tier1_cache: malformed names abort the whole call --------------

    /// Drive `try_tier1_cache` with an installer that has one step named "S" and
    /// otherwise empty inputs. Every case below decides before any simulation.
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

    /// `{"steps": [<step>]}` around one cached step value.
    fn blob_with_step(step: Value) -> Value {
        let mut steps = Value::array();
        steps.push(step);
        let mut blob = Value::object();
        blob.insert("steps", steps);
        blob
    }

    #[test]
    fn tier1_non_string_step_name_aborts_the_whole_call() {
        // A `name` key present but not a string is malformed, and malformed
        // fails the whole call: infer_selections returns "". Substituting ""
        // and falling through would emit a full inference document instead.
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
        // A step that is not an object at all is malformed the same way.
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
        // Groups are read the same way, but only for a step that resolved: the
        // group name is read after the step lookup succeeds.
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
        // A missing key is not malformed: it reads as "", which is an ordinary
        // stale-cache miss because no installer step is named "".
        let mut step = Value::object();
        step.insert("groups", Value::array());
        assert!(run_tier1(&blob_with_step(step)).is_miss());

        // ...as is a well-formed name that simply does not exist.
        let mut stale = Value::object();
        stale.insert("name", Value::string("NoSuchStep"));
        assert!(run_tier1(&blob_with_step(stale)).is_miss());
    }

    // --- scan_installed_files ----------------------------------------------

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
