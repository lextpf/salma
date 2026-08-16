#include "StaticFileHandler.hpp"
#include <filesystem>
#include "Utils.hpp"

namespace fs = std::filesystem;

// StaticFileHandler - serves web/dist for every path Crow did not route to an
// API handler, with an index.html fallback for client-side routing.
//
// serve() is the only containment check on this path. It ends in
// set_static_file_info_unsafe, which does no traversal checking of its own, so
// the guard below is what keeps a request inside the static root. Both the
// requested file and the index.html fallback are re-validated; do not add a
// third path that reaches the response without going through the same test.
//
// Return codes: 403 for a path resolving outside the root, 404 when
// canonicalization throws or the file cannot be opened, otherwise 200 with the
// content type from get_content_type, which defaults to
// application/octet-stream.

namespace mo2server
{

StaticFileHandler::StaticFileHandler(const std::string& static_dir)
    : static_dir_(static_dir)
{
}

crow::response StaticFileHandler::serve(const std::string& path)
{
    auto file_path = fs::path(static_dir_) / (path.empty() ? "index.html" : path);

    // Traversal guard. Canonicalize both the static root and the requested
    // file, take the file relative to the root, and reject an empty result or
    // one starting with "..". Comparing relative paths, rather than string
    // prefixes, is what stops a sibling directory matching: "web-extra/app.js"
    // relative to "web" is "../web-extra/app.js", while a prefix test on "web"
    // would accept it.
    //
    // The ".." test is itself a string prefix test, so a real entry whose name
    // begins with ".." is rejected as traversal. web/dist holds no such name,
    // and that is the safe direction to be wrong in.
    //
    // weakly_canonical rather than canonical, because the requested file need
    // not exist: a miss falls through to the index.html fallback below.
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

    // A missing path or a directory serves index.html, so the React router can
    // handle the route in the browser.
    if (!fs::exists(file_path) || fs::is_directory(file_path))
    {
        file_path = fs::path(static_dir_) / "index.html";
        // The fallback is a fresh path, so it gets the same containment test.
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
