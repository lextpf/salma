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
 * Serves the built React frontend assets (HTML, CSS, JS, images, fonts)
 * from a directory on disk, with an SPA fallback: a request for a path
 * that does not exist returns `index.html` so client-side routing works.
 *
 * `main.cpp` chooses the root directory, not this class. It probes
 * `<exe dir>/web/dist` (the release layout) and falls back to
 * `<exe dir>/../web/dist` (the in-tree build layout, where the exe sits
 * in `build/bin/Release`). If neither exists, `main.cpp` logs a warning
 * and constructs the handler anyway, so every request then lands on the
 * "root missing" edge case below.
 *
 * ## :material-help: Thread safety
 *
 * `serve()` is safe to call from several Crow worker threads at once.
 * `main.cpp` builds one instance, captures it by reference in two
 * routes, and runs Crow `multithreaded()`. Nothing in the object is
 * mutated after construction: the constructor fixes the root path, and
 * the extension-to-MIME table is an immutable function-local static. A
 * per-request cache added later would break that and would need its own
 * synchronization.
 *
 * ## :material-shield-check: Security
 *
 * Path traversal is stopped by canonicalizing both the static root and
 * the requested path with `weakly_canonical()`, then computing
 * `lexically_relative()`. An empty relative path, or one whose string
 * starts with `..`, is rejected with 403. `lexically_relative()` emits
 * `..` only as a whole path component, which is what makes the string
 * `starts_with` check sound here.
 *
 * The guard runs ahead of the existence check, so a traversal attempt
 * gets 403 whether or not the target exists. The SPA fallback builds a
 * second path (`root/index.html`) and runs the same guard on it, so the
 * fallback cannot escape the root either.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * flowchart TD
 *     A["serve(path)"] --> B{"path empty?"}
 *     B -- yes --> C["file = root/index.html"]
 *     B -- no --> D["file = root/path"]
 *     C --> E["weakly_canonical(root), weakly_canonical(file)"]
 *     D --> E
 *     E -- "filesystem_error" --> F["404"]
 *     E --> G{"relative path empty or starts with '..'?"}
 *     G -- yes --> H["403"]
 *     G -- no --> I{"exists and not a directory?"}
 *     I -- no --> J["SPA fallback: root/index.html"]
 *     J --> J2{"re-validate against root"}
 *     J2 -- "rejected" --> H
 *     J2 -- "filesystem_error" --> F
 *     J2 -- ok --> K["stat the file"]
 *     I -- yes --> K
 *     K -- "missing or not a regular file" --> F
 *     K --> M["200 + Content-Type + Content-Length"]
 * ```
 *
 * ## :material-file-document-outline: Content types
 *
 * The MIME type comes from the file extension. The caller lower-cases
 * the extension first, so the lookup is case-insensitive: `.PNG` and
 * `.png` both resolve to `image/png`.
 *
 * | Extension | Content-Type |
 * |-----------|--------------|
 * | `.html` | `text/html` |
 * | `.css` | `text/css` |
 * | `.js` | `application/javascript` |
 * | `.json` | `application/json` |
 * | `.map` | `application/json` |
 * | `.png` | `image/png` |
 * | `.jpg`, `.jpeg` | `image/jpeg` |
 * | `.gif` | `image/gif` |
 * | `.svg` | `image/svg+xml` |
 * | `.ico` | `image/x-icon` |
 * | `.woff` | `font/woff` |
 * | `.woff2` | `font/woff2` |
 * | `.ttf` | `font/ttf` |
 * | *anything else* | `application/octet-stream` |
 *
 * `.map` is there so browser devtools can fetch the Vite source maps.
 * The table is the complete set. Adding an entry here without adding it
 * to `get_content_type` changes nothing at runtime.
 *
 * ## :material-cached: Caching
 *
 * No `Cache-Control` and no `ETag` headers are set, so assets are served
 * fresh on every request. That is fine for the local dev server. Behind
 * a reverse proxy, configure caching at the proxy layer instead.
 *
 * ## :material-alert-circle-outline: Edge cases
 *
 * - An empty path means `index.html`. The `/` route in `main.cpp` calls
 *   `serve("")` and depends on that; it does not pass `"index.html"`
 *   itself.
 * - A missing file, or a path that names a directory, answers 200 with
 *   `index.html` rather than 404, so client-side routing works. A caller
 *   that needs a hard 404 has to filter before calling. `main.cpp` does
 *   exactly that for the `api/` prefix.
 * - When the static root directory is missing, `weakly_canonical()` still
 *   succeeds, because it does not require the path to exist. The request
 *   reaches the SPA fallback and then 404s, because the `stat` on
 *   `index.html` fails.
 * - When `weakly_canonical()` throws (a broken symlink, an I/O error),
 *   the exception is caught and the request returns 404. No filesystem
 *   exception propagates to Crow's internal handler.
 *
 * ## :material-code-tags: Usage example
 *
 * ```cpp
 * StaticFileHandler handler("./web/dist");
 * CROW_ROUTE(app, "/<path>")([&](const std::string& path) {
 *     return handler.serve(path);
 * });
 * ```
 */
class StaticFileHandler
{
public:
    /**
     * @brief Construct a handler rooted at the given directory.
     *
     * The path is stored verbatim: canonicalization and the existence
     * check both happen per request in serve(). A relative path is
     * therefore resolved against the process working directory at
     * request time, not at construction time.
     *
     * @param static_dir Absolute or relative path to the directory
     *        containing the built frontend assets.
     */
    explicit StaticFileHandler(const std::string& static_dir);

    /**
     * @brief Serve a file or fall back to index.html.
     *
     * Returns the requested file with the matching `Content-Type` header.
     * An empty @p path resolves to `index.html`. A path that does not
     * exist, or that names a directory, also resolves to `index.html`
     * through the SPA fallback and is answered with 200 and `text/html`,
     * not 404.
     *
     * The body is not read here. The response is handed to Crow as a
     * static-file response, so this call does path resolution plus a
     * `stat` and nothing more; Crow streams the bytes afterwards. No size
     * limit applies at any point.
     *
     * @param path Path relative to the static root, with `/`
     *        separators as received from Crow. Empty means
     *        `index.html`.
     * @return 200 with the file contents, 403 when the resolved path
     *         escapes the static root, or 404 when `weakly_canonical()`
     *         fails or the resolved file is missing or is not a regular
     *         file. No exception propagates to callers; filesystem errors
     *         from `weakly_canonical()` are caught and converted to 404.
     */
    crow::response serve(const std::string& path);

private:
    std::string static_dir_;  ///< Root directory for static assets, stored verbatim.
    std::string get_content_type(const std::string& extension);
};

}  // namespace mo2server
