#include "StaticFileHandler.h"
#include <filesystem>
#include <fstream>
#include <sstream>
#include "Utils.h"

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

    // Path traversal guard: canonicalize both paths and verify the
    // resolved file is a child of the static root. Uses lexical
    // path iteration so "web-extra/" doesn't false-match "web/".
    auto canonical_base = fs::weakly_canonical(static_dir_);
    auto canonical_file = fs::weakly_canonical(file_path);
    auto rel = canonical_file.lexically_relative(canonical_base);
    if (rel.empty() || rel.string().starts_with(".."))
    {
        return crow::response(403);
    }

    // SPA fallback: non-existent paths and directories serve index.html
    // so client-side routing works.
    if (!fs::exists(file_path) || fs::is_directory(file_path))
    {
        file_path = fs::path(static_dir_) / "index.html";
    }

    if (!fs::exists(file_path))
    {
        return crow::response(404);
    }

    std::ifstream ifs(file_path, std::ios::binary);
    if (!ifs)
    {
        return crow::response(500);
    }

    std::ostringstream oss;
    oss << ifs.rdbuf();

    auto ext = mo2core::to_lower(file_path.extension().string());
    auto content_type = get_content_type(ext);

    crow::response res(200);
    res.set_header("Content-Type", content_type);
    res.body = oss.str();
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
