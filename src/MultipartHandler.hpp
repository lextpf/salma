#pragma once

#include <crow.h>
#include <string>

namespace mo2server
{

/**
 * @struct UploadedFile
 * @brief Describes a multipart file saved to temporary storage.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup MultipartHandler
 *
 * An empty `temp_path` indicates every failure. The caller owns a successful
 * temporary file and must remove it.
 */
struct UploadedFile
{
    std::string filename;            ///< Unsanitized `Content-Disposition` filename.
    std::string temp_path;           ///< Absolute path, or empty on failure.
    std::string original_extension;  ///< Allowlisted extension, including the dot.
};

/**
 * @class MultipartHandler
 * @brief Reads multipart fields and saves uploaded files.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup MultipartHandler
 *
 * Files use `mo2_upload_{12-hex}{extension}` in the system temporary directory.
 * The original filename never enters the temporary path. Only alphanumeric,
 * dot, and hyphen characters remain in the extension.
 *
 * This type has no upload-size limit. The caller must apply one before parsing
 * because Crow has already buffered the request body.
 *
 * @warning Treat `UploadedFile::filename` and all text fields as untrusted input.
 * @see InstallationController
 */
class MultipartHandler
{
public:
    /**
     * @fn UploadedFile save_uploaded_file(const crow::multipart::message&, const std::string&)
     * @brief Never derives the temporary path from the client filename.
     * @author Alex (<https://github.com/lextpf>)
     *
     * A failed write removes the partial file. Locating the system temporary
     * directory can raise `std::filesystem::filesystem_error`.
     *
     * @param msg Multipart message to scan in document order.
     * @param part_name Form field name to match.
     * @return Metadata with `temp_path` set only after a complete write.
     */
    static UploadedFile save_uploaded_file(const crow::multipart::message& msg,
                                           const std::string& part_name);

    /**
     * @fn static std::string get_part_value(const crow::multipart::message&, const std::string&)
     * @brief Does not distinguish a missing field from an empty field.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The value is not decoded, trimmed, or length-limited. Missing and empty
     * fields both return an empty string.
     *
     * @param msg Multipart message to scan.
     * @param part_name Form field name to match.
     * @return Raw field content, or an empty string.
     */
    static std::string get_part_value(const crow::multipart::message& msg,
                                      const std::string& part_name);
};

}  // namespace mo2server
