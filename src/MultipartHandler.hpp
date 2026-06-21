#pragma once

#include <crow.h>
#include <string>

namespace mo2server
{

/**
 * @struct UploadedFile
 * @brief describes a multipart file saved to temporary storage.
 * @author Alex (https://github.com/lextpf)
 * @ingroup MultipartHandler
 *
 * an empty `temp_path` indicates every failure. the caller owns a successful
 * temporary file and must remove it.
 */
struct UploadedFile
{
    std::string filename;            ///< unsanitized `Content-Disposition` filename.
    std::string temp_path;           ///< absolute path, or empty on failure.
    std::string original_extension;  ///< allowlisted extension, including the dot.
};

/**
 * @class MultipartHandler
 * @brief reads multipart fields and saves uploaded files.
 * @author Alex (https://github.com/lextpf)
 * @ingroup MultipartHandler
 *
 * files use `mo2_upload_{12-hex}{extension}` in the system temporary directory.
 * the original filename never enters the temporary path. only alphanumeric,
 * dot, and hyphen characters remain in the extension.
 *
 * this type has no upload-size limit. the caller must apply one before parsing
 * because Crow has already buffered the request body.
 *
 * @warning treat `UploadedFile::filename` and all text fields as untrusted input.
 * @see InstallationController
 */
class MultipartHandler
{
public:
    /**
     * @fn UploadedFile save_uploaded_file(const crow::multipart::message&, const std::string&)
     * @brief never derives the temporary path from the client filename.
     * @author Alex (https://github.com/lextpf)
     *
     * a failed write removes the partial file. locating the system temporary
     * directory can raise `std::filesystem::filesystem_error`.
     *
     * @param msg multipart message to scan in document order.
     * @param part_name form field name to match.
     * @return metadata with `temp_path` set only after a complete write.
     */
    static UploadedFile save_uploaded_file(const crow::multipart::message& msg,
                                           const std::string& part_name);

    /**
     * @fn static std::string get_part_value(const crow::multipart::message&, const std::string&)
     * @brief does not distinguish a missing field from an empty field.
     * @author Alex (https://github.com/lextpf)
     *
     * the value is not decoded, trimmed, or length-limited. missing and empty
     * fields both return an empty string.
     *
     * @param msg multipart message to scan.
     * @param part_name form field name to match.
     * @return raw field content, or an empty string.
     */
    static std::string get_part_value(const crow::multipart::message& msg,
                                      const std::string& part_name);
};

}  // namespace mo2server
