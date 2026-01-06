#include "CApi.h"
#include "FomodInferenceService.h"
#include "InstallationService.h"
#include "Logger.h"

#include <atomic>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <string>

// Mutex-guarded global so that installSucceeded() works correctly even when
// install() and installSucceeded() are called from different threads (common
// with Python ctypes).  A thread_local would silently return the wrong value.
// The atomic type provides an additional safety net against accidental
// unsynchronised reads outside the mutex.
static std::mutex g_install_mutex;
static std::atomic<bool> g_last_install_success{false};

namespace CApi
{

extern "C"
{
    MO2_API void setLogCallback(Mo2LogCallback callback)
    {
        // Callback registration is mutex-guarded inside Logger.
        // Setting nullptr re-enables file logging (logs/salma.log).
        mo2core::Logger::instance().set_callback(callback);
    }

    MO2_API const char* install(const char* archivePath, const char* modPath)
    {
        if (!archivePath || !modPath)
        {
            std::lock_guard<std::mutex> lock(g_install_mutex);
            g_last_install_success = false;
            return _strdup("archivePath and modPath must not be null");
        }
        try
        {
            mo2core::InstallationService service;
            auto result = service.install_mod(archivePath, modPath);
            std::lock_guard<std::mutex> lock(g_install_mutex);
            g_last_install_success = true;
            return _strdup(result.c_str());
        }
        catch (const std::exception& e)
        {
            mo2core::Logger::instance().log_error(std::string("[install] Fatal error: ") +
                                                  e.what());
            std::lock_guard<std::mutex> lock(g_install_mutex);
            g_last_install_success = false;
            return _strdup(e.what());
        }
        catch (...)
        {
            mo2core::Logger::instance().log_error("[install] Fatal error: unknown exception");
            std::lock_guard<std::mutex> lock(g_install_mutex);
            g_last_install_success = false;
            return _strdup("Unknown fatal error during installation");
        }
    }

    MO2_API const char* installWithConfig(const char* archivePath,
                                          const char* modPath,
                                          const char* jsonPath)
    {
        if (!archivePath || !modPath)
        {
            std::lock_guard<std::mutex> lock(g_install_mutex);
            g_last_install_success = false;
            return _strdup("archivePath and modPath must not be null");
        }
        // Null jsonPath is coerced to empty string (skips optional steps).
        try
        {
            mo2core::InstallationService service;
            auto result = service.install_mod(archivePath, modPath, jsonPath ? jsonPath : "");
            std::lock_guard<std::mutex> lock(g_install_mutex);
            g_last_install_success = true;
            return _strdup(result.c_str());
        }
        catch (const std::exception& e)
        {
            mo2core::Logger::instance().log_error(std::string("[installWithConfig] Fatal error: ") +
                                                  e.what());
            std::lock_guard<std::mutex> lock(g_install_mutex);
            g_last_install_success = false;
            return _strdup(e.what());
        }
        catch (...)
        {
            mo2core::Logger::instance().log_error(
                "[installWithConfig] Fatal error: unknown exception");
            std::lock_guard<std::mutex> lock(g_install_mutex);
            g_last_install_success = false;
            return _strdup("Unknown fatal error during installation");
        }
    }

    MO2_API const char* inferFomodSelections(const char* archivePath, const char* modPath)
    {
        if (!archivePath || !modPath)
            return _strdup("archivePath and modPath must not be null");
        // Returns JSON selections on success, empty string on failure.
        try
        {
            mo2core::FomodInferenceService service;
            auto result = service.infer_selections(archivePath, modPath);
            return _strdup(result.c_str());
        }
        catch (const std::exception& e)
        {
            mo2core::Logger::instance().log_error(std::string("[infer] Fatal error: ") + e.what());
            return _strdup("");
        }
        catch (...)
        {
            mo2core::Logger::instance().log_error("[infer] Fatal error: unknown exception");
            return _strdup("");
        }
    }

    MO2_API bool installSucceeded()
    {
        std::lock_guard<std::mutex> lock(g_install_mutex);
        return g_last_install_success;
    }

    MO2_API void freeResult(const char* result)
    {
        free(const_cast<char*>(result));
    }

}  // extern "C"

}  // namespace CApi
