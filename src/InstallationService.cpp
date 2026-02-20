#include "InstallationService.h"
#include "ArchiveService.h"
#include "FileOperations.h"
#include "FomodIRParser.h"
#include "FomodService.h"
#include "Logger.h"
#include "ModStructureDetector.h"
#include "Utils.h"

#include <nlohmann/json.hpp>
#include <pugixml.hpp>

#include <chrono>
#include <format>
#include <fstream>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2core
{

std::string InstallationService::install_mod(const std::string& archive_path,
                                             const std::string& mod_path,
                                             const std::string& json_path)
{
    auto& logger = Logger::instance();
    auto start = std::chrono::steady_clock::now();

    // Clear the sticky disk-full marker before any work so a previous
    // install's disk pressure does not poison this run. The flag gets
    // set by FileOperations::copy_* / move_* when std::filesystem hits
    // errc::no_space_on_device, and is checked at the bottom of this
    // function to convert a silently-truncated install into a thrown
    // error the caller can react to.
    FileOperations::reset_disk_full();

    logger.log("[install] === Starting mod installation ===");
    logger.log(std::format("[install] Archive: {}", archive_path));
    logger.log(std::format("[install] Target mod directory: {}", mod_path));
    logger.log("[install] Initialization finished");

    if (!fs::exists(archive_path))
    {
        throw std::runtime_error("Archive file not found: " + archive_path);
    }

    std::error_code size_ec;
    auto archive_size = fs::file_size(archive_path, size_ec);
    if (size_ec)
    {
        logger.log_warning(
            std::format("[install] Could not read archive size: {}", size_ec.message()));
        archive_size = 0;
    }
    logger.log(std::format("[install] Archive size: {} bytes ({:.2f} MB)",
                           archive_size,
                           archive_size / 1024.0 / 1024.0));

    fs::create_directories(mod_path);
    logger.log(std::format("[install] Created mod directory: {}", mod_path));

    auto temp_dir = fs::temp_directory_path() / ("fomod-" + random_hex_string(8));
    auto archive_extract_dir = temp_dir / "archive";
    fs::create_directories(archive_extract_dir);
    logger.log(std::format("[install] Temporary directory: {}", temp_dir.string()));

    try
    {
        // Extract archive
        logger.log("[install] Extracting archive...");
        auto extract_start = std::chrono::steady_clock::now();
        ArchiveService archive_service;
        archive_service.extract(archive_path, archive_extract_dir.string());
        auto extract_duration =
            std::chrono::duration<double>(std::chrono::steady_clock::now() - extract_start).count();
        logger.log(std::format("[install] Archive extracted to {} in {:.2f} seconds",
                               temp_dir.string(),
                               extract_duration));

        // Find FOMOD folder
        logger.log("[install] Searching for FOMOD folder...");
        auto fomod_folder = find_fomod_folder(archive_extract_dir);

        std::string result;
        if (fomod_folder.empty())
        {
            logger.log("[install] No FOMOD folder detected - using standard installation");
            result =
                handle_non_fomod_install(archive_extract_dir, mod_path, archive_path, json_path);
        }
        else
        {
            logger.log(std::format("[install] FOMOD folder found: {}", fomod_folder.string()));
            result = handle_fomod_install(fomod_folder,
                                          archive_extract_dir,
                                          mod_path,
                                          archive_path,
                                          temp_dir.string(),
                                          json_path);
        }

        // Cleanup
        try
        {
            fs::remove_all(temp_dir);
            logger.log("[install] Cleaned up temporary directory");
        }
        catch (const std::exception& ex)
        {
            logger.log_warning(
                std::format("[install] WARNING: Failed to cleanup temp directory: {}", ex.what()));
        }

        auto total_duration =
            std::chrono::duration<double>(std::chrono::steady_clock::now() - start).count();
        logger.log(
            std::format("[install] Total installation time: {:.2f} seconds", total_duration));

        if (FileOperations::disk_full_encountered())
        {
            // Some files in the archive could not be copied because the volume
            // ran out of space. Surface this as a hard install failure so the
            // round-trip test harness reports "install failed" instead of
            // "missing files" and so MO2 doesn't deploy a half-empty mod.
            throw std::runtime_error(
                "Install aborted: disk full while copying files. Free space and retry.");
        }

        return result;
    }
    catch (...)
    {
        // Cleanup on error
        try
        {
            fs::remove_all(temp_dir);
        }
        catch (...)
        {
            Logger::instance().log_warning(
                "[install] Failed to cleanup temp directory on error path");
        }
        throw;
    }
}

fs::path InstallationService::find_fomod_folder(const fs::path& archive_root)
{
    for (const auto& entry : fs::recursive_directory_iterator(archive_root))
    {
        if (!entry.is_directory() || to_lower(entry.path().filename().string()) != "fomod")
        {
            continue;
        }
        // Case-insensitive search for ModuleConfig.xml - real-world
        // FOMOD archives use varying casing (moduleconfig.xml, etc.)
        for (const auto& child : fs::directory_iterator(entry.path()))
        {
            if (child.is_regular_file() &&
                to_lower(child.path().filename().string()) == "moduleconfig.xml")
            {
                return entry.path();
            }
        }
    }
    return {};
}

std::string InstallationService::handle_non_fomod_install(const fs::path& archive_root,
                                                          const std::string& mod_path,
                                                          const std::string& archive_path,
                                                          const std::string& json_path)
{
    auto& logger = Logger::instance();
    logger.log(
        std::format("[install] No 'fomod' folder found; checking for nested mod structure in: {}",
                    archive_root.string()));

    std::string module_name_lower;

    // Determine JSON path
    std::string effective_json = resolve_json_path(json_path, archive_path);

    if (!effective_json.empty() && fs::exists(effective_json))
    {
        std::ifstream f(effective_json);
        if (f)
        {
            json config;
            try
            {
                f >> config;
            }
            catch (const json::parse_error& ex)
            {
                logger.log_warning(std::format(
                    "[install] Failed to parse JSON config {}: {}", effective_json, ex.what()));
            }
            if (config.contains("moduleName") && config["moduleName"].is_string())
            {
                module_name_lower = to_lower(config["moduleName"].get<std::string>());
                logger.log(
                    std::format("[install] Detected moduleName \"{}\" in JSON", module_name_lower));
            }
        }
    }

    if (module_name_lower.find('/') != std::string::npos ||
        module_name_lower.find('\\') != std::string::npos ||
        module_name_lower.find("..") != std::string::npos)
    {
        logger.log_warning(std::format(
            "[install] Rejecting moduleName with path separators: \"{}\"", module_name_lower));
        module_name_lower.clear();
    }

    // Reject Windows reserved device names (CON, AUX, PRN, NUL, COM1-9, LPT1-9).
    // These cannot be used as directory names on Windows and cause silent failures.
    if (!module_name_lower.empty())
    {
        static const std::unordered_set<std::string> kReservedNames = {
            "con",  "prn",  "aux",  "nul",  "com1", "com2", "com3", "com4", "com5", "com6", "com7",
            "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
        };
        auto stem = module_name_lower;
        if (auto dot = stem.rfind('.'); dot != std::string::npos)
            stem = stem.substr(0, dot);
        if (kReservedNames.contains(stem))
        {
            logger.log_warning(std::format(
                "[install] Rejecting Windows reserved device name: \"{}\"", module_name_lower));
            module_name_lower.clear();
        }
    }

    // Fallback folder paths come from ModStructureDetector which enumerates
    // filesystem entries under archive_root -- they are always relative names,
    // so no additional traversal validation is needed beyond the check above.
    auto main_mod_folders = ModStructureDetector::find_main_mod_folders(archive_root);

    if (!main_mod_folders.empty())
    {
        fs::path chosen;

        if (main_mod_folders.size() == 1)
        {
            chosen = main_mod_folders[0];
            logger.log(std::format("[install] Only one mod folder \"{}\" found; copying it",
                                   chosen.filename().string()));
        }
        else
        {
            if (module_name_lower.empty())
            {
                throw std::runtime_error(
                    "Multiple mod folders detected but no moduleName in JSON to disambiguate.");
            }

            std::vector<fs::path> matches;
            for (const auto& p : main_mod_folders)
            {
                if (to_lower(p.filename().string()) == module_name_lower)
                {
                    matches.push_back(p);
                    logger.log(std::format("[install]      matches moduleName: \"{}\"",
                                           p.filename().string()));
                }
            }

            if (matches.size() != 1)
            {
                throw std::runtime_error(
                    matches.empty()
                        ? ("moduleName '" + module_name_lower + "' did not match any folder.")
                        : ("moduleName '" + module_name_lower + "' matched multiple folders."));
            }

            chosen = matches[0];
            logger.log(std::format("[install] Copying contents of chosen mod folder \"{}\"",
                                   chosen.filename().string()));
        }

        FileOperations::copy_directory_contents(chosen, mod_path);
        return mod_path;
    }

    // Fallback: copy all from archive root
    logger.log(
        std::format("[install] No nested mod structure detected; copying all files from archive "
                    "root to mod directory: {}",
                    mod_path));
    FileOperations::copy_directory_contents(archive_root, mod_path);
    return mod_path;
}

std::string InstallationService::handle_fomod_install(const fs::path& fomod_folder,
                                                      const fs::path& archive_root,
                                                      const std::string& mod_path,
                                                      const std::string& archive_path,
                                                      const std::string& temp_dir,
                                                      const std::string& json_path)
{
    auto& logger = Logger::instance();
    auto xml_path = (fomod_folder / "ModuleConfig.xml").string();
    auto src_base = fomod_folder.parent_path().string();
    auto dst_base = (fs::path(temp_dir) / "unfomod").string();

    // Determine JSON path
    std::string effective_json = resolve_json_path(json_path, archive_path);

    fs::create_directories(dst_base);

    // Parse XML
    pugi::xml_document doc;
    auto xml_result = doc.load_file(xml_path.c_str());
    if (!xml_result)
    {
        throw std::runtime_error(std::format("Cannot parse XML ({})", xml_result.description()));
    }
    logger.log(std::format("[install] Loaded XML: {}", xml_path));

    // Parse JSON
    json config_json;
    if (!effective_json.empty() && fs::exists(effective_json))
    {
        std::ifstream f(effective_json);
        if (f)
        {
            try
            {
                f >> config_json;
            }
            catch (const json::parse_error& ex)
            {
                logger.log_warning(std::format(
                    "[install] Failed to parse FOMOD JSON {}: {}", effective_json, ex.what()));
            }
            logger.log(std::format("[install] Loaded JSON: {}", effective_json));
        }
    }

    // Create dependency context
    FomodDependencyContext context;
    context.archive_root = archive_root.string();
    context.installed_plugins.insert("skyrim.esm");
    context.installed_plugins.insert("update.esm");
    context.installed_plugins.insert("dawnguard.esm");
    context.installed_plugins.insert("hearthfires.esm");
    context.installed_plugins.insert("dragonborn.esm");

    // Read game context from JSON if present
    if (!config_json.is_null())
    {
        if (config_json.contains("gamePath") && config_json["gamePath"].is_string())
        {
            context.game_path = config_json["gamePath"].get<std::string>();
            logger.log(std::format("[install] Game path from JSON: {}", context.game_path));
        }
        if (config_json.contains("gameVersion") && config_json["gameVersion"].is_string())
        {
            context.game_version = config_json["gameVersion"].get<std::string>();
            logger.log(std::format("[install] Game version from JSON: {}", context.game_version));
        }
    }

    // Scan game Data directory for plugins (.esp/.esm/.esl)
    if (!context.game_path.empty())
    {
        auto data_dir = fs::path(context.game_path) / "Data";
        if (fs::exists(data_dir))
        {
            for (const auto& entry : fs::directory_iterator(data_dir))
            {
                if (!entry.is_regular_file())
                    continue;
                auto ext = to_lower(entry.path().extension().string());
                if (ext == ".esp" || ext == ".esm" || ext == ".esl")
                {
                    context.installed_plugins.insert(to_lower(entry.path().filename().string()));
                }
            }
            logger.log(std::format("[install] Found {} plugins in game Data directory",
                                   context.installed_plugins.size()));
        }
    }

    // Populate installed_files from existing mod directory (for re-installs)
    if (fs::exists(mod_path) && fs::is_directory(mod_path))
    {
        for (const auto& entry : fs::recursive_directory_iterator(mod_path))
        {
            if (!entry.is_regular_file())
                continue;
            auto rel = fs::relative(entry.path(), mod_path).string();
            std::replace(rel.begin(), rel.end(), '\\', '/');
            context.installed_files.insert(rel);
        }
        if (!context.installed_files.empty())
        {
            logger.log(std::format("[install] Scanned {} existing files in mod directory",
                                   context.installed_files.size()));
        }
    }

    FomodService fomod_service;
    fomod_service.set_installer(FomodIRParser::parse(doc, ""));

    // Check module-level dependencies
    logger.log("[install] Checking module-level dependencies...");
    if (!fomod_service.check_module_dependencies(&context))
    {
        throw std::runtime_error("Module-level dependencies not met - installation cannot proceed");
    }

    // Validate JSON selections
    if (!config_json.is_null())
    {
        logger.log("[install] Validating JSON selections...");
        if (!fomod_service.validate_json_selections(config_json))
        {
            logger.log_warning(
                "[install] WARNING: JSON selections have group-type constraint violations");
        }
    }

    // Caller-owned operations vector and document-order counter.
    // Each process method appends to the same vector so that
    // execute_file_operations can sort everything in one pass.
    std::vector<mo2core::FileOperation> file_ops;
    int next_doc_order = 0;

    // Process required files
    logger.log("[install] Processing required install files...");
    fomod_service.process_required_files(src_base, dst_base, file_ops, next_doc_order);

    // Process optional files and auto-install behavior.
    // Even when selections are missing, this still installs Required plugins and
    // alwaysInstall/installIfUsable entries from unselected plugins.
    logger.log("[install] Processing optional install files...");
    fomod_service.process_optional_files(
        config_json, src_base, dst_base, &context, file_ops, next_doc_order);

    // Process conditional files
    logger.log("[install] Processing conditional file installs...");
    fomod_service.process_conditional_files(src_base, dst_base, &context, file_ops, next_doc_order);

    // Execute all file operations in a single priority-sorted pass
    int file_op_failures = FomodService::execute_file_operations(file_ops);
    if (file_op_failures > 0)
    {
        logger.log_warning(std::format("[install] {} file operations failed during FOMOD install",
                                       file_op_failures));
    }

    // Move result into the mod directory. dst_base lives inside temp_dir,
    // which gets removed at the bottom of install_mod, so we don't need to
    // keep its files around. fs::rename per child is essentially free on the
    // same volume; on cross-volume installs (temp on C:, mod_path on D:) it
    // falls back to copy + delete inside FileOperations.
    logger.log(std::format("[install] Moving unfomod files to mod directory: {}", mod_path));
    FileOperations::move_directory_contents(fs::path(dst_base), mod_path);

    logger.log(std::format("[install] FOMOD installation steps completed in {}", temp_dir));
    return mod_path;
}

std::string InstallationService::resolve_json_path(const std::string& json_path,
                                                   const std::string& archive_path)
{
    auto& logger = Logger::instance();

    // Caller provided an explicit path -- trust it.
    if (!json_path.empty())
        return json_path;

    // Derive from archive stem, but only accept if it lives next to the archive
    // (prevents accidentally loading an unrelated JSON from another directory).
    auto p = fs::path(archive_path);
    auto derived = (p.parent_path() / (p.stem().string() + ".json")).string();

    if (fs::exists(derived))
    {
        auto parent_dir = p.parent_path();
        if (!parent_dir.empty() && !mo2core::is_inside(parent_dir, fs::path(derived)))
        {
            logger.log_warning(std::format(
                "[install] Rejecting derived JSON path outside archive directory: {}", derived));
            return "";
        }
        return derived;
    }
    return "";
}

}  // namespace mo2core
