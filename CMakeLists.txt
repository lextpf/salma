cmake_minimum_required(VERSION 3.20)
project(salma VERSION 1.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Ask for compile_commands.json. Only the Ninja and Makefile generators honour this, and the
# default preset uses the Visual Studio generator, so build/ has none: clangd falls back to
# .clangd, and `cmake --preset compile-db` is the Ninja sidecar that emits a real one.
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

enable_testing()

# The C++ here is the Crow dashboard server plus the shared helpers mo2-server and salma_tests
# link. The engine is mo2-salma.dll, built from the Rust crate at the repo root and reached
# through src/SalmaEngine.cpp.

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
        "${CMAKE_SOURCE_DIR}/setup.bat"
        "${OUTDIR}/"
    COMMAND ${CMAKE_COMMAND} -E copy_directory
        "${CMAKE_SOURCE_DIR}/scripts"
        "${OUTDIR}/scripts"
    COMMENT "Copying test suite and scripts to output directory"
)

# mo2-server and salma_tests both load mo2-salma.dll from their own directory and both land in
# build/bin/<config>, so this one copy serves both. It hangs off mo2-server, so a build that
# targets salma_tests alone (build presets tests-only and ci-tests) skips it and every SalmaEngine
# test fails on ensure_loaded(). CMake does not build the DLL; it is the Rust artifact staged by
# `python scripts/package.py`, and the EXISTS check below runs at configure time, so staging it
# later needs a reconfigure.
if(EXISTS "${CMAKE_SOURCE_DIR}/target/package/mo2-salma.dll")
    add_custom_command(TARGET mo2-server POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
            "${CMAKE_SOURCE_DIR}/target/package/mo2-salma.dll"
            "${OUTDIR}/"
        COMMENT "Copying the Rust engine DLL next to mo2-server")
else()
    message(WARNING
        "target/package/mo2-salma.dll not found. mo2-server needs it at runtime; "
        "run build.bat (or python scripts/package.py) and reconfigure, or copy it "
        "next to mo2-server.exe by hand.")
endif()

# --- Release bundle: install layout + CPack ZIP ---
#
# Two CPack components combine into one salma-mo2-<ver>.zip:
#   - MO2_PLUGIN: drops into <MO2 instance>/plugins/ (mo2-salma.py + salma/*)
#   - DASHBOARD:  mo2-server, web/dist and the helper scripts. The engine DLL is
#                 not duplicated here: copy plugins/salma/mo2-salma.dll next to
#                 mo2-server.exe or the server loads no engine.
#
# src/capi.rs owns the version: MO2_SALMA_API_VERSION is what the engine reports over the ABI and
# what names the release zip. project()'s 1.0.0 above is not used for packaging. Changing the
# constant's spelling fails configure below.
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

install(FILES "${CMAKE_SOURCE_DIR}/scripts/mo2-salma.py"
        DESTINATION plugins
        COMPONENT MO2_PLUGIN)
# The plugin DLL is the RUST artifact, which CMake does not build. Staged by
# `python scripts/package.py` into target/package/mo2-salma.dll and installed from
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
# The dashboard bundle ships the harness helpers common.py, scan.py, install.py and the compare.py
# CLI. The excludes below drop build-time scripts that only work inside a source tree;
# _clean_docs.py and _promote_subgroups.py are doc-site tooling and currently ship too.
install(DIRECTORY "${CMAKE_SOURCE_DIR}/scripts/"
        DESTINATION dashboard/scripts
        COMPONENT DASHBOARD
        REGEX "/__pycache__($|/)" EXCLUDE
        PATTERN "*.pyc" EXCLUDE
        PATTERN "package.py" EXCLUDE
        PATTERN "run_harness.py" EXCLUDE
        PATTERN "smoke_ctypes.py" EXCLUDE
        PATTERN "smoke_plugin.py" EXCLUDE)
install(FILES
            "${CMAKE_SOURCE_DIR}/test_all.py"
            "${CMAKE_SOURCE_DIR}/test_one.py"
            "${CMAKE_SOURCE_DIR}/test.bat"
            "${CMAKE_SOURCE_DIR}/deploy.bat"
            "${CMAKE_SOURCE_DIR}/purge.bat"
            "${CMAKE_SOURCE_DIR}/setup.bat"
        DESTINATION dashboard
        COMPONENT DASHBOARD)

# Top-level metadata: VERSION always, README if present.
install(FILES "${CMAKE_BINARY_DIR}/VERSION"
        DESTINATION .
        COMPONENT MO2_PLUGIN)
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
