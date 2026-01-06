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
 *    selections as JSON in the mod's `meta.ini` file (under the
 *    `[Settings]` key `fomod plus/fomod`). When this cached result is
 *    present and contains a non-empty `steps` array, inference is
 *    skipped entirely and the cached selections are returned as-is.
 * 2. **List archive entries** with sizes (header-only scan)
 * 3. **Parse ModuleConfig.xml** into a typed IR
 * 4. **Expand atoms**: resolve folder entries into individual file
 *    atoms with content hashes via the archive
 * 5. **Build target tree**: scan installed files, hash contested ones
 * 6. **CSP solve**: backtracking search for selections that reproduce
 *    the installed file tree
 * 7. **Assemble JSON** from the solver result
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
 * - **Cache key**: an archive signature (canonical path, file size,
 *   and last-write time) combined with the entry path, formatted as
 *   `"<canonical_path>|<size>|<mtime>\n<entry_path>"`. The signature
 *   component ensures the cache self-invalidates when the archive
 *   file changes on disk.
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
        uint64_t hash = 0;  ///< FNV-1a content hash of the archive entry's data.
        uint64_t size = 0;  ///< Uncompressed size of the archive entry in bytes.
    };

    /**
     * @brief Upper bound on cache entries before the entire cache is cleared.
     *
     * Prevents unbounded memory growth in long-running server processes.
     * When exceeded, the cache is cleared under the lock so that subsequent
     * calls re-populate only the entries they need.
     */
    static constexpr size_t kMaxCacheEntries = 100000;

private:
    /** Bundled pipeline state for infer_selections. */
    struct InferenceContext
    {
        // Step 1 outputs
        ArchiveService::EntryListing listing;
        std::vector<std::string> sorted_norm_entries;
        std::unordered_map<std::string, uint64_t> norm_entry_sizes;

        // Step 2 outputs
        std::string fomod_prefix;
        std::string xml_entry_norm;

        // Step 4 output
        FomodInstaller installer;

        // Step 5 outputs
        ExpandedAtoms atoms;
        AtomIndex atom_index;
        std::unordered_set<std::string> excluded;

        // Step 6 outputs
        std::unordered_map<std::string, uint64_t> installed;
        TargetTree target;

        // Step 7b output
        InferenceOverrides overrides;

        // Step 7c output
        PropagationResult propagation;

        // Timing
        int64_t t_list = 0;
        int64_t t_scan = 0;
        int64_t t_solve = 0;
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

    static InferenceOverrides compute_overrides(const FomodInstaller& installer,
                                                const ExpandedAtoms& atoms,
                                                const AtomIndex& atom_index,
                                                const TargetTree& target,
                                                const std::unordered_set<std::string>& excluded);

    /**
     * Instance-scoped hash cache for contested archive entries.
     * Bounded to kMaxCacheEntries; cleared when exceeded.
     */
    std::mutex cache_mutex_;
    std::unordered_map<std::string, CachedHash> archive_entry_hash_cache_;
};

}  // namespace mo2core
