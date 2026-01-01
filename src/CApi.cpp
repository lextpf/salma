#include "CApi.h"
#include <string>
#include "FomodInferenceService.h"
#include "InstallationService.h"
#include "Logger.h"

// Static result buffers - install/installWithConfig share g_result,
// inferFomodSelections uses g_infer_result. Each buffer's .c_str()
// is valid until the next call to the same function group.
// Not thread-safe: concurrent calls race on these buffers.
static std::string g_result;
static std::string g_infer_result;

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
            g_result = "archivePath and modPath must not be null";
            return g_result.c_str();
        }
        try
        {
            mo2core::InstallationService service;
            g_result = service.install_mod(archivePath, modPath);
            return g_result.c_str();
        }
        catch (const std::exception& e)
        {
            mo2core::Logger::instance().log_error(std::string("[install] Fatal error: ") +
                                                  e.what());
            g_result = e.what();
            return g_result.c_str();
        }
    }

    MO2_API const char* installWithConfig(const char* archivePath,
                                          const char* modPath,
                                          const char* jsonPath)
    {
        if (!archivePath || !modPath)
        {
            g_result = "archivePath and modPath must not be null";
            return g_result.c_str();
        }
        // Null jsonPath is coerced to empty string (skips optional steps).
        try
        {
            mo2core::InstallationService service;
            g_result = service.install_mod(archivePath, modPath, jsonPath ? jsonPath : "");
            return g_result.c_str();
        }
        catch (const std::exception& e)
        {
            mo2core::Logger::instance().log_error(std::string("[install] Fatal error: ") +
                                                  e.what());
            g_result = e.what();
            return g_result.c_str();
        }
    }

    MO2_API const char* inferFomodSelections(const char* archivePath, const char* modPath)
    {
        // Null inputs are coerced to empty strings (will fail gracefully).
        // Returns JSON selections on success, empty string on failure.
        // Uses separate g_infer_result buffer so inference doesn't clobber install results.
        try
        {
            mo2core::FomodInferenceService service;
            g_infer_result =
                service.infer_selections(archivePath ? archivePath : "", modPath ? modPath : "");
            return g_infer_result.c_str();
        }
        catch (const std::exception& e)
        {
            mo2core::Logger::instance().log_error(std::string("[infer] Fatal error: ") + e.what());
            g_infer_result = "";
            return g_infer_result.c_str();
        }
    }

}  // extern "C"

}  // namespace CApi
