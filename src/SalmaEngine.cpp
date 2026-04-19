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

// Flat ABI of mo2-salma.dll. Kept byte-identical to the declarations the MO2
// Python plugin binds through ctypes; see the Rust crate's src/capi.rs.
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

// `installSucceeded` is a single process-global flag in the engine, so an
// install must not observe another install's result. The server runs installs
// on background jobs, which can overlap; this serializes the call plus its flag
// read. The old in-process C++ InstallationService needed no such guard because
// each call had its own instance.
std::mutex g_install_mutex;

#ifdef _WIN32
/// Candidate DLL locations, most specific first. Mirrors the search the MO2
/// Python plugin does in `find_dll`.
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

/// Resolve one export, logging and failing the whole load when it is absent.
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

/// Take ownership of an engine-allocated string and release it.
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
        // The engine returns its error text in the same slot as the success
        // value, which is what the old C++ threw as `what()`.
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
