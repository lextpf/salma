#pragma once

#include <crow.h>
#include <string>

namespace mo2server
{

/**
 * @class StaticFileHandler
 * @brief Static file serving for the React SPA.
 * @author Alex (https://github.com/lextpf)
 * @ingroup StaticFileHandler
 *
 * Serves the built React frontend assets (HTML, CSS, JS, images,
 * fonts) from a directory on disk. Includes SPA fallback: requests
 * for non-existent paths return `index.html` so client-side routing
 * works correctly.
 *
 * ## :material-shield-check: Security
 *
 * Path traversal is prevented by canonicalizing both the static
 * root and the requested path via `weakly_canonical()`, then
 * computing `lexically_relative()`. If the relative path is empty
 * or its string starts with `..`, the request is rejected with 403.
 * The `lexically_relative()` output only produces `..` as whole
 * path components, so the string `starts_with` check is safe here.
 *
 * ## :material-file-document-outline: Content Types
 *
 * MIME types are resolved from the file extension. Supported types
 * include HTML, CSS, JS, JSON, common image formats (PNG, JPG, GIF,
 * SVG, ICO), and web fonts (WOFF, WOFF2, TTF). Unknown extensions
 * fall back to `application/octet-stream`.
 *
 * ## :material-cached: Caching
 *
 * No `Cache-Control` or `ETag` headers are set. Assets are served
 * fresh on every request, which is fine for the local dev server.
 * In production behind a reverse proxy, caching should be configured
 * at the proxy layer (e.g. nginx / Caddy).
 *
 * ## :material-alert-circle-outline: Edge Cases
 *
 * - If the static root directory is missing or unreadable, the
 *   `weakly_canonical()` call may throw (resulting in a 500), or
 *   if it succeeds the SPA fallback serves `index.html`, which
 *   will itself 404 if the root is truly absent.
 * - If `weakly_canonical()` throws (e.g. broken symlink, I/O error),
 *   the exception propagates uncaught. Crow's internal handler will
 *   return a 500 response.
 *
 * ## :material-code-tags: Usage Example
 *
 * ```cpp
 * StaticFileHandler handler("./web/dist");
 * CROW_ROUTE(app, "/<path>")([&](const std::string& path) {
 *     return handler.serve(path);
 * });
 * ```
 *
 * @see InstallationController
 */
class StaticFileHandler
{
public:
    /**
     * @brief Construct a handler rooted at the given directory.
     *
     * @param static_dir Absolute or relative path to the directory
     *        containing the built frontend assets.
     */
    explicit StaticFileHandler(const std::string& static_dir);

    /**
     * @brief Serve a file or fall back to index.html.
     *
     * Returns the requested file with the appropriate Content-Type
     * header. If the path does not exist or is a directory, returns
     * `index.html` (SPA fallback). Returns 403 on path traversal
     * attempts and 404 if index.html itself is missing.
     *
     * @param path Relative path within the static directory.
     * @return Crow response with file contents or error status.
     * @throw std::filesystem::filesystem_error if `weakly_canonical()`
     *        fails (e.g. broken symlink, I/O error). Crow's internal
     *        handler converts uncaught exceptions to a 500 response.
     */
    crow::response serve(const std::string& path);

private:
    std::string static_dir_;  ///< Root directory for static assets
    std::string GetContentType(const std::string& extension);
};

}  // namespace mo2server
