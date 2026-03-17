#pragma once

#include <crow.h>
#include <string>

namespace mo2server
{

/**
 * @class MultipartHandler
 * @brief Multipart form-data parsing and file saving.
 * @author Alex (https://github.com/lextpf)
 * @ingroup MultipartHandler
 *
 * Static utility for extracting file uploads and text fields from
 * Crow multipart requests. Used by InstallationController to receive
 * archive uploads from the React frontend.
 *
 * ## :material-upload: File Handling
 *
 * Uploaded files are saved to the system temp directory with a random
 * 5-digit numeric suffix and the original file extension preserved
 * (important for archive format detection by ArchiveService). The
 * random range is 10000-99999, so collisions are possible under
 * concurrent uploads -- a collision silently overwrites the existing
 * temp file. If this becomes a concern, callers could add a
 * timestamp or UUID to the filename, or use a per-request temp
 * directory.
 *
 * ## :material-alert-circle-outline: Failure Behavior
 *
 * If the write to disk fails (e.g. disk full, permission denied),
 * the partial temp file is deleted and UploadedFile::temp_path is
 * cleared -- callers receive an empty result identical to "part not
 * found".  No exception is thrown.
 *
 * ## :material-shield-outline: Filename Handling
 *
 * The original filename from the multipart `Content-Disposition`
 * header is stored in UploadedFile::filename but is **not** used
 * to construct the temp file path (only the extension is reused).
 * No sanitization is performed on the stored filename -- callers
 * should not use it to construct filesystem paths without
 * validation.
 *
 * ## :material-upload-outline: Upload Limits
 *
 * No explicit body-size limit is enforced by this handler. The
 * effective limit depends on Crow's configuration (default is
 * unbounded). For production use, consider setting Crow's max
 * payload size to prevent memory exhaustion from oversized uploads.
 *
 * ## :material-code-tags: Usage Example
 *
 * ```cpp
 * crow::multipart::message msg(req);
 * auto file = MultipartHandler::save_uploaded_file(msg, "archive");
 * auto mod_name = MultipartHandler::get_part_value(msg, "modName");
 * // file.temp_path -> "C:/tmp/mo2_upload_38291.7z"
 * ```
 *
 * @see InstallationController
 */

/**
 * @struct UploadedFile
 * @brief Metadata for a saved multipart file upload.
 * @ingroup MultipartHandler
 */
struct UploadedFile
{
    std::string filename;            ///< Original filename from the upload
    std::string temp_path;           ///< Absolute path to the saved temp file
    std::string original_extension;  ///< File extension including the dot (e.g. `.7z`)
};

class MultipartHandler
{
public:
    /**
     * @brief Save an uploaded file from a multipart request to a temp file.
     *
     * Searches the multipart message for a part matching @p part_name,
     * writes its body to a temp file, and returns the metadata.
     * Returns an empty UploadedFile if the part is not found or the
     * write to disk fails.
     *
     * @param msg The Crow multipart message.
     * @param part_name Form field name to look for (e.g. `"archive"`).
     * @return UploadedFile with temp_path set, or empty on failure.
     */
    static UploadedFile save_uploaded_file(const crow::multipart::message& msg,
                                           const std::string& part_name);

    /**
     * @brief Extract a text field value from a multipart request.
     *
     * @param msg The Crow multipart message.
     * @param part_name Form field name to look for.
     * @return The field value as a string, or empty string if not found.
     */
    static std::string get_part_value(const crow::multipart::message& msg,
                                      const std::string& part_name);
};

}  // namespace mo2server
