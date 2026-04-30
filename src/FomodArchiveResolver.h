#pragma once

#include "Export.h"

#include <filesystem>
#include <string>

namespace mo2core
{

/**
 * @brief Resolve a mod's source archive path from its `installationFile`
 *        meta.ini value plus surrounding directories.
 *
 * Used by both the local dashboard's FOMOD scan (server-internal call) and
 * the MO2 Python plugin (via the `resolveModArchive` C-API). Centralizing
 * the resolution rules guarantees the two callers agree on which archive
 * a given mod folder maps to.
 *
 * Search order, first hit wins:
 * 1. If @p archive_value is absolute and exists, return it.
 * 2. `$SALMA_DOWNLOADS_PATH/<archive_value>`
 * 3. `<mod_folder>/<archive_value>`
 * 4. `<mods_dir>/../<archive_value>`
 * 5. `<mods_dir>/../downloads/<archive_value>`
 * 6. `<mods_dir>/../../downloads/<archive_value>`
 *
 * @param archive_value Raw value from meta.ini's `[General] installationFile`
 *                      (may be relative or absolute).
 * @param mod_folder    Absolute path to the mod folder being looked up.
 * @param mods_dir      Absolute path to the parent mods directory.
 * @return Absolute resolved path on hit, or an empty path on miss.
 */
MO2_API std::filesystem::path resolve_mod_archive(const std::string& archive_value,
                                                  const std::filesystem::path& mod_folder,
                                                  const std::filesystem::path& mods_dir);

}  // namespace mo2core
