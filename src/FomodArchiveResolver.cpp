#include "FomodArchiveResolver.h"

#include <cstdlib>
#include <vector>

namespace fs = std::filesystem;

namespace mo2core
{

fs::path resolve_mod_archive(const std::string& archive_value,
                             const fs::path& mod_folder,
                             const fs::path& mods_dir)
{
    if (archive_value.empty())
    {
        return {};
    }

    fs::path archive_path = fs::path(archive_value);
    if (archive_path.is_absolute())
    {
        return fs::exists(archive_path) ? archive_path : fs::path{};
    }

    std::vector<fs::path> candidates;

    if (const char* downloads = std::getenv("SALMA_DOWNLOADS_PATH"); downloads && *downloads)
    {
        candidates.push_back(fs::path(downloads) / archive_path);
    }

    // Extra fallbacks for common local setups.
    candidates.push_back(mod_folder / archive_path);
    if (!mods_dir.empty())
    {
        auto parent = mods_dir.parent_path();
        candidates.push_back(parent / archive_path);
        candidates.push_back(parent / "downloads" / archive_path);
        if (parent.has_parent_path())
        {
            candidates.push_back(parent.parent_path() / "downloads" / archive_path);
        }
    }

    for (const auto& candidate : candidates)
    {
        if (fs::exists(candidate))
        {
            return candidate;
        }
    }

    return {};
}

}  // namespace mo2core
