#pragma once

#include "ArchiveService.h"
#include "Export.h"
#include "FomodAtom.h"
#include "FomodCSPSolver.h"
#include "FomodIR.h"
#include "FomodPropagator.h"

#include <cstdint>
#include <filesystem>
#include <mutex>
#include <string>
#include <unordered_map>
#include <unordered_set>

#include <nlohmann/json.hpp>

namespace mo2core
{

/**
 * @class FomodInferenceService
 * @brief Infers FOMOD selections from previously installed files.
 * @author Alex (https://github.com/lextpf)
 *
 * Given a mod archive containing a FOMOD `ModuleConfig.xml` and a
 * directory of already-installed files, this service reverse-engineers
 * which FOMOD options were originally selected using a constraint-
 * satisfaction solver.
 *
 * ## Pipeline
 *
 * 1. **Tier-1 shortcut**: Check meta.ini for cached fomod-plus JSON.
 *    "fomod-plus" refers to a MO2 plugin that caches the user's FOMOD
 *    selections as JSON in the mod's `meta.ini`. The implementation
 *    scans `meta.ini` line-by-line and looks up the key
 *    `fomod plus/fomod` (case-insensitive) **inside the `[Settings]`
 *    section** -- keys outside that section are ignored even if they
 *    match the name. When the parsed JSON value contains a non-empty
 *    `steps` array, inference is skipped entirely and the cached
 *    selections are returned as-is.
 * 2. **List archive entries** with sizes (header-only scan)
 * 3. **Parse ModuleConfig.xml** into a typed IR
 * 4. **Expand atoms**: resolve folder entries into individual file
 *    atoms with content hashes via the archive
 * 5. **Build target tree**: scan installed files, hash contested ones
 * 6. **CSP solve**: backtracking search for selections that reproduce
 *    the installed file tree
 * 7. **Assemble JSON** from the solver result
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * flowchart TD
 *     T1[1. Tier-1 shortcut: meta.ini fomod-plus]:::cache
 *     L[2. List archive entries with sizes]:::step
 *     P[3. Parse ModuleConfig.xml -> IR]:::step
 *     A[4. Expand atoms + content hashes]:::step
 *     B[5. Build target tree, hash contested]:::step
 *     S[6. CSP solve: backtracking search]:::solve
 *     J[7. Assemble JSON]:::out
 *
 *     T1 -->|hit| J
 *     T1 -->|miss| L --> P --> A --> B --> S --> J
 *     classDef cache fill:#134e3a,stroke:#10b981,color:#e2e8f0
 *     classDef step fill:#1e3a5f,stroke:#3b82f6,color:#e2e8f0
 *     classDef solve fill:#2e1f5e,stroke:#8b5cf6,color:#e2e8f0
 *     classDef out fill:transparent,stroke:#94a3b8,color:#e2e8f0,stroke-dasharray:6 4
 * ```
 *
 * ## Thread Safety
 *
 * The `infer_selections` method is safe to call concurrently from
 * multiple threads. Each invocation creates its own `ArchiveService`,
 * `InferenceContext`, and other pipeline-local objects, so there is no
 * shared mutable state apart from the content hash cache. The cache
 * (`archive_entry_hash_cache_`) is protected by `cache_mutex_`, so
 * concurrent reads and writes are properly serialized.
 *
 * ## Content Hash Cache
 *
 * The instance-scoped `archive_entry_hash_cache_` stores FNV-1a
 * content hashes for archive entries that have been read during
 * contested-file resolution. This avoids re-extracting and re-hashing
 * the same archive entry across multiple inference calls against the
 * same archive.
 *
 * - **Cache key**: an archive signature (canonical path, uncompressed
 *   file size, and last-write time as the raw integer count from
 *   `time_since_epoch().count()`) combined with the entry path,
 *   formatted as `"<canonical_path>|<size>|<mtime_ticks>\n<entry_path>"`.
 *   The signature self-invalidates when **either** size or mtime
 *   changes; a same-mtime same-size mutation would not be detected
 *   (vanishingly unlikely in practice but worth noting for strict
 *   integrity work).
 * - **Bound**: limited to `kMaxCacheEntries` (100,000) entries. When
 *   exceeded, the entire cache is cleared under the lock to prevent
 *   unbounded memory growth in long-running server processes.
 * - **Thread safety**: all reads and writes are guarded by
 *   `cache_mutex_` using `std::scoped_lock`.
 *
 * @see FomodService, FomodCSPSolver
 */
class MO2_API FomodInferenceService
{
public:
    /**
     * @brief Infer FOMOD selections from installed files vs archive FOMOD XML.
     *
     * @param archive_path Path to the mod archive.
     * @param mod_path Path to the installed mod directory.
     * @return JSON selections string, or empty string on failure.
     * @throw Does not throw. All exceptions from the inference pipeline
     *        (archive I/O, XML parsing, solver) are caught internally, logged,
     *        and result in an empty string return.
     */
    std::string infer_selections(const std::string& archive_path, const std::string& mod_path);

    /** Cached content hash for an archive entry. */
    struct CachedHash
    {
        uint64_t hash = 0; /**< FNV-1a content hash of the archive entry's data. */
        uint64_t size = 0; /**< Uncompressed size of the archive entry in bytes. */
    };

    /**
     * @brief Upper bound on cache entries before the entire cache is cleared.
     *
     * Prevents unbounded memory growth in long-running server processes.
     * When exceeded, the cache is cleared under the lock so that subsequent
     * calls re-populate only the entries they need.
     */
    static constexpr size_t kMaxCacheEntries = 100000;

    /**
     * @brief Pre-compute conditional and step-visibility overrides for inference.
     *
     * Step visibility uses a STEP-UNIQUE evidence rule: a step is forced
     * visible only if at least one of its plugins' atoms targets a dest in
     * the target tree that no other step's plugins also target. This keeps
     * mutually-exclusive conditional steps that share dest paths as Unknown
     * so the simulator can decide visibility from accumulated flags.
     *
     * Public for testability. Not part of the C-API surface.
     */
    static InferenceOverrides compute_overrides(const FomodInstaller& installer,
                                                const ExpandedAtoms& atoms,
                                                const AtomIndex& atom_index,
                                                const TargetTree& target,
                                                const std::unordered_set<std::string>& excluded);

private:
    /**
     * Bundled pipeline state for infer_selections. The grouping
     * comments name the pipeline phase from the class-level diagram
     * each group corresponds to; some phases produce multiple fields.
     */
    struct InferenceContext
    {
        // List archive entries (pipeline step 2)
        ArchiveService::EntryListing listing; /**< Archive entry list with sizes */
        std::vector<std::string>
            sorted_norm_entries; /**< Lowercase/forward-slash entry paths, sorted */
        std::unordered_map<std::string, uint64_t>
            norm_entry_sizes; /**< Norm path -> uncompressed size */

        // Locate FOMOD ModuleConfig.xml (precondition for pipeline step 3)
        std::string fomod_prefix;   /**< Archive prefix containing the fomod folder */
        std::string xml_entry_norm; /**< Normalized path of ModuleConfig.xml inside the archive */

        // Parse XML into IR (pipeline step 3)
        FomodInstaller installer; /**< Parsed FOMOD installer (steps, groups, plugins) */

        // Expand atoms (pipeline step 4)
        ExpandedAtoms atoms;  /**< Per-plugin and per-conditional file atoms */
        AtomIndex atom_index; /**< dest -> atoms reverse index */
        std::unordered_set<std::string>
            excluded; /**< Dests excluded from scoring (e.g. metadata files) */

        // Build target tree (pipeline step 5)
        std::unordered_map<std::string, uint64_t> installed; /**< Installed dests with sizes */
        TargetTree target;                                   /**< Target file tree to reproduce */

        // Pre-solve inputs (feed into pipeline step 6)
        InferenceOverrides overrides;  /**< Tri-state overrides for external conditions */
        PropagationResult propagation; /**< Pre-pass narrowed plugin domains */

        // Timing diagnostics (microseconds)
        int64_t t_list = 0;  /**< Elapsed time in archive listing (us) */
        int64_t t_scan = 0;  /**< Elapsed time in installed-file scan (us) */
        int64_t t_solve = 0; /**< Elapsed time in CSP solve (us) */
    };

    nlohmann::json try_fomod_plus_json(const std::filesystem::path& mod_path);

    static std::unordered_map<std::string, uint64_t> scan_installed_files(
        const std::filesystem::path& mod_path);

    void hash_contested_files(TargetTree& target,
                              ExpandedAtoms& atoms,
                              AtomIndex& atom_index,
                              const std::filesystem::path& mod_path,
                              const std::string& archive_path,
                              const std::unordered_set<std::string>& excluded);

    /**
     * Instance-scoped hash cache for contested archive entries.
     * Bounded to kMaxCacheEntries; cleared when exceeded.
     */
    std::mutex cache_mutex_;
    std::unordered_map<std::string, CachedHash> archive_entry_hash_cache_;
};

}  // namespace mo2core
