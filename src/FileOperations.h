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
 * These create destination directories automatically. Individual file
 * copy errors are caught and logged, but `create_directories()` calls
 * may throw `std::filesystem::filesystem_error` if the destination
 * path is invalid or inaccessible (e.g. permission denied, read-only
 * filesystem). Callers that need full fault tolerance should wrap
 * calls in a try/catch.
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
 * `std::filesystem::copy_file` with `overwrite_existing` follows
 * symlinks -- the target file content is copied, not the link itself.
 * Special files (devices, FIFOs) are not expected in mod archives
 * and their behavior with `copy_file` is platform-defined.
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
     * returns silently if the source file is missing. Individual
     * copy errors are caught and logged. However,
     * `create_directories()` may throw if the destination path is
     * invalid or inaccessible.
     *
     * @param src Source file path (must exist, or the call is a no-op).
     * @param dst Destination file path.
     * @throw std::filesystem::filesystem_error if parent directory
     *        creation fails.
     */
    static void copy_file(const std::filesystem::path& src, const std::filesystem::path& dst);

    /**
     * @brief Recursively copy a folder tree.
     *
     * Recreates the full directory structure of @p src under @p dst,
     * overwriting existing files. Logs and returns silently if the
     * source folder is missing. Individual file copy errors are
     * caught and logged. However, `create_directories()` may throw
     * if the destination path is invalid or inaccessible.
     *
     * @param src Source directory (must exist, or the call is a no-op).
     * @param dst Destination directory.
     * @throw std::filesystem::filesystem_error if directory creation
     *        fails.
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
     * @param src Source directory whose contents are copied (must exist).
     * @param dst Destination directory.
     * @throw std::filesystem::filesystem_error if directory creation
     *        fails or @p src cannot be iterated.
     */
    static void copy_directory_contents(const std::filesystem::path& src,
                                        const std::filesystem::path& dst);

private:
    std::vector<FileOperation> ops_;  ///< Queued operations awaiting execute()
};

}  // namespace mo2core
