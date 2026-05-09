// SalmaEngine - the only place in the server that knows the engine is a DLL.
//
// Everything below is process-global, not per-instance: one HMODULE, one set of
// bound function pointers, and one `installSucceeded` flag inside the engine.
// SalmaEngine has no members and no instances; all methods are static.
//
// Two mutexes, never held at the same time:
//   g_load_mutex     guards the one-time load and the g_engine fields it fills.
//                    Every entry point calls ensure_loaded() first, so the load
//                    is safe to race and does work only once.
//   g_install_mutex  serializes install_mod, because the engine's
//                    `installSucceeded` flag is process-global and the server
//                    runs installs on overlapping background jobs.
//
// The DLL is loaded lazily on first use and never unloaded, and no C++ code
// unloads or reloads it. The server also never calls the engine's
// `setLogCallback`: only the MO2 Python plugin and the harness scripts do. In
// the server process the engine therefore writes its own log lines through its
// own file handle, which is the second-writer case Logger.cpp's torn-line
// comment describes.
//
// Failure conventions differ per call, deliberately. The flat ABI reports every
// failure as a return value; the dashboard controllers expect an exception from
// an install and a sentinel from the rest, so this layer restores that shape:
//   install_mod         throws std::runtime_error on failure.
//   infer_selections    returns "" on any failure. Never throws.
//   resolve_mod_archive returns an empty path on any failure. Never throws.
//   api_version         returns "" when the DLL is not loadable.

#include "SalmaEngine.hpp"

#include "Logger.hpp"
#include "Utils.hpp"

#include <format>
#include <mutex>
#include <stdexcept>

#ifdef _WIN32
#include <windows.h>
#endif

namespace fs = std::filesystem;

namespace mo2server
{

namespace
{

// Flat ABI of mo2-salma.dll. These signatures must match what the MO2 Python
// plugin binds through ctypes; src/capi.rs holds the full export list.
//
// The server binds six of the eight exports. It skips `install`, because
// install_mod always routes through `installWithConfig` with an explicit
// selections-JSON path, and `setLogCallback`, because it never registers one.
// A typedef alone does not make an export usable: bind it in ensure_loaded()
// too, or the pointer stays null.
using FnInstallWithConfig = const char*(__cdecl*)(const char*, const char*, const char*);
using FnInferSelections = const char*(__cdecl*)(const char*, const char*);
using FnResolveModArchive = const char*(__cdecl*)(const char*, const char*, const char*);
using FnFreeResult = void(__cdecl*)(const char*);
using FnInstallSucceeded = bool(__cdecl*)();
using FnGetApiVersion = const char*(__cdecl*)();

struct Engine
{
    bool loaded = false;
    std::string path;
    FnInstallWithConfig install_with_config = nullptr;
    FnInferSelections infer_selections = nullptr;
    FnResolveModArchive resolve_mod_archive = nullptr;
    FnFreeResult free_result = nullptr;
    FnInstallSucceeded install_succeeded = nullptr;
    FnGetApiVersion get_api_version = nullptr;
};

Engine g_engine;
std::mutex g_load_mutex;

// `installSucceeded` is a single process-global flag in the engine, so one
// install must not read another's result. The server runs installs on
// background jobs that can overlap, so this serializes the call together with
// the flag read that follows it.
std::mutex g_install_mutex;

#ifdef _WIN32
// Candidate DLL locations, in probe order:
//   1. <exe dir>/mo2-salma.dll        - what CMake copies next to mo2-server.exe
//   2. <exe dir>/salma/mo2-salma.dll  - the layout deploy.bat produces
// ensure_loaded() then falls back to the OS default search order (PATH and the
// system directories) when neither candidate exists or loads.
//
// This order is deliberately not the MO2 Python plugin's. `find_dll` in
// scripts/mo2-salma.py probes <plugin dir>/salma before the flat path, and also
// probes the working directory and cwd/dlls/salma. The server never probes the
// working directory, because a service process must not load code from wherever
// it happened to start. A host holding the DLL in both the flat and the salma/
// location therefore loads a different file here than the plugin does. Deploy
// one copy, not two.
std::vector<fs::path> candidates()
{
    std::vector<fs::path> out;
    auto exe_dir = mo2core::executable_directory();
    if (!exe_dir.empty())
    {
        out.push_back(fs::path(exe_dir) / "mo2-salma.dll");
        out.push_back(fs::path(exe_dir) / "salma" / "mo2-salma.dll");
    }
    return out;
}

// Resolve one export. A missing export means the DLL is an older or different
// build, so the whole load fails rather than leaving a null function pointer
// behind for a later call to trip over.
template <typename Fn>
bool bind(HMODULE mod, const char* name, Fn& out)
{
    // GetProcAddress returns FARPROC; a direct reinterpret_cast to the real
    // signature is the sanctioned Win32 idiom (going via void* trips
    // bugprone-casting-through-void).
    out = reinterpret_cast<Fn>(GetProcAddress(mod, name));
    if (!out)
    {
        mo2core::Logger::instance().log_error(
            std::format("[server] mo2-salma.dll is missing the export '{}'", name));
        return false;
    }
    return true;
}
#endif

// Copy an engine-allocated string, then release the original through the
// engine's own freeResult. Every owned return in the ABI has to pass through
// here: the engine allocated the buffer, so only the engine may free it. A null
// pointer yields an empty string, which is how the ABI spells failure for infer
// and resolve.
std::string take(const char* owned)
{
    std::string out = owned ? owned : "";
    if (owned && g_engine.free_result)
    {
        g_engine.free_result(owned);
    }
    return out;
}

}  // namespace

bool SalmaEngine::ensure_loaded()
{
    std::lock_guard<std::mutex> lock(g_load_mutex);
    if (g_engine.loaded)
    {
        return true;
    }

#ifdef _WIN32
    auto& logger = mo2core::Logger::instance();
    HMODULE mod = nullptr;
    std::string chosen;

    for (const auto& candidate : candidates())
    {
        std::error_code ec;
        if (!fs::exists(candidate, ec))
        {
            continue;
        }
        mod = LoadLibraryW(candidate.wstring().c_str());
        if (mod)
        {
            chosen = candidate.string();
            break;
        }
        logger.log_warning(
            std::format("[server] LoadLibrary failed for {} (error {})", candidate.string(),
                        GetLastError()));
    }

    if (!mod)
    {
        // Fall back to the default search order (PATH, system dirs).
        mod = LoadLibraryW(L"mo2-salma.dll");
        chosen = mod ? "mo2-salma.dll (default search path)" : "";
    }

    if (!mod)
    {
        logger.log_error("[server] mo2-salma.dll not found. Build the Rust engine "
                         "(build.bat) and copy target/package/mo2-salma.dll next to "
                         "mo2-server.exe.");
        return false;
    }

    if (!bind(mod, "installWithConfig", g_engine.install_with_config) ||
        !bind(mod, "inferFomodSelections", g_engine.infer_selections) ||
        !bind(mod, "resolveModArchive", g_engine.resolve_mod_archive) ||
        !bind(mod, "freeResult", g_engine.free_result) ||
        !bind(mod, "installSucceeded", g_engine.install_succeeded) ||
        !bind(mod, "getApiVersion", g_engine.get_api_version))
    {
        return false;
    }

    g_engine.loaded = true;
    g_engine.path = chosen;
    logger.log(std::format("[server] Engine loaded: {} (API {})", chosen,
                           g_engine.get_api_version() ? g_engine.get_api_version() : "?"));
    return true;
#else
    mo2core::Logger::instance().log_error("[server] The engine DLL is Windows-only");
    return false;
#endif
}

std::string SalmaEngine::loaded_path()
{
    std::lock_guard<std::mutex> lock(g_load_mutex);
    return g_engine.path;
}

std::string SalmaEngine::api_version()
{
    if (!ensure_loaded())
    {
        return "";
    }
    const char* v = g_engine.get_api_version();
    return v ? v : "";
}

std::string SalmaEngine::install_mod(const std::string& archive_path,
                                     const std::string& mod_path,
                                     const std::string& json_path)
{
    if (!ensure_loaded())
    {
        throw std::runtime_error("salma engine (mo2-salma.dll) could not be loaded");
    }

    std::lock_guard<std::mutex> lock(g_install_mutex);
    auto result = take(g_engine.install_with_config(archive_path.c_str(), mod_path.c_str(),
                                                    json_path.c_str()));
    // The flag is only meaningful while the install mutex is held.
    const bool ok = g_engine.install_succeeded();
    if (!ok)
    {
        // On failure the engine returns its error text in the same slot as the
        // success value, so that text becomes the exception message.
        throw std::runtime_error(result.empty() ? "installation failed" : result);
    }
    return result;
}

std::string SalmaEngine::infer_selections(const std::string& archive_path,
                                          const std::string& mod_path)
{
    if (!ensure_loaded())
    {
        return "";
    }
    return take(g_engine.infer_selections(archive_path.c_str(), mod_path.c_str()));
}

fs::path SalmaEngine::resolve_mod_archive(const std::string& installation_file,
                                          const fs::path& mod_folder,
                                          const fs::path& mods_dir)
{
    if (!ensure_loaded())
    {
        return {};
    }
    auto resolved = take(g_engine.resolve_mod_archive(installation_file.c_str(),
                                                      mod_folder.string().c_str(),
                                                      mods_dir.string().c_str()));
    return resolved.empty() ? fs::path{} : fs::path(resolved);
}

}  // namespace mo2server
