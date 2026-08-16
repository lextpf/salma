#pragma once

#include <crow.h>
#include <string>

namespace mo2server
{

/**
 * @struct UploadedFile
 * @brief Metadata for a saved multipart file upload.
 * @author Alex (https://github.com/lextpf)
 * @ingroup MultipartHandler
 *
 * `temp_path` is the only failure flag: it is empty on every failure
 * path. Test `temp_path.empty()`, never `filename.empty()`, because the
 * failures do not all look alike:
 *
 * - No matching part, or a matching part with no `filename` parameter:
 *   all three fields are empty.
 * - The write to disk failed: `filename` and `original_extension` hold
 *   the values read from the part, and only `temp_path` is empty.
 *
 * The struct owns no filesystem resource. Deleting the temp file is the
 * caller's job; nothing here removes it once the write has succeeded.
 */
struct UploadedFile
{
    std::string filename;            ///< Original name from `Content-Disposition`. Not sanitized.
    std::string temp_path;           ///< Absolute temp-file path. Empty on any failure.
    std::string original_extension;  ///< Sanitized extension with the dot, for example `.7z`.
};

/**
 * @class MultipartHandler
 * @brief Multipart form-data parsing and file saving.
 * @author Alex (https://github.com/lextpf)
 * @ingroup MultipartHandler
 *
 * Static utility for extracting file uploads and text fields from Crow
 * multipart requests. InstallationController uses it to receive archive
 * uploads from the React frontend.
 *
 * Both members are static and hold no state, so both are safe to call
 * from several Crow worker threads at once. Each call builds its own
 * temp filename from a fresh random suffix.
 *
 * ## :material-upload: File handling
 *
 * An uploaded file goes to the system temp directory, named
 * `mo2_upload_{12-hex}{ext}` under
 * `std::filesystem::temp_directory_path()`. The body is written in
 * binary mode, byte for byte, with no transformation.
 *
 * The original extension is preserved because the engine detects the
 * archive format from it. The 12 hex characters give 16^12
 * (~2.8 * 10^14) possibilities, so collisions stay negligible even under
 * heavy concurrent uploads.
 *
 * Parts are scanned in document order. The first part whose
 * `Content-Disposition` name matches and that also carries a `filename`
 * parameter wins; a name-matching part with no `filename` parameter is
 * skipped and the scan continues. At most one file is written per call.
 *
 * ## :material-alert-circle-outline: Failure behavior
 *
 * A failed write (disk full, permission denied) deletes the partial temp
 * file and clears UploadedFile::temp_path. See the UploadedFile block
 * for how that result differs from a part that was never found, and why
 * `temp_path` is the only reliable success test.
 *
 * The write path does not throw: the stream state is checked instead,
 * and the cleanup `remove` uses the `std::error_code` overload. Exactly
 * one call inside the function can throw:
 * `std::filesystem::temp_directory_path()` uses the throwing overload
 * and raises `std::filesystem::filesystem_error` when the temp directory
 * cannot be determined. InstallationController::handle_upload wraps the
 * call in a try/catch that turns that into an HTTP 500.
 *
 * ## :material-shield-outline: Filename handling
 *
 * The temp path is built from the random suffix and the extension only.
 * The original filename is stored in UploadedFile::filename and is never
 * part of any path this class builds. Nothing sanitizes it, so a caller
 * must not build a filesystem path from it without validating it first.
 *
 * `original_extension` is sanitized: every character that is not
 * alphanumeric, a dot or a hyphen is stripped before the extension is
 * appended to the temp filename. That is what stops a crafted filename
 * such as `evil.../../etc/passwd` from steering the write out of the
 * temp directory. The rule is a whitelist over single characters, so it
 * says nothing about the rest of the multipart input; treat every other
 * field as untrusted.
 *
 * ## :material-upload-outline: Upload limits
 *
 * This handler enforces no body-size limit of its own. The effective cap
 * is 8 GiB, applied upstream in
 * `InstallationController::parse_and_validate_upload`, which returns
 * HTTP 413 when the body exceeds it. Crow's `stream_threshold` in
 * `main.cpp` controls buffering rather than the cap, and has to stay
 * equal to that 8 GiB value: `handle_upload` reads `req.body`, which
 * Crow populates only for bodies under the threshold.
 *
 * Memory cost is not bounded here. The archive is held in `req.body` and
 * again in the Crow multipart copy, so a large upload needs headroom for
 * both before the first byte reaches disk.
 *
 * ## :material-code-tags: Usage example
 *
 * ```cpp
 * crow::multipart::message msg(req);
 * auto file = MultipartHandler::save_uploaded_file(msg, "file");
 * auto mod_name = MultipartHandler::get_part_value(msg, "modName");
 * // file.temp_path -> "C:/tmp/mo2_upload_9f3c1ab77e02.7z"
 * ```
 *
 * @see InstallationController
 */
class MultipartHandler
{
public:
    /**
     * @brief Save an uploaded file from a multipart request to a temp file.
     *
     * Searches the multipart message for a part matching @p part_name
     * that also carries a `filename` parameter, writes its body to a new
     * temp file, and returns the metadata. Nothing is written when no
     * such part exists.
     *
     * On success this leaves one file under the system temp directory.
     * The caller owns that file and has to delete it.
     *
     * @param msg The Crow multipart message.
     * @param part_name Form field name to look for. The upload route uses
     *        `"file"`.
     * @return UploadedFile with `temp_path` set on success. On failure
     *         `temp_path` is empty; the other two fields may still be set.
     * @throw std::filesystem::filesystem_error When the system temp
     *        directory cannot be determined. No other exception is raised
     *        by this function beyond allocation failure.
     */
    static UploadedFile save_uploaded_file(const crow::multipart::message& msg,
                                           const std::string& part_name);

    /**
     * @brief Extract a text field value from a multipart request.
     *
     * Returns the raw part body with no trimming, decoding or length
     * limit. An absent field and a present-but-empty field are both
     * reported as an empty string; the two cannot be distinguished.
     *
     * @param msg The Crow multipart message.
     * @param part_name Form field name to look for.
     * @return The field value as a string, or empty string if not found.
     */
    static std::string get_part_value(const crow::multipart::message& msg,
                                      const std::string& part_name);
};

}  // namespace mo2server
