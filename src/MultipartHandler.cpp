#include "MultipartHandler.h"
#include <filesystem>
#include <fstream>
#include <random>

namespace fs = std::filesystem;

namespace mo2server
{

UploadedFile MultipartHandler::save_uploaded_file(const crow::multipart::message& msg,
                                                  const std::string& part_name)
{
    UploadedFile result;

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

        // Temp filename uses a random 5-digit suffix to reduce collision risk.
        // On collision the file is silently overwritten (no uniqueness guarantee).
        std::random_device rd;
        std::mt19937 gen(rd());
        std::uniform_int_distribution<> dist(10000, 99999);
        auto temp_name = "mo2_upload_" + std::to_string(dist(gen)) + result.original_extension;
        result.temp_path = (fs::temp_directory_path() / temp_name).string();

        // Write the upload body to disk.
        std::ofstream ofs(result.temp_path, std::ios::binary);
        ofs.write(part.body.data(), static_cast<std::streamsize>(part.body.size()));

        // Check stream state before and after close to catch both write and flush errors
        bool write_ok = ofs.good();
        ofs.close();

        // Clear temp_path on write failure so callers get an empty result
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
