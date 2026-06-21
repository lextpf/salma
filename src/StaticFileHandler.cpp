#include "StaticFileHandler.hpp"
#include <filesystem>
#include "Utils.hpp"

namespace fs = std::filesystem;

namespace mo2server
{

StaticFileHandler::StaticFileHandler(const std::string& static_dir)
    : static_dir_(static_dir)
{
}

crow::response StaticFileHandler::serve(const std::string& path)
{
    auto file_path = fs::path(static_dir_) / (path.empty() ? "index.html" : path);

    // compare canonical relative paths because string prefixes accept sibling roots.
    // weak canonicalization permits a missing path to reach the SPA fallback.
    fs::path canonical_base, canonical_file;
    try
    {
        canonical_base = fs::weakly_canonical(static_dir_);
        canonical_file = fs::weakly_canonical(file_path);
    }
    catch (const fs::filesystem_error&)
    {
        return crow::response(404);
    }
    auto rel = canonical_file.lexically_relative(canonical_base);
    if (rel.empty() || rel.string().starts_with(".."))
    {
        return crow::response(403);
    }

    // let the browser router handle missing paths and directories.
    if (!fs::exists(file_path) || fs::is_directory(file_path))
    {
        file_path = fs::path(static_dir_) / "index.html";
        // validate the fallback as an independent path.
        try
        {
            auto fallback_canonical = fs::weakly_canonical(file_path);
            auto fallback_rel = fallback_canonical.lexically_relative(canonical_base);
            if (fallback_rel.empty() || fallback_rel.string().starts_with(".."))
            {
                return crow::response(403);
            }
        }
        catch (const fs::filesystem_error&)
        {
            return crow::response(404);
        }
    }

    auto ext = mo2core::to_lower(file_path.extension().string());
    auto content_type = get_content_type(ext);

    crow::response res;
    res.set_static_file_info_unsafe(file_path.string(), content_type);
    if (res.code == 404)
    {
        return crow::response(404);
    }
    return res;
}

std::string StaticFileHandler::get_content_type(const std::string& extension)
{
    static const std::unordered_map<std::string, std::string> types = {
        {".html", "text/html"},
        {".css", "text/css"},
        {".js", "application/javascript"},
        {".json", "application/json"},
        {".png", "image/png"},
        {".jpg", "image/jpeg"},
        {".jpeg", "image/jpeg"},
        {".gif", "image/gif"},
        {".svg", "image/svg+xml"},
        {".ico", "image/x-icon"},
        {".woff", "font/woff"},
        {".woff2", "font/woff2"},
        {".ttf", "font/ttf"},
        {".map", "application/json"},
    };

    auto it = types.find(extension);
    if (it != types.end())
        return it->second;
    return "application/octet-stream";
}

}  // namespace mo2server
