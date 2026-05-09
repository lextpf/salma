#include "MultipartHandler.hpp"
#include "Utils.hpp"

#include <filesystem>
#include <fstream>

namespace fs = std::filesystem;

// MultipartHandler - pull one file part or one field value out of a parsed
// Crow multipart body.
//
// save_uploaded_file writes the part to the system temp directory and hands the
// caller the path. The file is the caller's from that moment: nothing here ever
// deletes it again except on its own write failure, where temp_path comes back
// empty. InstallationController owns the matching cleanup rules.
//
// Neither function reports why it found nothing. An empty temp_path means "no
// usable file part", and an empty string from get_part_value means "no such
// part", with no distinction from a part that was present and empty.

namespace mo2server
{

UploadedFile MultipartHandler::save_uploaded_file(const crow::multipart::message& msg,
                                                  const std::string& part_name)
{
    UploadedFile result;

    // The first part that both matches `part_name` and carries a `filename`
    // parameter wins, and the loop stops there whether the write succeeded or
    // not. A matching part without a filename is skipped rather than treated as
    // an error, so a form that repeats the field name keeps the first file and
    // ignores the later copies instead of overwriting the temp file.
    for (const auto& part : msg.parts)
    {
        auto it = part.headers.find("Content-Disposition");
        if (it == part.headers.end())
            continue;

        auto& disposition = it->second;
        if (disposition.params.find("name") == disposition.params.end())
            continue;
        if (disposition.params.at("name") != part_name)
            continue;

        if (disposition.params.find("filename") != disposition.params.end())
        {
            result.filename = disposition.params.at("filename");
        }

        if (result.filename.empty())
            continue;

        result.original_extension = fs::path(result.filename).extension().string();
        // The extension is appended to a temp filename, so keep only
        // alphanumerics, dot and hyphen. A crafted extension such as
        // ".../../etc/foo" would otherwise steer the write out of the temp
        // directory.
        std::erase_if(
            result.original_extension,
            [](char c)
            { return !std::isalnum(static_cast<unsigned char>(c)) && c != '.' && c != '-'; });

        // 12 hex characters, so 16^12 possible names. Uploads run on
        // overlapping Crow worker threads and the name is never checked for an
        // existing file, so the width is the only thing preventing one upload
        // from clobbering another.
        auto temp_name = "mo2_upload_" + mo2core::random_hex_string(12) + result.original_extension;
        result.temp_path = (fs::temp_directory_path() / temp_name).string();

        std::ofstream ofs(result.temp_path, std::ios::binary);
        ofs.write(part.body.data(), static_cast<std::streamsize>(part.body.size()));

        // Sample the state both before and after close: the write can fail
        // here, and the flush can fail inside close().
        bool write_ok = ofs.good();
        ofs.close();

        // On failure, remove the partial file and clear temp_path, which is the
        // only signal the caller gets.
        if (!write_ok || ofs.fail())
        {
            std::error_code ec;
            fs::remove(result.temp_path, ec);
            result.temp_path.clear();
        }

        break;
    }

    return result;
}

std::string MultipartHandler::get_part_value(const crow::multipart::message& msg,
                                             const std::string& part_name)
{
    for (const auto& part : msg.parts)
    {
        auto it = part.headers.find("Content-Disposition");
        if (it == part.headers.end())
            continue;

        auto& disposition = it->second;
        if (disposition.params.find("name") == disposition.params.end())
            continue;
        if (disposition.params.at("name") != part_name)
            continue;

        return part.body;
    }
    return "";
}

}  // namespace mo2server
