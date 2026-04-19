cmake_minimum_required(VERSION 3.20)
project(salma VERSION 1.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Always emit compile_commands.json so clangd can find the right flags
# without re-deriving them from CMakePresets. Note: the Visual Studio
# generator silently ignores this setting; the `compile-db` preset uses
# Ninja into build-cdb/ specifically to produce a real compile DB for
# clang-tidy (see build.bat step 3).
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

enable_testing()

# --- salma-support (static library) ---
#
# What is left of the C++ after the engine moved to Rust: the shared helpers the
# Crow server still needs. The engine itself is mo2-salma.dll, built from the
# Rust crate at the repo root and reached through src/SalmaEngine.cpp.
#
# This is deliberately no longer a SHARED library named mo2-salma. That name now
# belongs to the Rust artifact, and having two different DLLs claim it is how you
# ship the wrong engine by accident.

find_package(nlohmann_json CONFIG REQUIRED)

add_library(salma-support STATIC
    src/Utils.cpp
    src/Logger.cpp
    src/SecurityContext.cpp
)

target_include_directories(salma-support
    PUBLIC ${CMAKE_SOURCE_DIR}/src
)

target_link_libraries(salma-support
    PRIVATE nlohmann_json::nlohmann_json
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
    src/SalmaEngine.cpp
)

target_link_libraries(mo2-server
    PRIVATE salma-support
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
    tests/security_context_test.cpp
    tests/salma_engine_test.cpp
    src/SalmaEngine.cpp
)

target_link_libraries(salma_tests
    PRIVATE salma-support
    PRIVATE GTest::gtest_main
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

# The server loads the engine from mo2-salma.dll at runtime (src/SalmaEngine.cpp
# searches beside the exe first). CMake does not build it - it is the Rust
# artifact staged by `python tools/package.py` - so copy it next to the exe when
# it is there. Without this the dashboard starts but every install/infer request
# fails with "mo2-salma.dll not found".
if(EXISTS "${CMAKE_SOURCE_DIR}/target/package/mo2-salma.dll")
    add_custom_command(TARGET mo2-server POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
            "${CMAKE_SOURCE_DIR}/target/package/mo2-salma.dll"
            "${OUTDIR}/"
        COMMENT "Copying the Rust engine DLL next to mo2-server")
else()
    message(WARNING
        "target/package/mo2-salma.dll not found. mo2-server needs it at runtime; "
        "run build.bat (or python tools/package.py) and reconfigure, or copy it "
        "next to mo2-server.exe by hand.")
endif()

# --- Release bundle: install layout + CPack ZIP ---
#
# Two CPack components combine into one salma-mo2-<ver>.zip:
#   - MO2_PLUGIN: drops into <MO2 instance>/plugins/ (mo2-salma.py + salma/*)
#   - DASHBOARD:  standalone server, web/dist, and helper scripts
#
# The C-API ABI version is the source of truth. It moved to the Rust engine with
# the rest of the C ABI and now lives in src/capi.rs as MO2_SALMA_API_VERSION.
# CMake reads it via regex so the bundle filename always matches the ABI string
# the DLL reports at runtime.
file(READ "${CMAKE_SOURCE_DIR}/src/capi.rs" _capi_rs_content)
string(REGEX MATCH "MO2_SALMA_API_VERSION:[ \t]*&str[ \t]*=[ \t]*\"([0-9]+\\.[0-9]+\\.[0-9]+)\""
       _ver_match "${_capi_rs_content}")
set(SALMA_API_VERSION "${CMAKE_MATCH_1}")
if(NOT SALMA_API_VERSION)
    message(FATAL_ERROR
        "Failed to parse MO2_SALMA_API_VERSION from src/capi.rs - "
        "the regex in CMakeLists.txt is out of sync with the Rust constant.")
endif()
file(WRITE "${CMAKE_BINARY_DIR}/VERSION" "${SALMA_API_VERSION}\n")

# MO2_PLUGIN component
install(FILES "${CMAKE_SOURCE_DIR}/scripts/mo2-salma.py"
        DESTINATION plugins
        COMPONENT MO2_PLUGIN)
# The plugin DLL is the RUST artifact, which CMake does not build. Staged by
# `python tools/package.py` into target/package/mo2-salma.dll and installed from
# there when present, so `cpack` still produces a complete MO2_PLUGIN bundle.
install(FILES "${CMAKE_SOURCE_DIR}/target/package/mo2-salma.dll"
        DESTINATION plugins/salma
        COMPONENT MO2_PLUGIN
        OPTIONAL)
# DASHBOARD component
install(TARGETS mo2-server
        RUNTIME DESTINATION dashboard
        COMPONENT DASHBOARD)
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
