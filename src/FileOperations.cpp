#include "FileOperations.h"
#include <algorithm>
#include <atomic>
#include <format>
#include <system_error>
#include "Logger.h"

namespace fs = std::filesystem;

namespace mo2core
{

namespace
{

// Sticky process-wide flag; once set, callers can decide the install is
// unrecoverable without inspecting individual error logs. Reset by
// reset_disk_full() at install boundaries.
std::atomic<bool> g_disk_full{false};

// True when @p ec means "the volume ran out of space". Bit7z and Windows
// surface this as both ENOSPC and ERROR_DISK_FULL/ERROR_HANDLE_DISK_FULL,
// so check the portable error_condition rather than raw values.
bool is_disk_full(const std::error_code& ec)
{
    return ec == std::errc::no_space_on_device;
}

void note_disk_full_if_applicable(const fs::filesystem_error& ex)
{
    if (is_disk_full(ex.code()))
    {
        g_disk_full.store(true, std::memory_order_relaxed);
    }
}

}  // namespace

bool FileOperations::disk_full_encountered()
{
    return g_disk_full.load(std::memory_order_relaxed);
}

void FileOperations::reset_disk_full()
{
    g_disk_full.store(false, std::memory_order_relaxed);
}

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
        note_disk_full_if_applicable(ex);
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
                note_disk_full_if_applicable(ex);
                Logger::instance().log_error(std::format("[install] Copy error: {}", ex.what()));
            }
        }
    }
    catch (const fs::filesystem_error& ex)
    {
        note_disk_full_if_applicable(ex);
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
        note_disk_full_if_applicable(ex);
        Logger::instance().log_error(
            std::format("[install] Failed to iterate directory {}: {}", src.string(), ex.what()));
    }
}

void FileOperations::move_directory_contents(const fs::path& src, const fs::path& dst)
{
    // Same shape as copy_directory_contents, but tries fs::rename per child
    // first. The rename is the win: a same-volume rename is a metadata
    // operation, so unfomod -> mod_path costs effectively zero disk.
    // When the destination is on a different volume, std::filesystem::rename
    // raises errc::cross_device_link; fall back to copy + delete in that case.
    if (!fs::exists(src))
    {
        Logger::instance().log_warning(
            std::format("[install] Missing folder for move: {}", src.string()));
        return;
    }
    try
    {
        fs::create_directories(dst);
    }
    catch (const fs::filesystem_error& ex)
    {
        note_disk_full_if_applicable(ex);
        Logger::instance().log_error(
            std::format("[install] Failed to create directory {}: {}", dst.string(), ex.what()));
        return;
    }
    try
    {
        for (const auto& entry : fs::directory_iterator(src))
        {
            auto target = dst / entry.path().filename();
            std::error_code rename_ec;
            fs::rename(entry.path(), target, rename_ec);
            if (!rename_ec)
            {
                continue;
            }
            // Cross-volume moves cannot be done by rename; fall back to
            // copy + remove. Other rename failures (e.g. target locked,
            // permissions) also benefit from the fallback.
            if (is_disk_full(rename_ec))
            {
                g_disk_full.store(true, std::memory_order_relaxed);
                Logger::instance().log_error(std::format(
                    "[install] Move error (disk full) {} -> {}: {}",
                    entry.path().string(),
                    target.string(),
                    rename_ec.message()));
                continue;
            }
            // Log which fallback path we are on so we can tell same-volume
            // rename failures (a real problem) from cross-volume moves
            // (expected, the copy fallback below handles them correctly).
            Logger::instance().log_warning(std::format(
                "[install] rename {} -> {} failed ({}); falling back to copy",
                entry.path().string(),
                target.string(),
                rename_ec.message()));
            if (fs::is_directory(entry))
            {
                copy_folder(entry.path(), target);
            }
            else
            {
                copy_file(entry.path(), target);
            }
            // Best-effort source cleanup; if it fails the temp dir gets
            // cleaned up anyway when the install scope ends.
            std::error_code remove_ec;
            fs::remove_all(entry.path(), remove_ec);
            if (remove_ec)
            {
                Logger::instance().log_warning(std::format(
                    "[install] Move fallback could not remove source {}: {}",
                    entry.path().string(),
                    remove_ec.message()));
            }
        }
    }
    catch (const fs::filesystem_error& ex)
    {
        note_disk_full_if_applicable(ex);
        Logger::instance().log_error(
            std::format("[install] Failed to iterate directory for move {}: {}",
                        src.string(),
                        ex.what()));
    }
}

}  // namespace mo2core
