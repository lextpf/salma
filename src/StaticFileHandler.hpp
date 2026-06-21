#pragma once

#include <crow.h>
#include <string>

namespace mo2server
{

/**
 * @class StaticFileHandler
 * @brief serves the React SPA from a fixed directory.
 * @author Alex (https://github.com/lextpf)
 * @ingroup StaticFileHandler
 *
 * missing files and directories fall back to `index.html`. canonical paths are
 * checked before both the requested file and the fallback are served. the object
 * is immutable after construction and supports concurrent requests.
 *
 * ### :material-transit-connection-variant: request flow
 *
 * ```mermaid
 * flowchart TD
 *     request --> path[canonicalize under root]
 *     path -->|outside root| reject[403]
 *     path -->|regular file| serve[200]
 *     path -->|missing or directory| fallback[validate index.html]
 *     fallback -->|regular file| serve
 *     fallback -->|outside or missing| missing[404]
 * ```
 */
class StaticFileHandler
{
public:
    /**
     * @fn StaticFileHandler::StaticFileHandler(const std::string& static_dir)
     * @brief stores the root verbatim for request-time resolution.
     * @author Alex (https://github.com/lextpf)
     *
     * relative paths use the process working directory at request time.
     *
     * @param static_dir absolute or relative asset directory.
     */
    explicit StaticFileHandler(const std::string& static_dir);

    /**
     * @fn crow::response StaticFileHandler::serve(const std::string& path)
     * @brief validates both the requested path and fallback below the root.
     * @author Alex (https://github.com/lextpf)
     *
     * an empty, missing, or directory path selects `index.html`. Crow streams
     * the file without a size limit. filesystem errors become HTTP 404.
     *
     * @param path path relative to the static root. empty selects `index.html`.
     * @return HTTP 200, 403 for an escaped root, or 404 when no file can be served.
     */
    crow::response serve(const std::string& path);

private:
    // static asset root, stored verbatim.
    std::string static_dir_;
    std::string get_content_type(const std::string& extension);
};

}  // namespace mo2server
