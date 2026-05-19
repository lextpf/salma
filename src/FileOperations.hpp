#pragma once

#include <filesystem>
#include <vector>
#include "Types.h"

namespace mo2core
{

/**
 * @class FileOperations
 * @brief Queued file copy operations with priority sorting.
 * @author Alex (https://github.com/lextpf)
 * @ingroup FileOperations
 *
 * Collects individual file and folder copy operations into a queue,
 * then executes them all at once sorted by priority. This is the
 * mechanism FOMOD installers use to apply file-level overrides:
 * higher-priority operations overwrite lower-priority ones when
 * multiple options install to the same destination path.
 *
 * ## :material-sort-ascending: Priority Sorting
 *
 * Each FileOperation carries a `priority` field (default 0).
 * When execute() is called, operations are `stable_sort`-ed by
 * ascending priority so that higher-priority files are copied last
 * and win on conflict -- matching MO2's FOMOD behavior.  Among
 * equal-priority operations, `stable_sort` preserves insertion
 * order, so callers that add ops in XML document order get the
 * correct tiebreaker for free.  (FomodService uses its own sort
 * with an explicit `document_order` field for additional safety.)
 *
 * ## :material-file-tree: Static Utilities
 *
 * Three static methods (copy_file, copy_folder, copy_directory_contents)
 * are used independently by InstallationService and FomodService for
 * non-queued copies (e.g. flat archive extraction, single-folder mods).
 * These create destination directories automatically. All errors --
 * including `create_directories()` failures and iteration errors --
 * are caught, logged, and cause the method to return without throwing.
 *
 * ## :material-code-tags: Usage Examples
 *
 * ### Queue FOMOD file operations and execute
 * ```cpp
 * FileOperations ops;
 * ops.add({FileOpType::File, "tmp/textures/body.dds", "mod/textures/body.dds", 0});
 * ops.add({FileOpType::File, "tmp/textures/body.dds", "mod/textures/body.dds", 1});
 * ops.execute();  // priority 1 overwrites priority 0
 * ```
 *
 * ### Copy a directory without queuing
 * ```cpp
 * FileOperations::copy_directory_contents("tmp/extracted", "mods/MyMod");
 * ```
 *
 * ## :material-link: Symlinks and Special Files
 *
 * `copy_folder()` explicitly **skips** symlinks during recursive
 * traversal (`is_symlink()` entries are `continue`-d). `copy_file()`
 * for individual files delegates to `std::filesystem::copy_file` with
 * `overwrite_existing`, which follows symlinks -- the target file
 * content is copied, not the link itself. Special files (devices,
 * FIFOs) are not expected in mod archives and their behavior with
 * `std::filesystem::copy_file` is platform-defined.
 *
 * ## :material-help: Thread Safety
 *
 * Instances are **not** thread-safe. The static copy methods are
 * safe to call from any thread (they only use local state and the
 * thread-safe Logger singleton).
 *
 * @see FomodService, InstallationService
 */
class FileOperations
{
public:
    /**
     * @brief Enqueue a file operation for deferred execution.
     *
     * The operation is appended to the internal queue. No I/O is
     * performed until execute() is called.
     *
     * @param op File operation to enqueue (moved into the queue).
     */
    void add(FileOperation op);

    /**
     * @brief Execute all queued operations in priority order.
     *
     * Stable-sorts the queue by ascending FileOperation::priority,
     * then copies each entry. Files use copy_file(); folders use
     * copy_folder(). Individual failures are caught, logged, and
     * skipped -- one failure does not abort the remaining operations.
     *
     * The queue is **always** cleared after execution, even if some
     * operations failed.
     */
    void execute();

    /**
     * @brief Discard all queued operations without executing.
     */
    void clear();

    /**
     * @brief Return the number of queued operations.
     */
    int count() const;

    /**
     * @brief Copy a single file, creating parent directories as needed.
     *
     * Overwrites the destination if it already exists. Logs and
     * returns silently if the source file is missing. All errors --
     * including `create_directories()` failures and file copy errors
     * -- are caught, logged, and cause the method to return without
     * throwing.
     *
     * @param src Source file path (must exist, or the call is a no-op).
     * @param dst Destination file path.
     * @throw Does not throw.
     */
    static void copy_file(const std::filesystem::path& src, const std::filesystem::path& dst);

    /**
     * @brief Recursively copy a folder tree.
     *
     * Recreates the full directory structure of @p src under @p dst,
     * overwriting existing files. Logs and returns silently if the
     * source folder is missing. All errors -- including
     * `create_directories()` failures and iteration errors -- are
     * caught, logged, and cause the method to return without throwing.
     *
     * @param src Source directory (must exist, or the call is a no-op).
     * @param dst Destination directory.
     * @throw Does not throw.
     */
    static void copy_folder(const std::filesystem::path& src, const std::filesystem::path& dst);

    /**
     * @brief Copy the immediate contents of a directory (one level deep for folders).
     *
     * Unlike copy_folder(), this copies *into* @p dst rather than
     * recreating the source root. Files are copied directly;
     * subdirectories are copied recursively via copy_folder().
     *
     * Used by InstallationService to flatten a single-subfolder mod
     * structure into the mod root.
     *
     * All errors -- including `create_directories()` failures and
     * iteration errors -- are caught, logged, and cause the method
     * to return without throwing.
     *
     * @param src Source directory whose contents are copied (must exist).
     * @param dst Destination directory.
     * @throw Does not throw.
     */
    static void copy_directory_contents(const std::filesystem::path& src,
                                        const std::filesystem::path& dst);

    /**
     * @brief Move the immediate contents of a directory.
     *
     * Same shape as copy_directory_contents, but tries `std::filesystem::rename`
     * first and only falls back to copy + remove when rename fails with
     * `errc::cross_device_link` (source and destination on different volumes).
     * Used as the final unfomod -> mod_path step so the install does not
     * keep two full copies of the FOMOD output on disk simultaneously.
     *
     * The sticky disk-full flag (see disk_full_encountered()) is set on
     * copy fallbacks just like the copy_* functions; rename failures with
     * errc::no_space_on_device also set it.
     *
     * @param src Source directory whose contents are moved (must exist).
     * @param dst Destination directory.
     * @throw Does not throw.
     */
    static void move_directory_contents(const std::filesystem::path& src,
                                        const std::filesystem::path& dst);

    /**
     * @brief True iff any copy or move call has hit "no space on device".
     *
     * Sticky across calls; the flag is process-global, set whenever a
     * `std::filesystem::filesystem_error` whose code matches
     * `std::errc::no_space_on_device` is caught. Callers (FomodService,
     * InstallationService) check this after a batch of operations to
     * surface disk-full as a hard install failure instead of letting
     * the missing files masquerade as a successful partial install.
     */
    static bool disk_full_encountered();

    /**
     * @brief Reset the sticky disk-full flag.
     *
     * Called at the start of an install so a prior install's disk-full
     * does not poison the next one. Tests also use this between cases.
     */
    static void reset_disk_full();

private:
    std::vector<FileOperation> ops_; /**< Queued operations awaiting execute() */
};

}  // namespace mo2core
