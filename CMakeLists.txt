# @brief build and package the C++ server and support library.
# @author Alex (https://github.com/lextpf)
#

cmake_minimum_required(VERSION 3.20)
project(salma VERSION 1.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# only the Ninja `compile-db` preset writes `compile_commands.json`.
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

enable_testing()

# the C++ targets load the separately built Rust engine through `SalmaEngine.cpp`.

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

# both binaries load the DLL beside the executable. a tests-only build skips this server hook.
# stage `target/package/mo2-salma.dll` before configure because `EXISTS` is evaluated then.
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

# CPack puts the MO2 plugin and dashboard in one archive. the engine is not duplicated.
# `MO2_SALMA_API_VERSION` owns the package version. its spelling is part of this parser contract.
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
# install the separately staged Rust artifact when it is present.
install(FILES "${CMAKE_SOURCE_DIR}/target/package/mo2-salma.dll"
        DESTINATION plugins/salma
        COMPONENT MO2_PLUGIN
        OPTIONAL)
install(TARGETS mo2-server
        RUNTIME DESTINATION dashboard
        COMPONENT DASHBOARD)
install(DIRECTORY "${CMAKE_SOURCE_DIR}/web/dist/"
        DESTINATION dashboard/web/dist
        COMPONENT DASHBOARD
        OPTIONAL)
# ship runtime helpers, but omit source-tree packaging and smoke tools.
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
set(CPACK_ARCHIVE_COMPONENT_INSTALL OFF)   # one archive contains both components
include(CPack)
