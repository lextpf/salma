#include "ModStructureDetector.h"
#include <format>
#include "Logger.h"

namespace fs = std::filesystem;

namespace mo2core
{

bool ModStructureDetector::has_mod_structure(const fs::path& dir)
{
    // Case-insensitive on NTFS (the default Windows filesystem), so
    // "meshes", "Meshes", and "MESHES" all match via fs::exists.
    static const char* mod_folders[] = {
        "SKSE",
        "meshes",
        "textures",
        "interface",
        "sound",
        "scripts",
        "seq",
        "F4SE",
        "SFSE",
        "OBSE",
        "materials",
    };
    for (const auto& folder : mod_folders)
    {
        if (fs::exists(dir / folder))
            return true;
    }
    return false;
}

std::vector<fs::path> ModStructureDetector::find_main_mod_folders(const fs::path& archive_root)
{
    // Only scans one level deep - does not recurse into nested subdirectories.
    std::vector<fs::path> results;
    auto& logger = Logger::instance();

    try
    {
        for (const auto& entry : fs::directory_iterator(archive_root))
        {
            if (!entry.is_directory())
                continue;
            if (has_mod_structure(entry.path()))
            {
                results.push_back(entry.path());
                logger.log(std::format("[install]    candidate mod folder: \"{}\"",
                                       entry.path().filename().string()));
            }
        }
    }
    catch (const fs::filesystem_error& e)
    {
        logger.log_warning(
            std::format("[install] Cannot scan \"{}\": {}", archive_root.string(), e.what()));
    }
    return results;
}

}  // namespace mo2core
