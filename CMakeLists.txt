cmake_minimum_required(VERSION 3.20)
project(salma VERSION 1.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

enable_testing()

# --- mo2-core (shared library) ---

find_package(pugixml CONFIG REQUIRED)
find_package(nlohmann_json CONFIG REQUIRED)
find_package(LibArchive REQUIRED)
find_package(unofficial-bit7z CONFIG REQUIRED)

add_library(mo2-core SHARED
    src/Utils.cpp
    src/Logger.cpp
    src/SecurityContext.cpp
    src/FileOperations.cpp
    src/ArchiveService.cpp
    src/ModStructureDetector.cpp
    src/FomodArchiveResolver.cpp
    src/FomodDependencyEvaluator.cpp
    src/FomodService.cpp
    src/FomodIRParser.cpp
    src/FomodForwardSimulator.cpp
    src/FomodCSPPrecompute.cpp
    src/FomodCSPOptions.cpp
    src/FomodCSPSolver.cpp
    src/FomodCSPSolverPhases.cpp
    src/FomodPropagator.cpp
    src/FomodInferenceAtoms.cpp
    src/FomodInferenceService.cpp
    src/InstallationService.cpp
    src/CApi.cpp
)

target_include_directories(mo2-core
    PUBLIC ${CMAKE_SOURCE_DIR}/src
)

target_link_libraries(mo2-core
    PRIVATE pugixml::pugixml
    PRIVATE nlohmann_json::nlohmann_json
    PRIVATE LibArchive::LibArchive
    PRIVATE unofficial::bit7z::bit7z64
)

target_compile_definitions(mo2-core PRIVATE MO2_CORE_EXPORTS LIBARCHIVE_STATIC)

set_target_properties(mo2-core PROPERTIES
    OUTPUT_NAME "mo2-salma"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin"
    LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin"
    ARCHIVE_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/lib"
)

# --- mo2-server (executable) ---

find_package(Crow CONFIG REQUIRED)

add_executable(mo2-server
    src/main.cpp
    src/InstallationController.cpp
    src/Mo2Controller.cpp
    src/Mo2ConfigController.cpp
    src/Mo2FomodController.cpp
    src/Mo2LogController.cpp
    src/Mo2TestController.cpp
    src/Mo2PluginController.cpp
    src/Mo2Helpers.cpp
    src/ConfigService.cpp
    src/MultipartHandler.cpp
    src/SecurityMiddleware.cpp
    src/StaticFileHandler.cpp
)

target_link_libraries(mo2-server
    PRIVATE mo2-core
    PRIVATE Crow::Crow
    PRIVATE nlohmann_json::nlohmann_json
)

if(WIN32)
    target_compile_definitions(mo2-server PRIVATE _WIN32_WINNT=0x0A00)
endif()

set_target_properties(mo2-server PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin"
)

# --- salma_tests (Google Test) ---

find_package(GTest CONFIG REQUIRED)
include(GoogleTest)

add_executable(salma_tests
    tests/utils_test.cpp
    tests/fomod_inference_test.cpp
    tests/security_context_test.cpp
)

target_link_libraries(salma_tests
    PRIVATE mo2-core
    PRIVATE GTest::gtest_main
    PRIVATE pugixml::pugixml
)

set_target_properties(salma_tests PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin"
)

gtest_discover_tests(salma_tests
    DISCOVERY_MODE PRE_TEST
    PROPERTIES TIMEOUT 60
)

# --- Copy test suite + scripts next to the exe ---

# The output dir varies by config: bin/Release, bin/Debug, etc.
# Use a generator expression so the files land beside the actual exe.
set(OUTDIR "$<TARGET_FILE_DIR:mo2-server>")

add_custom_command(TARGET mo2-server POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${CMAKE_SOURCE_DIR}/test.bat"
        "${CMAKE_SOURCE_DIR}/test_all.py"
        "${CMAKE_SOURCE_DIR}/test_one.py"
        "${CMAKE_SOURCE_DIR}/deploy.bat"
        "${CMAKE_SOURCE_DIR}/purge.bat"
        "${OUTDIR}/"
    COMMAND ${CMAKE_COMMAND} -E copy_directory
        "${CMAKE_SOURCE_DIR}/scripts"
        "${OUTDIR}/scripts"
    COMMENT "Copying test suite and scripts to output directory"
)

# --- Release bundle: install layout + CPack ZIP ---
#
# Two CPack components combine into one salma-mo2-<ver>.zip:
#   - MO2_PLUGIN: drops into <MO2 instance>/plugins/ (mo2-salma.py + salma/*)
#   - DASHBOARD:  standalone server, web/dist, and helper scripts
#
# The C-API ABI version is the source of truth - it lives in src/CApi.h as
# MO2_SALMA_API_VERSION. CMake reads it via regex so the bundle filename
# always matches the ABI string the DLL reports at runtime.
file(READ "${CMAKE_SOURCE_DIR}/src/CApi.h" _capi_h_content)
string(REGEX MATCH "MO2_SALMA_API_VERSION[ \t]+\"([0-9]+\\.[0-9]+\\.[0-9]+)\""
       _ver_match "${_capi_h_content}")
set(SALMA_API_VERSION "${CMAKE_MATCH_1}")
if(NOT SALMA_API_VERSION)
    message(FATAL_ERROR
        "Failed to parse MO2_SALMA_API_VERSION from src/CApi.h - "
        "the regex in CMakeLists.txt is out of sync with the header.")
endif()
file(WRITE "${CMAKE_BINARY_DIR}/VERSION" "${SALMA_API_VERSION}\n")

# Resolve 7z.dll for bundling. Override with -DSALMA_7Z_DLL=<path>;
# otherwise check SEVENZIP_PATH (file or dir), then Program Files. If
# nothing matches we leave SALMA_7Z_DLL empty and warn - deployed installs
# will then fall back to a system 7-Zip at runtime (see ArchiveService.cpp).
if(NOT DEFINED SALMA_7Z_DLL)
    if(DEFINED ENV{SEVENZIP_PATH})
        set(_seven "$ENV{SEVENZIP_PATH}")
        if(EXISTS "${_seven}" AND _seven MATCHES "\\.dll$")
            set(SALMA_7Z_DLL "${_seven}")
        elseif(EXISTS "${_seven}/7z.dll")
            set(SALMA_7Z_DLL "${_seven}/7z.dll")
        endif()
    endif()
    if(NOT DEFINED SALMA_7Z_DLL)
        # CMake's $ENV{NAME} parser rejects parentheses in NAME, so query
        # ProgramFiles via $ENV{} and fall back to the standard literal path
        # for the (x86) variant. file(TO_CMAKE_PATH) normalizes the
        # backslashes Windows returns to forward slashes - otherwise paths
        # like "C:\Program Files/7-Zip/7z.dll" trip CMake's `\P` escape
        # warning when serialized into cmake_install.cmake.
        set(_pf_env "$ENV{ProgramFiles}")
        if(_pf_env)
            file(TO_CMAKE_PATH "${_pf_env}" _pf_env)
        endif()
        foreach(_root
                "${_pf_env}"
                "C:/Program Files"
                "C:/Program Files (x86)")
            if(_root AND EXISTS "${_root}/7-Zip/7z.dll")
                set(SALMA_7Z_DLL "${_root}/7-Zip/7z.dll")
                break()
            endif()
        endforeach()
    endif()
endif()
if(NOT DEFINED SALMA_7Z_DLL OR NOT EXISTS "${SALMA_7Z_DLL}")
    message(WARNING
        "SALMA_7Z_DLL not set or not found. The release bundle will not include "
        "7z.dll - deployed installs will need a system 7-Zip at runtime. "
        "Set -DSALMA_7Z_DLL=<path> or the SEVENZIP_PATH env var to bundle a codec.")
    set(SALMA_7Z_DLL "")
endif()

# MO2_PLUGIN component
install(FILES "${CMAKE_SOURCE_DIR}/scripts/mo2-salma.py"
        DESTINATION plugins
        COMPONENT MO2_PLUGIN)
install(TARGETS mo2-core
        RUNTIME DESTINATION plugins/salma
        COMPONENT MO2_PLUGIN)
if(SALMA_7Z_DLL)
    install(FILES "${SALMA_7Z_DLL}"
            DESTINATION plugins/salma
            COMPONENT MO2_PLUGIN)
endif()

# DASHBOARD component
install(TARGETS mo2-server
        RUNTIME DESTINATION dashboard
        COMPONENT DASHBOARD)
if(SALMA_7Z_DLL)
    install(FILES "${SALMA_7Z_DLL}"
            DESTINATION dashboard
            COMPONENT DASHBOARD)
endif()
install(DIRECTORY "${CMAKE_SOURCE_DIR}/web/dist/"
        DESTINATION dashboard/web/dist
        COMPONENT DASHBOARD
        OPTIONAL)
install(DIRECTORY "${CMAKE_SOURCE_DIR}/scripts/"
        DESTINATION dashboard/scripts
        COMPONENT DASHBOARD
        REGEX "/__pycache__($|/)" EXCLUDE
        PATTERN "*.pyc" EXCLUDE)
install(FILES
            "${CMAKE_SOURCE_DIR}/test_all.py"
            "${CMAKE_SOURCE_DIR}/test_one.py"
            "${CMAKE_SOURCE_DIR}/test.bat"
            "${CMAKE_SOURCE_DIR}/deploy.bat"
            "${CMAKE_SOURCE_DIR}/purge.bat"
        DESTINATION dashboard
        COMPONENT DASHBOARD)

# Top-level metadata: VERSION always, README/CHANGELOG if present.
install(FILES "${CMAKE_BINARY_DIR}/VERSION"
        DESTINATION .
        COMPONENT MO2_PLUGIN)
if(EXISTS "${CMAKE_SOURCE_DIR}/CHANGELOG.md")
    install(FILES "${CMAKE_SOURCE_DIR}/CHANGELOG.md"
            DESTINATION .
            COMPONENT MO2_PLUGIN)
endif()
if(EXISTS "${CMAKE_SOURCE_DIR}/README.md")
    install(FILES "${CMAKE_SOURCE_DIR}/README.md"
            DESTINATION .
            COMPONENT MO2_PLUGIN)
endif()

set(CPACK_GENERATOR "ZIP")
set(CPACK_PACKAGE_NAME "salma-mo2")
set(CPACK_PACKAGE_VERSION "${SALMA_API_VERSION}")
set(CPACK_PACKAGE_FILE_NAME "salma-mo2-${SALMA_API_VERSION}")
set(CPACK_COMPONENTS_ALL MO2_PLUGIN DASHBOARD)
set(CPACK_ARCHIVE_COMPONENT_INSTALL OFF)   # one zip containing both components
include(CPack)
