#include "FileOperations.h"
#include <algorithm>
#include <format>
#include "Logger.h"

namespace fs = std::filesystem;

namespace mo2core
{

void FileOperations::add(FileOperation op)
{
    ops_.push_back(std::move(op));
}

void FileOperations::execute()
{
    // Stable sort ascending by priority - higher-priority ops execute later
    // and overwrite earlier ones at the same destination (last-write-wins).
    // Stable sort preserves document order among equal-priority operations.
    std::stable_sort(ops_.begin(),
                     ops_.end(),
                     [](const FileOperation& a, const FileOperation& b)
                     { return a.priority < b.priority; });

    auto& logger = Logger::instance();
    logger.log(
        std::format("[install] Executing {} file operations in priority order...", ops_.size()));

    // Individual failures are logged and skipped; they do not abort the batch.
    for (const auto& op : ops_)
    {
        try
        {
            if (op.type == FileOpType::File)
            {
                copy_file(op.source, op.destination);
            }
            else
            {
                copy_folder(op.source, op.destination);
            }
        }
        catch (const std::exception& ex)
        {
            logger.log_error(std::format("[install] Failed to execute file operation: {} -> {}: {}",
                                         op.source,
                                         op.destination,
                                         ex.what()));
        }
    }

    ops_.clear();
}

void FileOperations::clear()
{
    ops_.clear();
}

int FileOperations::count() const
{
    return static_cast<int>(ops_.size());
}

void FileOperations::copy_file(const fs::path& src, const fs::path& dst)
{
    // Missing source is logged and silently skipped (not an error).
    if (!fs::exists(src))
    {
        Logger::instance().log_warning(std::format("[install] Missing file: {}", src.string()));
        return;
    }
    try
    {
        fs::create_directories(dst.parent_path());
    }
    catch (const fs::filesystem_error& ex)
    {
        Logger::instance().log_error(std::format(
            "[install] Failed to create directory {}: {}", dst.parent_path().string(), ex.what()));
        return;
    }
    try
    {
        // Always overwrites existing destination files.
        fs::copy_file(src, dst, fs::copy_options::overwrite_existing);
    }
    catch (const fs::filesystem_error& ex)
    {
        Logger::instance().log_error(std::format("[install] Copy error: {}", ex.what()));
    }
}

void FileOperations::copy_folder(const fs::path& src, const fs::path& dst)
{
    // Missing source is logged and silently skipped.
    if (!fs::exists(src))
    {
        Logger::instance().log_warning(std::format("[install] Missing folder: {}", src.string()));
        return;
    }
    try
    {
        fs::create_directories(dst);
    }
    catch (const fs::filesystem_error& ex)
    {
        Logger::instance().log_error(
            std::format("[install] Failed to create directory {}: {}", dst.string(), ex.what()));
        return;
    }
    try
    {
        // Recursive traversal - overwrites existing files at each destination.
        // skip_permission_denied avoids throwing on inaccessible entries.
        for (const auto& entry :
             fs::recursive_directory_iterator(src, fs::directory_options::skip_permission_denied))
        {
            if (fs::is_symlink(entry.symlink_status()))
                continue;

            auto relative = fs::relative(entry.path(), src);
            auto dest_path = dst / relative;
            try
            {
                if (fs::is_directory(entry))
                {
                    fs::create_directories(dest_path);
                }
                else
                {
                    fs::create_directories(dest_path.parent_path());
                    fs::copy_file(entry.path(), dest_path, fs::copy_options::overwrite_existing);
                }
            }
            catch (const fs::filesystem_error& ex)
            {
                Logger::instance().log_error(std::format("[install] Copy error: {}", ex.what()));
            }
        }
    }
    catch (const fs::filesystem_error& ex)
    {
        Logger::instance().log_error(
            std::format("[install] Failed to iterate directory {}: {}", src.string(), ex.what()));
    }
}

void FileOperations::copy_directory_contents(const fs::path& src, const fs::path& dst)
{
    // Copies immediate children of src into dst (one level for files,
    // recursive for subdirectories via copy_folder).
    try
    {
        fs::create_directories(dst);
    }
    catch (const fs::filesystem_error& ex)
    {
        Logger::instance().log_error(
            std::format("[install] Failed to create directory {}: {}", dst.string(), ex.what()));
        return;
    }
    try
    {
        for (const auto& entry : fs::directory_iterator(src))
        {
            if (fs::is_directory(entry))
            {
                copy_folder(entry.path(), dst / entry.path().filename());
            }
            else
            {
                copy_file(entry.path(), dst / entry.path().filename());
            }
        }
    }
    catch (const fs::filesystem_error& ex)
    {
        Logger::instance().log_error(
            std::format("[install] Failed to iterate directory {}: {}", src.string(), ex.what()));
    }
}

}  // namespace mo2core
