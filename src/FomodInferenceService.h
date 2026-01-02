#pragma once

#include "Export.h"

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
 * 1. **Tier-1 shortcut**: Check meta.ini for cached fomod-plus JSON
 * 2. **List archive entries** with sizes (header-only scan)
 * 3. **Parse ModuleConfig.xml** into a typed IR
 * 4. **Expand atoms**: resolve folder entries into individual file
 *    atoms with content hashes via the archive
 * 5. **Build target tree**: scan installed files, hash contested ones
 * 6. **CSP solve**: backtracking search for selections that reproduce
 *    the installed file tree
 * 7. **Assemble JSON** from the solver result
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
     */
    std::string infer_selections(const std::string& archive_path, const std::string& mod_path);

    /// Cached content hash for an archive entry.
    struct CachedHash
    {
        uint64_t hash = 0;
        uint64_t size = 0;
    };

    /// Maximum cache entries before clearing.
    static constexpr size_t kMaxCacheEntries = 100000;

private:
    nlohmann::json try_fomod_plus_json(const std::filesystem::path& mod_path);

    static std::unordered_set<std::string> scan_installed_files(
        const std::filesystem::path& mod_path);

    /// Instance-scoped hash cache for contested archive entries.
    /// Bounded to kMaxCacheEntries; cleared when exceeded.
    std::mutex cache_mutex_;
    std::unordered_map<std::string, CachedHash> archive_entry_hash_cache_;
};

}  // namespace mo2core
