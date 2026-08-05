#pragma once

#include <crow.h>
#include <string>

namespace mo2server
{

/**
 * @class StaticFileHandler
 * @brief Serves the React SPA from a fixed directory.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup StaticFileHandler
 *
 * Missing files and directories fall back to `index.html`. Canonical paths are
 * checked before both the requested file and the fallback are served. The object
 * is immutable after construction and supports concurrent requests.
 *
 * ### :material-transit-connection-variant: Request flow
 *
 * ```mermaid
 * flowchart TD
 *     request --> path[canonicalize under root]
 *     path -->|outside root| reject[403]
 *     path -->|regular file| serve[200]
 *     path -->|missing or directory| fallback[validate index.html]
 *     fallback -->|regular file| serve
 *     fallback -->|outside root| reject
 *     fallback -->|missing| missing[404]
 * ```
 */
class StaticFileHandler
{
public:
    /**
     * @fn StaticFileHandler::StaticFileHandler(const std::string& static_dir)
     * @brief Stores the root verbatim for request-time resolution.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Relative paths use the process working directory at request time.
     *
     * @param static_dir Absolute or relative asset directory.
     */
    explicit StaticFileHandler(const std::string& static_dir);

    /**
     * @fn crow::response StaticFileHandler::serve(const std::string& path)
     * @brief Validates both the requested path and fallback below the root.
     * @author Alex (<https://github.com/lextpf>)
     *
     * An empty, missing, or directory path selects `index.html`. Crow streams
     * the file without a size limit. Canonicalization errors become HTTP 404;
     * other filesystem probes can throw.
     *
     * @param path Path relative to the static root. Empty selects `index.html`.
     * @return HTTP 200, 403 for an escaped root, or 404 when no file can be served.
     */
    crow::response serve(const std::string& path);

private:
    // Static asset root, stored verbatim.
    std::string static_dir_;
    /**
     * @fn std::string StaticFileHandler::get_content_type(const std::string& extension)
     * @brief Choose a MIME type from the fixed asset-extension map.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @param extension Lowercase extension including the leading dot.
     * @return The mapped type, or application/octet-stream for an unknown extension.
     */
    std::string get_content_type(const std::string& extension);
};

}  // namespace mo2server
