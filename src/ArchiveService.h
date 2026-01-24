#pragma once

#include <cstdint>
#include <functional>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

// Forward-declare libarchive's opaque handle so consumers of this header
// (including the MO2 plugin path through mo2-core) do not transitively
// pull in <archive.h>. The destructor bodies that actually call into
// libarchive live in ArchiveService.cpp where the full header is included.
struct archive;

namespace mo2core
{

/**
 * @struct ArchiveReadGuard
 * @brief RAII guard that closes and frees a libarchive read handle.
 * @author Alex (https://github.com/lextpf)
 * @ingroup ArchiveService
 *
 * Wraps the result of `archive_read_new()` so early returns and exceptions
 * cannot leak the handle. The guard owns the handle exclusively; copy and
 * assignment are deleted to prevent double-free.
 *
 * The destructor calls `archive_read_close()` before `archive_read_free()`
 * so any pending read state is flushed; both calls ignore a null handle.
 *
 * Used by every libarchive code path in ArchiveService (extraction, header
 * scans, single-entry reads, batch reads). Construct directly from the
 * result of `archive_read_new()`:
 *
 * ```cpp
 * auto a = archive_read_new();
 * ArchiveReadGuard guard(a);
 * archive_read_support_filter_all(a);
 * // ... use `a` freely; the guard frees it on scope exit
 * ```
 *
 * @note Move construction is not provided; the guard is intended for
 *       single-scope ownership only.
 */
struct ArchiveReadGuard
{
    struct archive* handle_ = nullptr; /**< Owned libarchive read handle (nullable). */

    /**
     * @brief Take ownership of an `archive_read_new()` handle.
     * @param archive_handle Handle to wrap. May be nullptr (no-op on destruction).
     */
    explicit ArchiveReadGuard(struct archive* archive_handle) noexcept
        : handle_(archive_handle)
    {
    }

    /**
     * @brief Closes and frees the wrapped handle if non-null.
     *
     * Defined out-of-line in ArchiveService.cpp because the body requires
     * the full libarchive header.
     */
    ~ArchiveReadGuard();

    ArchiveReadGuard(const ArchiveReadGuard&) = delete;
    ArchiveReadGuard& operator=(const ArchiveReadGuard&) = delete;
};

/**
 * @struct ArchiveWriteGuard
 * @brief RAII guard that closes and frees a libarchive write handle.
 * @author Alex (https://github.com/lextpf)
 * @ingroup ArchiveService
 *
 * Counterpart to ArchiveReadGuard for `archive_write_new()` and
 * `archive_write_disk_new()` handles. Wraps the result so early returns
 * and exceptions cannot leak the handle.
 *
 * The destructor calls `archive_write_close()` before
 * `archive_write_free()`; both calls ignore a null handle.
 *
 * Used in ArchiveService for the extract-to-disk path (where each entry
 * is written through `archive_write_disk_new()`) and for `create_zip()`
 * (where `archive_write_new()` produces the zip output). Construct
 * directly from either factory:
 *
 * ```cpp
 * auto ext = archive_write_disk_new();
 * ArchiveWriteGuard guard(ext);
 * archive_write_disk_set_options(ext, flags);
 * // ... use `ext` freely; the guard frees it on scope exit
 * ```
 *
 * @note Move construction is not provided; the guard is intended for
 *       single-scope ownership only.
 */
struct ArchiveWriteGuard
{
    struct archive* handle_ = nullptr; /**< Owned libarchive write handle (nullable). */

    /**
     * @brief Take ownership of an `archive_write_*_new()` handle.
     * @param archive_handle Handle to wrap. May be nullptr (no-op on destruction).
     */
    explicit ArchiveWriteGuard(struct archive* archive_handle) noexcept
        : handle_(archive_handle)
    {
    }

    /**
     * @brief Closes and frees the wrapped handle if non-null.
     *
     * Defined out-of-line in ArchiveService.cpp because the body requires
     * the full libarchive header.
     */
    ~ArchiveWriteGuard();

    ArchiveWriteGuard(const ArchiveWriteGuard&) = delete;
    ArchiveWriteGuard& operator=(const ArchiveWriteGuard&) = delete;
};

/**
 * @class ArchiveService
 * @brief Unified archive I/O facade over libarchive and bit7z.
 * @author Alex (https://github.com/lextpf)
 * @ingroup ArchiveService
 *
 * Provides extraction, listing, in-memory reading, and zip creation through
 * a single interface backed by two libraries.
 *
 * ## :material-archive: Backend Selection
 *
 * |    Backend |     Formats     | When used                        |
 * |------------|-----------------|----------------------------------|
 * | libarchive |   zip, tar.gz   | Default for most archive types   |
 * |      bit7z |     7z, rar     | Preferred when extension matches |
 *
 * Format routing is automatic: bit7z is tried first for `.7z`, `.rar`, and
 * `.001` archives. All other formats go straight to libarchive.
 *
 * Fallback behavior on bit7z failure varies by method:
 * - **extract**, **list_entries**, **extract_prefix**, **read_entries_batch**:
 *   fall back to libarchive transparently.
 * - **read_entry**: returns empty (no fallback) -- callers should treat
 *   an empty result as "not found".
 *
 * ## :material-format-letter-case-lower: Path Normalization
 *
 * All lookup-oriented methods normalize entry paths before comparison:
 * lowercase, backslashes converted to forward slashes. The sizes map
 * and bit7z code paths additionally strip leading `./` and `/` prefixes
 * via `normalize_entry_path()`; the libarchive paths for `read_entry`
 * and `read_entries_batch` only apply lowercase + slash conversion.
 *
 * ## :material-swap-horizontal: Backend Routing
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * flowchart LR
 *     classDef caller fill:#1e3a5f,stroke:#3b82f6,color:#e2e8f0
 *     classDef service fill:#134e3a,stroke:#10b981,color:#e2e8f0
 *     classDef bit7z fill:#4a3520,stroke:#f59e0b,color:#e2e8f0
 *     classDef libarch fill:#2e1f5e,stroke:#8b5cf6,color:#e2e8f0
 *     classDef output fill:transparent,stroke:#94a3b8,color:#e2e8f0,stroke-dasharray:6 4
 *
 *     Caller[Caller]:::caller
 *     AS[ArchiveService]:::service
 *     Check{use bit7z ?}:::service
 *
 *     subgraph B7[bit7z]
 *         direction TB
 *         Lib7z[Singleton]:::bit7z
 *         Reader7z[Reader]:::bit7z
 *     end
 *
 *     subgraph LA[libarchive]
 *         direction TB
 *         ARead[archive_read]:::libarch
 *         AWrite[archive_write]:::libarch
 *     end
 *
 *     Dest[Extracted Files / Memory Buffer]:::output
 *
 *     Caller -->|extract| AS
 *     AS --> Check
 *     Check -->|.7z .rar| B7
 *     Check -->|.zip .tar.gz| LA
 *     B7 -->|failure| LA
 *     B7 --> Dest
 *     LA --> Dest
 * ```
 *
 * ## :material-code-tags: Usage Examples
 *
 * ### Extract an archive
 * ```cpp
 * ArchiveService archiver;
 * archiver.extract("C:/downloads/mod.zip", "C:/mods/MyMod");
 * ```
 *
 * ### List entries and check for a FOMOD installer
 * ```cpp
 * auto entries = archiver.list_entries("C:/downloads/mod.7z");
 * bool hasFomod = std::ranges::any_of(entries, [](const auto& p) {
 *     return to_lower(p).ends_with("moduleconfig.xml");
 * });
 * ```
 *
 * ### Read a single file without extracting the whole archive
 * ```cpp
 * auto xml = archiver.read_entry("C:/downloads/mod.7z", "fomod/ModuleConfig.xml");
 * if (!xml.empty()) {
 *     std::string content(xml.begin(), xml.end());
 * }
 * ```
 *
 * ### Batch-read multiple files in one pass
 * ```cpp
 * std::unordered_set<std::string> needed = {
 *     "fomod/moduleconfig.xml",
 *     "fomod/info.xml"
 * };
 * auto files = archiver.read_entries_batch("C:/downloads/mod.rar", needed);
 * ```
 *
 * ## :material-shield-check: Path Traversal
 *
 * Both extraction backends guard against path traversal attacks:
 *
 * - **bit7z**: after extraction, `validate_bit7z_extraction()` canonicalizes
 *   every extracted path and deletes any entry whose canonical location falls
 *   outside the destination directory.
 * - **libarchive**: extraction rejects entries whose resolved path would escape
 *   the destination directory, skipping them before any data is written.
 *
 * ## :material-weight: Resource Limits
 *
 * Single archive entries larger than **256 MiB** (`kMaxEntrySize` in
 * ArchiveService.cpp) are rejected during in-memory reads (read_entry,
 * read_entries_batch) as a decompression-bomb guard. The threshold
 * applies to the uncompressed size reported in the archive header, so
 * malicious "zip bombs" are caught before any data is buffered.
 *
 * ## :material-alert-circle-outline: Error Semantics
 *
 * - extract(), extract_filtered(), extract_prefix(), create_zip():
 *   throw `std::runtime_error` on open or fatal I/O failure.
 *   extract_prefix() catches bit7z failures and falls back to
 *   libarchive via extract_filtered(), which throws if the archive
 *   cannot be opened.
 * - list_entries(), list_entries_with_sizes(), read_entry(),
 *   read_entries_batch(): return empty results on failure (no throw).
 *
 * ## :material-help: Thread Safety
 *
 * Instances are **not** thread-safe. Each thread should use its own
 * ArchiveService, or callers must synchronize externally. The bit7z
 * library singleton is initialized with a C++11 magic static, so the
 * first call from any thread is safe.
 *
 * @see InstallationService, FomodService
 */
class ArchiveService
{
public:
    /**
     * @struct EntryListing
     * @brief Result of a header-only archive scan.
     *
     * Bundles the ordered entry paths with a size lookup table so callers
     * can inspect archive contents without extracting anything.
     *
     * Paths in `paths` preserve the original casing and separators from
     * the archive. Keys in `sizes` are normalized (lowercase, forward-slash)
     * so lookups are case-insensitive.
     */
    struct EntryListing
    {
        std::vector<std::string> paths; /**< Entry paths in archive order (original casing) */
        std::unordered_map<std::string, uint64_t>
            sizes; /**< Normalized path to uncompressed size in bytes */

        EntryListing() = default;
        EntryListing(EntryListing&&) noexcept = default;
        EntryListing& operator=(EntryListing&&) noexcept = default;
    };

    /**
     * @brief Extract every entry in an archive to a destination directory.
     *
     * Tries bit7z first for supported formats (`.7z`, `.rar`, `.001`),
     * then falls back to libarchive. The destination directory is created
     * if it does not exist.
     *
     * On the libarchive path, timestamps, permissions, ACLs, and file
     * flags are preserved from the archive metadata. Warnings (e.g.
     * unsupported ACL types) are logged but do not stop extraction;
     * hard errors throw immediately.
     *
     * @param archivePath Path to the archive file.
     * @param destinationPath Output directory for extracted files.
     * @throw std::runtime_error if the archive cannot be opened or a
     *        fatal read error occurs.
     */
    void extract(const std::string& archivePath, const std::string& destinationPath);

    /**
     * @brief Extract only entries accepted by a filter predicate.
     *
     * Uses libarchive in a single sequential pass (no bit7z path).
     * Each entry path is passed to @p filter exactly once; entries for
     * which it returns `false` are skipped without decompression, so
     * filtering is cheap even on large archives.
     *
     * Only timestamps are preserved (no ACLs/permissions), since this
     * is used for selective mod file extraction where full metadata is
     * unnecessary.
     *
     * @param archivePath Path to the archive file.
     * @param destinationPath Output directory for extracted files.
     * @param filter Predicate receiving the raw entry path as stored in
     *        the archive (original casing/separators).
     * @throw std::runtime_error if the archive cannot be opened.
     */
    void extract_filtered(const std::string& archivePath,
                          const std::string& destinationPath,
                          std::function<bool(const std::string&)> filter);

    /**
     * @brief Extract entries whose path starts with a given prefix.
     *
     * Primary method for FOMOD sub-folder extraction. Comparison is
     * case-insensitive with normalized separators (backslashes converted
     * to forward slashes before matching).
     *
     * On bit7z-supported formats, collects matching item indices first
     * and extracts them in a single batch call. Falls back to
     * extract_filtered() on other formats or on bit7z failure.
     *
     * @param archivePath Path to the archive file.
     * @param destinationPath Output directory for extracted files.
     * @param prefix Case-insensitive path prefix to match (e.g.
     *        `"textures/actors"`).
     * @throw std::runtime_error if bit7z fails and the libarchive
     *        fallback cannot open the archive (propagated from
     *        extract_filtered()).
     */
    void extract_prefix(const std::string& archivePath,
                        const std::string& destinationPath,
                        const std::string& prefix);

    /**
     * @brief List all entry paths in an archive (header-only, no extraction).
     *
     * Convenience wrapper around list_entries_with_sizes() that discards
     * the size information. Useful when callers only need to check which
     * files exist (e.g. detecting FOMOD structure).
     *
     * @param archivePath Path to the archive file.
     * @return Ordered list of entry paths (original casing), or empty
     *         vector on failure.
     */
    std::vector<std::string> list_entries(const std::string& archivePath);

    /**
     * @brief List entries with their uncompressed sizes.
     *
     * Performs a header-only scan of the archive without extracting any
     * data. Both backends skip past file data blocks, so this is fast
     * even on multi-gigabyte archives.
     *
     * Sizes are keyed by normalized path (lowercase, forward-slash
     * separators) for case-insensitive lookup. The `paths` vector
     * preserves original casing.
     *
     * @param archivePath Path to the archive file.
     * @return EntryListing with paths and size map, or empty on failure.
     */
    EntryListing list_entries_with_sizes(const std::string& archivePath);

    /**
     * @brief Read a single entry's contents into memory.
     *
     * Scans the archive for an entry matching @p entryName using
     * case-insensitive, separator-normalized comparison. Returns
     * immediately once the entry is found without reading the rest
     * of the archive.
     *
     * On bit7z formats, extracts to an in-memory buffer by item index.
     * On libarchive formats, streams sequentially until the match is
     * found. Unlike other methods, **no libarchive fallback** is
     * attempted if bit7z fails -- the method returns empty.
     *
     * @param archivePath Path to the archive file.
     * @param entryName Entry path to read (case-insensitive match).
     * @return File contents as a byte vector, or empty vector if the
     *         entry was not found, bit7z failed, or the archive could
     *         not be opened.
     */
    std::vector<char> read_entry(const std::string& archivePath, const std::string& entryName);

    /**
     * @brief Read multiple entries in a single pass.
     *
     * Optimized for batch reads where opening the archive once is much
     * cheaper than repeated read_entry() calls. Three strategies are
     * used depending on the backend and archive structure:
     *
     * 1. **bit7z solid archive, 4+ entries**: extracts the matched
     *    subset to a temporary directory, reads files from disk, then
     *    cleans up. This avoids repeated decompression of the solid
     *    block (which starts from byte 0 on each random access).
     * 2. **bit7z non-solid archive**: extracts each matched item
     *    directly to an in-memory buffer by index.
     * 3. **libarchive**: single sequential pass, collecting matches
     *    and stopping early once all requested entries are found.
     *
     * @param archivePath Path to the archive file.
     * @param entryNames Set of normalized (lowercase, forward-slash)
     *        entry paths to read.
     * @return Map of normalized path to file contents. Entries not
     *         found in the archive are absent from the map.
     */
    std::unordered_map<std::string, std::vector<char>> read_entries_batch(
        const std::string& archivePath, const std::unordered_set<std::string>& entryNames);

    /**
     * @brief Create a zip archive from a directory.
     *
     * Recursively adds every file under @p folderPath using default
     * deflate compression. Directory entries are not stored explicitly
     * (they are implied by file paths). Parent directories of
     * @p outputZipPath are created if needed.
     *
     * Files are streamed in 8 KiB chunks so memory usage stays
     * constant regardless of individual file sizes.
     *
     * @param folderPath Source directory to compress.
     * @param outputZipPath Destination zip file path.
     * @throw std::runtime_error if the output file cannot be opened.
     */
    void create_zip(const std::string& folderPath, const std::string& outputZipPath);

private:
    /**
     * @brief libarchive extraction implementation for extract().
     *
     * Handles the full read-header / write-to-disk loop with progress
     * logging every 100 entries. Preserves timestamps, permissions,
     * ACLs, and file flags.
     *
     * @param archivePath Path to the archive file.
     * @param destinationPath Output directory.
     * @throw std::runtime_error on open or fatal read failure.
     */
    void extract_with_libarchive(const std::string& archivePath,
                                 const std::string& destinationPath);

    /**
     * @brief Check whether an archive should use bit7z based on extension.
     *
     * @param archivePath Path to the archive file.
     * @return `true` for `.7z`, `.rar`, and `.001` extensions
     *         (case-insensitive).
     */
    static bool use_bit7z(const std::string& archivePath);
};

}  // namespace mo2core
