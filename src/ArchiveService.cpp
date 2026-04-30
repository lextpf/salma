#include "ArchiveService.h"
#include "Logger.h"
#include "Utils.h"

#include <archive.h>
#include <archive_entry.h>

#include <bit7z/bit7zlibrary.hpp>
#include <bit7z/bitarchivereader.hpp>
#include <bit7z/bitexception.hpp>
#include <bit7z/bitformat.hpp>

#include <algorithm>
#include <chrono>
#include <cstring>
#include <filesystem>
#include <format>
#include <fstream>

namespace fs = std::filesystem;

namespace mo2core
{

// Out-of-line destructors for the RAII guards declared in ArchiveService.h.
// Bodies live here so the header does not have to include <archive.h>.
ArchiveReadGuard::~ArchiveReadGuard()
{
    if (handle_)
    {
        archive_read_close(handle_);
        archive_read_free(handle_);
    }
}

ArchiveWriteGuard::~ArchiveWriteGuard()
{
    if (handle_)
    {
        archive_write_close(handle_);
        archive_write_free(handle_);
    }
}

// Streams raw data blocks from the reader to the disk writer.
// Returns ARCHIVE_OK on success or the first error code encountered.
static int copy_data(struct archive* ar, struct archive* aw)
{
    const void* buff;
    size_t size;
    la_int64_t offset;
    while (true)
    {
        int r = archive_read_data_block(ar, &buff, &size, &offset);
        if (r == ARCHIVE_EOF)
        {
            return ARCHIVE_OK;
        }
        if (r < ARCHIVE_OK)
        {
            return r;
        }
        r = archive_write_data_block(aw, buff, size, offset);
        if (r < ARCHIVE_OK)
        {
            return r;
        }
    }
}

// Use shared normalize_path for archive entry path normalization.
static std::string normalize_entry_path(const std::string& p)
{
    return normalize_path(p);
}

// 1 MiB read buffer - reduces syscall overhead on large mods
static constexpr size_t kArchiveReadBlockSize = 1024 * 1024;

// Maximum decompressed entry size we will allocate in memory (256 MiB).
// Prevents decompression-bomb style OOM from crafted archives.
static constexpr int64_t kMaxEntrySize = 256LL * 1024 * 1024;

// Validate that all files extracted by bit7z stay within the destination.
// Logs and removes any path-traversal entries.
static void validate_bit7z_extraction(const fs::path& destination, mo2core::Logger& logger)
{
    auto canonical_dest = fs::weakly_canonical(destination);
    std::vector<fs::path> to_remove;
    for (const auto& entry : fs::recursive_directory_iterator(destination))
    {
        auto canonical_entry = fs::weakly_canonical(entry.path());
        auto rel = canonical_entry.lexically_relative(canonical_dest);
        if (rel.empty() || rel.string().starts_with(".."))
        {
            logger.log_warning(
                std::format("[archive] Removing path-traversal entry from bit7z extraction: {}",
                            entry.path().string()));
            to_remove.push_back(entry.path());
        }
    }
    // Sort by descending path length so deepest entries are removed first,
    // preventing remove_all on a parent from invalidating child entries.
    std::sort(to_remove.begin(),
              to_remove.end(),
              [](const fs::path& a, const fs::path& b)
              { return a.string().size() > b.string().size(); });
    for (const auto& p : to_remove)
    {
        std::error_code ec;
        fs::remove_all(p, ec);
    }
}

// Reads an entire file into memory. Returns empty vector on any failure.
static std::vector<char> read_file_bytes(const fs::path& file_path)
{
    std::ifstream ifs(file_path, std::ios::binary);
    if (!ifs)
    {
        return {};
    }

    std::error_code ec;
    auto size = fs::file_size(file_path, ec);
    if (ec)
    {
        return {};
    }

    std::vector<char> data(static_cast<size_t>(size));
    if (size > 0)
    {
        ifs.read(data.data(), static_cast<std::streamsize>(data.size()));
        if (static_cast<size_t>(ifs.gcount()) != data.size())
        {
            return {};
        }
    }
    return data;
}

// Searches common 7-Zip install locations for 7z.dll.
// Checks SEVENZIP_PATH env var first, then Program Files directories.
static std::string find_7z_library()
{
#ifdef _WIN32
    const char* env = std::getenv("SEVENZIP_PATH");
    if (env)
    {
        auto p = fs::path(env);
        // Accept either the DLL path directly or its parent directory
        if (fs::exists(p) && p.extension() == ".dll")
        {
            return p.string();
        }
        auto dll = p.parent_path() / "7z.dll";
        if (fs::exists(dll))
        {
            return dll.string();
        }
    }

    auto pf = std::string(std::getenv("ProgramFiles") ? std::getenv("ProgramFiles")
                                                      : "C:\\Program Files");
    auto pf86 = std::string(std::getenv("ProgramFiles(x86)") ? std::getenv("ProgramFiles(x86)")
                                                             : "C:\\Program Files (x86)");
    for (const auto& dir : {pf + "\\7-Zip", pf86 + "\\7-Zip"})
    {
        auto dll = fs::path(dir) / "7z.dll";
        if (fs::exists(dll))
        {
            return dll.string();
        }
    }
#endif
    // Fall back to default DLL search order (PATH, system dirs)
    return "7z.dll";
}

// Lazy-initialized singleton - loads 7z codecs once for the process lifetime.
static bit7z::Bit7zLibrary& get_bit7z_lib()
{
    static bit7z::Bit7zLibrary lib{find_7z_library()};
    return lib;
}

bool ArchiveService::use_bit7z(const std::string& archive_path)
{
    auto ext = to_lower(fs::path(archive_path).extension().string());
    return ext == ".7z" || ext == ".rar" || ext == ".001";
}

void ArchiveService::extract_with_libarchive(const std::string& archive_path,
                                             const std::string& destination_path)
{
    auto& logger = Logger::instance();
    logger.log(std::format("[archive] Using libarchive for extraction: {}", archive_path));

    fs::create_directories(destination_path);

    // Reader handles decompression; disk writer handles writing entries to the filesystem.
    // support_filter_all / support_format_all enable auto-detection of any format/compression.
    auto a = archive_read_new();
    auto ext = archive_write_disk_new();
    ArchiveReadGuard read_guard(a);
    ArchiveWriteGuard write_guard(ext);
    archive_read_support_filter_all(a);
    archive_read_support_format_all(a);

    if (archive_read_open_filename(a, archive_path.c_str(), kArchiveReadBlockSize) != ARCHIVE_OK)
    {
        std::string err = archive_error_string(a) ? archive_error_string(a) : "unknown error";
        throw std::runtime_error("archive_read_open_filename() error: " + err);
    }

    // Full extraction preserves all metadata so installed mods keep their original timestamps
    // and permissions. ACLs/fflags are best-effort (warnings logged, not fatal).
    archive_write_disk_set_options(
        ext,
        ARCHIVE_EXTRACT_TIME | ARCHIVE_EXTRACT_PERM | ARCHIVE_EXTRACT_ACL | ARCHIVE_EXTRACT_FFLAGS);
    // standard_lookup maps archive uid/gid to the host OS's user/group database
    archive_write_disk_set_standard_lookup(ext);

    auto canonical_dest = fs::weakly_canonical(destination_path);

    struct archive_entry* entry = nullptr;
    int r;
    int count = 0;
    int failed_entries = 0;
    // Cache created directories - fs::create_directories is expensive (stat + mkdir) and many
    // archive entries share the same parent, so deduplicating saves significant I/O
    std::unordered_set<std::string> created_dirs;
    while ((r = archive_read_next_header(a, &entry)) != ARCHIVE_EOF)
    {
        // ARCHIVE_WARN is non-fatal (e.g. unsupported ACL type); anything below is fatal
        if (r < ARCHIVE_WARN)
        {
            std::string err = archive_error_string(a) ? archive_error_string(a) : "unknown error";
            throw std::runtime_error("Archive read error: " + err);
        }

        // Rewrite entry path to place output under destination directory
        fs::path full_output = fs::path(destination_path) / archive_entry_pathname(entry);

        // Path traversal guard: reject entries that escape the destination directory
        auto canonical_output = fs::weakly_canonical(full_output);
        auto rel = canonical_output.lexically_relative(canonical_dest);
        if (rel.empty() || rel.string().starts_with(".."))
        {
            logger.log_warning(std::format("[archive] Skipping path-traversal entry: {}",
                                           archive_entry_pathname(entry)));
            archive_read_data_skip(a);
            continue;
        }
        auto parent = full_output.parent_path();
        if (!parent.empty())
        {
            auto parent_str = parent.string();
            if (created_dirs.insert(parent_str).second)
            {
                fs::create_directories(parent);
            }
        }
        archive_entry_set_pathname(entry, full_output.string().c_str());

        r = archive_write_header(ext, entry);
        if (r < ARCHIVE_OK)
        {
            logger.log_warning(
                std::format("[archive] Write header warning: {}",
                            archive_error_string(ext) ? archive_error_string(ext) : "unknown"));
        }
        else if (archive_entry_size(entry) > 0)
        {
            if (copy_data(a, ext) < ARCHIVE_OK)
            {
                logger.log_warning(
                    std::format("[archive] Copy data warning: {}",
                                archive_error_string(ext) ? archive_error_string(ext) : "unknown"));
                failed_entries++;
            }
        }
        archive_write_finish_entry(ext);
        count++;

        if (count % 100 == 0)
        {
            logger.log(std::format("[archive] Extracted {} files...", count));
        }
    }

    if (failed_entries > 0)
    {
        logger.log_warning(std::format(
            "[archive] Libarchive extraction completed with {} failed entries out of {}",
            failed_entries,
            count));
    }
    else
    {
        logger.log(std::format("[archive] Libarchive extraction completed: {} entries", count));
    }
}

void ArchiveService::extract(const std::string& archive_path, const std::string& destination_path)
{
    auto& logger = Logger::instance();
    logger.log(std::format("[archive] Extracting archive: {}", archive_path));

    // Try bit7z first for formats it handles natively (.7z, .rar, .001)
    if (use_bit7z(archive_path))
    {
        try
        {
            fs::create_directories(destination_path);
            bit7z::BitArchiveReader reader{get_bit7z_lib(), archive_path, bit7z::BitFormat::Auto};
            reader.extractTo(destination_path);
            validate_bit7z_extraction(fs::path(destination_path), logger);
            logger.log("[archive] bit7z extraction completed");
            return;
        }
        catch (const bit7z::BitException& e)
        {
            // bit7z can fail on edge cases (split volumes, broken headers) - fall through to
            // libarchive
            logger.log(std::format(
                "[archive] bit7z extraction failed: {}, falling back to libarchive", e.what()));
        }
    }

    extract_with_libarchive(archive_path, destination_path);
}

std::vector<std::string> ArchiveService::list_entries(const std::string& archive_path)
{
    return list_entries_with_sizes(archive_path).paths;
}

ArchiveService::EntryListing ArchiveService::list_entries_with_sizes(
    const std::string& archive_path)
{
    using clock = std::chrono::steady_clock;
    auto& logger = Logger::instance();
    auto t0 = clock::now();
    auto ext = to_lower(fs::path(archive_path).extension().string());
    EntryListing listing;

    if (use_bit7z(archive_path))
    {
        try
        {
            logger.log(std::format("[archive] list_entries via bit7z for {} file", ext));
            bit7z::BitArchiveReader reader{get_bit7z_lib(), archive_path, bit7z::BitFormat::Auto};

            // bit7z iterates the archive's central directory - no decompression needed.
            // Directory entries are skipped because callers only care about files.
            for (const auto& item : reader)
            {
                if (item.isDir())
                {
                    continue;
                }
                auto path = item.path();
                // paths keeps original casing; sizes map uses normalized key for case-insensitive
                // lookup
                listing.paths.push_back(path);
                listing.sizes[normalize_entry_path(path)] = item.size();
            }

            auto ms =
                std::chrono::duration_cast<std::chrono::milliseconds>(clock::now() - t0).count();
            logger.log(std::format("[archive] list_entries: {} entries, {} sizes via bit7z ({}ms)",
                                   listing.paths.size(),
                                   listing.sizes.size(),
                                   ms));
            return listing;
        }
        catch (const bit7z::BitException& e)
        {
            // Clear any partial results from the failed bit7z attempt before falling through
            logger.log(std::format(
                "[archive] bit7z list_entries failed: {}, falling back to libarchive", e.what()));
            listing.paths.clear();
            listing.sizes.clear();
        }
    }

    // libarchive fallback - reads headers sequentially, skipping data blocks
    logger.log(std::format("[archive] list_entries via libarchive for {} file", ext));
    auto a = archive_read_new();
    ArchiveReadGuard guard(a);
    archive_read_support_filter_all(a);
    archive_read_support_format_all(a);

    if (archive_read_open_filename(a, archive_path.c_str(), kArchiveReadBlockSize) != ARCHIVE_OK)
    {
        // Return empty listing on open failure rather than throwing - callers
        // treat empty results as "no entries found" which is a safe default
        return listing;
    }

    struct archive_entry* entry = nullptr;
    while (archive_read_next_header(a, &entry) == ARCHIVE_OK)
    {
        const char* path = archive_entry_pathname(entry);
        if (path)
        {
            listing.paths.emplace_back(path);
            auto raw_size = archive_entry_size(entry);
            if (raw_size >= 0)
            {
                listing.sizes[normalize_entry_path(path)] = static_cast<uint64_t>(raw_size);
            }
        }
        // Advance past the data block without decompressing - header-only scan
        archive_read_data_skip(a);
    }
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(clock::now() - t0).count();
    logger.log(std::format("[archive] list_entries: {} entries, {} sizes via libarchive ({}ms)",
                           listing.paths.size(),
                           listing.sizes.size(),
                           ms));
    return listing;
}

void ArchiveService::extract_filtered(const std::string& archive_path,
                                      const std::string& destination_path,
                                      std::function<bool(const std::string&)> filter)
{
    auto& logger = Logger::instance();
    fs::create_directories(destination_path);

    auto a = archive_read_new();
    auto ext = archive_write_disk_new();
    ArchiveReadGuard read_guard(a);
    ArchiveWriteGuard write_guard(ext);
    archive_read_support_filter_all(a);
    archive_read_support_format_all(a);

    if (archive_read_open_filename(a, archive_path.c_str(), kArchiveReadBlockSize) != ARCHIVE_OK)
    {
        std::string err = archive_error_string(a) ? archive_error_string(a) : "unknown error";
        throw std::runtime_error("extract_filtered open error: " + err);
    }

    // Only timestamps for filtered extraction - ACLs/permissions are irrelevant for
    // selective mod file copies and skipping them is slightly faster
    archive_write_disk_set_options(ext, ARCHIVE_EXTRACT_TIME);
    archive_write_disk_set_standard_lookup(ext);

    auto canonical_dest = fs::weakly_canonical(destination_path);

    struct archive_entry* entry = nullptr;
    int count = 0;
    std::unordered_set<std::string> created_dirs;
    while (archive_read_next_header(a, &entry) == ARCHIVE_OK)
    {
        const char* path = archive_entry_pathname(entry);
        // archive_read_data_skip advances past the compressed data without decompressing,
        // so rejected entries are essentially free
        if (!path || !filter(path))
        {
            archive_read_data_skip(a);
            continue;
        }

        fs::path full_output = fs::path(destination_path) / path;

        // Path traversal guard: reject entries that escape the destination directory
        auto canonical_output = fs::weakly_canonical(full_output);
        auto rel = canonical_output.lexically_relative(canonical_dest);
        if (rel.empty() || rel.string().starts_with(".."))
        {
            logger.log_warning(std::format("[archive] Skipping path-traversal entry: {}", path));
            archive_read_data_skip(a);
            continue;
        }
        auto parent = full_output.parent_path();
        if (!parent.empty())
        {
            auto parent_str = parent.string();
            if (created_dirs.insert(parent_str).second)
            {
                fs::create_directories(parent);
            }
        }
        archive_entry_set_pathname(entry, full_output.string().c_str());

        int r = archive_write_header(ext, entry);
        if (r >= ARCHIVE_OK && archive_entry_size(entry) > 0)
        {
            int cd_r = copy_data(a, ext);
            if (cd_r < ARCHIVE_OK)
            {
                logger.log_warning(
                    std::format("[archive] copy_data failed for entry: code {}", cd_r));
            }
        }
        archive_write_finish_entry(ext);
        count++;
    }

    logger.log(std::format("[archive] extract_filtered: extracted {} entries", count));
}

void ArchiveService::extract_prefix(const std::string& archive_path,
                                    const std::string& destination_path,
                                    const std::string& prefix)
{
    auto& logger = Logger::instance();

    if (use_bit7z(archive_path))
    {
        try
        {
            fs::create_directories(destination_path);
            bit7z::BitArchiveReader reader{get_bit7z_lib(), archive_path, bit7z::BitFormat::Auto};

            auto prefix_lower = to_lower(prefix);
            std::replace(prefix_lower.begin(), prefix_lower.end(), '\\', '/');

            // Collect matching indices first, then extract in one call -
            // much faster than extracting items individually from solid archives
            std::vector<uint32_t> indices;
            for (const auto& item : reader)
            {
                auto norm = normalize_entry_path(item.path());
                if (norm.starts_with(prefix_lower))
                {
                    indices.push_back(item.index());
                }
            }

            if (!indices.empty())
            {
                reader.extractTo(destination_path, indices);
                validate_bit7z_extraction(fs::path(destination_path), logger);
            }

            logger.log(std::format("[archive] bit7z extract_prefix: {} entries", indices.size()));
            return;
        }
        catch (const bit7z::BitException& e)
        {
            logger.log(std::format(
                "[archive] bit7z extract_prefix failed: {}, falling back to libarchive", e.what()));
        }
    }

    // libarchive fallback - wrap prefix match as a filter predicate
    auto prefix_lower = to_lower(prefix);
    std::replace(prefix_lower.begin(), prefix_lower.end(), '\\', '/');

    extract_filtered(archive_path,
                     destination_path,
                     [&](const std::string& entry_path)
                     {
                         auto norm = to_lower(entry_path);
                         std::replace(norm.begin(), norm.end(), '\\', '/');
                         return norm.starts_with(prefix_lower);
                     });
}

std::vector<char> ArchiveService::read_entry(const std::string& archive_path,
                                             const std::string& entry_name)
{
    if (use_bit7z(archive_path))
    {
        try
        {
            bit7z::BitArchiveReader reader{get_bit7z_lib(), archive_path, bit7z::BitFormat::Auto};
            auto target = normalize_entry_path(entry_name);

            for (const auto& item : reader)
            {
                if (item.isDir())
                {
                    continue;
                }
                if (normalize_entry_path(item.path()) == target)
                {
                    // Extract single item to memory by index - avoids decompressing the whole
                    // archive
                    std::vector<bit7z::byte_t> buffer;
                    reader.extractTo(buffer, item.index());
                    return {buffer.begin(), buffer.end()};
                }
            }
            return {};
        }
        catch (const bit7z::BitException&)
        {
            return {};
        }
    }

    // libarchive: sequential scan - entries are streamed in order so we
    // must walk through until we find the match or reach the end
    auto a = archive_read_new();
    ArchiveReadGuard guard(a);
    archive_read_support_filter_all(a);
    archive_read_support_format_all(a);

    if (archive_read_open_filename(a, archive_path.c_str(), kArchiveReadBlockSize) != ARCHIVE_OK)
    {
        return {};
    }

    // Normalize the target the same way as entry paths for case-insensitive comparison
    std::string target = normalize_entry_path(entry_name);

    struct archive_entry* entry = nullptr;
    while (archive_read_next_header(a, &entry) == ARCHIVE_OK)
    {
        std::string path = archive_entry_pathname(entry) ? archive_entry_pathname(entry) : "";
        path = normalize_entry_path(path);

        if (path == target)
        {
            // Read the entire entry into memory in one call - safe because
            // mod files are typically small (configs, XMLs, textures)
            auto size = archive_entry_size(entry);
            if (size < 0 || size > kMaxEntrySize)
            {
                return {};
            }
            std::vector<char> data(static_cast<size_t>(size));
            if (size > 0)
            {
                auto bytes_read = archive_read_data(a, data.data(), data.size());
                if (bytes_read < 0)
                {
                    return {};
                }
                data.resize(static_cast<size_t>(bytes_read));
            }
            return data;
        }
        archive_read_data_skip(a);
    }

    return {};
}

std::unordered_map<std::string, std::vector<char>> ArchiveService::read_entries_batch(
    const std::string& archive_path, const std::unordered_set<std::string>& entry_names)
{
    using clock = std::chrono::steady_clock;
    std::unordered_map<std::string, std::vector<char>> results;
    if (entry_names.empty())
    {
        return results;
    }

    auto& logger = Logger::instance();
    auto t0 = clock::now();
    logger.log(
        std::format("[archive] read_entries_batch: {} entries requested", entry_names.size()));

    if (use_bit7z(archive_path))
    {
        try
        {
            bit7z::BitArchiveReader reader{get_bit7z_lib(), archive_path, bit7z::BitFormat::Auto};

            // Map requested entry names to archive item indices.
            // Sorted by index so extraction follows the archive's physical order.
            struct Match
            {
                uint32_t index;
                std::string norm_name;
            };
            std::vector<Match> matches;

            for (const auto& item : reader)
            {
                if (item.isDir())
                {
                    continue;
                }
                auto norm = normalize_entry_path(item.path());
                if (entry_names.count(norm))
                {
                    matches.push_back({item.index(), norm});
                }
            }

            std::sort(matches.begin(),
                      matches.end(),
                      [](const Match& a, const Match& b) { return a.index < b.index; });

            // Solid archives store all data in one compressed block. Random-access
            // reads decompress from the start each time, so for 4+ entries it's
            // cheaper to extract the subset to a temp dir and read from disk.
            if (matches.size() >= 4 && reader.isSolid())
            {
                fs::path temp_dir = fs::temp_directory_path() /
                                    std::format("salma-bit7z-batch-{}", random_hex_string());
                fs::create_directories(temp_dir);

                try
                {
                    std::vector<uint32_t> indices;
                    indices.reserve(matches.size());
                    for (const auto& m : matches)
                    {
                        indices.push_back(m.index);
                    }

                    reader.extractTo(temp_dir.string(), indices);
                    validate_bit7z_extraction(temp_dir, logger);

                    // Read extracted files back into memory
                    for (const auto& p : fs::recursive_directory_iterator(temp_dir))
                    {
                        if (!p.is_regular_file())
                        {
                            continue;
                        }
                        auto rel = fs::relative(p.path(), temp_dir).string();
                        auto norm = normalize_entry_path(rel);
                        if (!entry_names.count(norm))
                        {
                            continue;
                        }
                        results[norm] = read_file_bytes(p.path());
                    }
                }
                catch (...)
                {
                    // Clean up temp dir even on failure
                    std::error_code ec;
                    fs::remove_all(temp_dir, ec);
                    throw;
                }

                std::error_code ec;
                fs::remove_all(temp_dir, ec);

                auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(clock::now() - t0)
                              .count();
                logger.log(std::format(
                    "[archive] read_entries_batch: {}/{} entries via bit7z solid-batch ({}ms)",
                    results.size(),
                    entry_names.size(),
                    ms));
                return results;
            }

            // Non-solid archive - extract each matched item directly to memory
            for (const auto& m : matches)
            {
                std::vector<bit7z::byte_t> buffer;
                reader.extractTo(buffer, m.index);
                results[m.norm_name] = {buffer.begin(), buffer.end()};
            }

            auto ms =
                std::chrono::duration_cast<std::chrono::milliseconds>(clock::now() - t0).count();
            logger.log(std::format("[archive] read_entries_batch: {}/{} entries via bit7z ({}ms)",
                                   results.size(),
                                   entry_names.size(),
                                   ms));
            return results;
        }
        catch (const bit7z::BitException& e)
        {
            logger.log(std::format(
                "[archive] bit7z read_entries_batch failed: {}, falling back to libarchive",
                e.what()));
            results.clear();
        }
    }

    // libarchive fallback - single sequential pass over the archive.
    // Matched entries are read into memory; non-matches are skipped without decompression.
    auto a = archive_read_new();
    ArchiveReadGuard guard(a);
    archive_read_support_filter_all(a);
    archive_read_support_format_all(a);

    if (archive_read_open_filename(a, archive_path.c_str(), kArchiveReadBlockSize) != ARCHIVE_OK)
    {
        return results;
    }

    // Track how many entries are still outstanding so we can stop scanning
    // early once everything has been found
    size_t remaining = entry_names.size();
    struct archive_entry* entry = nullptr;
    while (remaining > 0 && archive_read_next_header(a, &entry) == ARCHIVE_OK)
    {
        std::string raw_path = archive_entry_pathname(entry) ? archive_entry_pathname(entry) : "";
        // Use normalize_entry_path for consistent matching with bit7z path
        // (strips leading "./" and "/" prefixes in addition to lowercase + slash conversion)
        auto path = normalize_entry_path(raw_path);

        if (entry_names.count(path))
        {
            auto size = archive_entry_size(entry);
            if (size < 0 || size > kMaxEntrySize)
            {
                archive_read_data_skip(a);
                continue;
            }
            std::vector<char> data(static_cast<size_t>(size));
            if (size > 0)
            {
                auto bytes_read = archive_read_data(a, data.data(), data.size());
                if (bytes_read < 0)
                {
                    archive_read_data_skip(a);
                    continue;
                }
                data.resize(static_cast<size_t>(bytes_read));
            }
            results[path] = std::move(data);
            remaining--;
        }
        else
        {
            // Skip non-matching entries without decompression
            archive_read_data_skip(a);
        }
    }

    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(clock::now() - t0).count();
    logger.log(std::format("[archive] read_entries_batch: {}/{} entries via libarchive ({}ms)",
                           results.size(),
                           entry_names.size(),
                           ms));
    return results;
}

void ArchiveService::create_zip(const std::string& folder_path, const std::string& output_zip_path)
{
    fs::create_directories(fs::path(output_zip_path).parent_path());

    auto a = archive_write_new();
    ArchiveWriteGuard guard(a);
    archive_write_set_format_zip(a);
    if (archive_write_open_filename(a, output_zip_path.c_str()) != ARCHIVE_OK)
    {
        throw std::runtime_error("Failed to open " + output_zip_path + " for writing");
    }

    for (const auto& p : fs::recursive_directory_iterator(folder_path))
    {
        if (fs::is_directory(p))
        {
            continue;
        }

        auto entry = archive_entry_new();
        // Store paths relative to the source folder so the zip unpacks cleanly
        auto rel_path = fs::relative(p.path(), folder_path).string();
        archive_entry_set_pathname(entry, rel_path.c_str());

        std::error_code size_ec;
        auto file_sz = fs::file_size(p, size_ec);
        if (size_ec)
        {
            Logger::instance().log_warning(
                std::format("[archive] Skipping file in zip (cannot read size): {}", rel_path));
            archive_entry_free(entry);
            continue;
        }

        archive_entry_set_size(entry, static_cast<la_int64_t>(file_sz));
        archive_entry_set_filetype(entry, AE_IFREG);  // regular file (not symlink/directory)
        archive_entry_set_perm(entry, 0644);          // rw-r--r- standard permissions

        if (archive_write_header(a, entry) != ARCHIVE_OK)
        {
            archive_entry_free(entry);
            continue;
        }

        // Stream file contents in 8 KiB chunks to avoid loading large files entirely into memory
        std::ifstream ifs(p.path(), std::ios::binary);
        char buffer[8192];
        bool write_ok = true;
        while (write_ok && (ifs.read(buffer, sizeof(buffer)) || ifs.gcount()))
        {
            auto to_write = static_cast<size_t>(ifs.gcount());
            size_t total_written = 0;
            while (total_written < to_write)
            {
                auto written =
                    archive_write_data(a, buffer + total_written, to_write - total_written);
                if (written <= 0)
                {
                    Logger::instance().log_warning(
                        std::format("[archive] Write error in zip for: {}", rel_path));
                    write_ok = false;
                    break;
                }
                total_written += static_cast<size_t>(written);
            }
        }
        archive_entry_free(entry);
    }
}

}  // namespace mo2core
