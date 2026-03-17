cmake_minimum_required(VERSION 3.20)
project(salma VERSION 1.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# --- mo2-core (shared library) ---

find_package(pugixml CONFIG REQUIRED)
find_package(nlohmann_json CONFIG REQUIRED)
find_package(LibArchive REQUIRED)
find_package(unofficial-bit7z CONFIG REQUIRED)

add_library(mo2-core SHARED
    src/Utils.cpp
    src/Logger.cpp
    src/FileOperations.cpp
    src/ArchiveService.cpp
    src/ModStructureDetector.cpp
    src/FomodDependencyEvaluator.cpp
    src/FomodService.cpp
    src/FomodIRParser.cpp
    src/FomodForwardSimulator.cpp
    src/FomodCSPSolver.cpp
    src/FomodPropagator.cpp
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
    src/ConfigService.cpp
    src/MultipartHandler.cpp
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

add_executable(salma_tests
    tests/utils_test.cpp
    tests/fomod_inference_test.cpp
)

target_link_libraries(salma_tests
    PRIVATE mo2-core
    PRIVATE GTest::gtest_main
    PRIVATE pugixml::pugixml
)

set_target_properties(salma_tests PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin"
)

# --- Copy test suite + scripts next to the exe ---

# The output dir varies by config: bin/Release, bin/Debug, etc.
# Use a generator expression so the files land beside the actual exe.
set(OUTDIR "$<TARGET_FILE_DIR:mo2-server>")

add_custom_command(TARGET mo2-server POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${CMAKE_SOURCE_DIR}/test.bat"
        "${CMAKE_SOURCE_DIR}/test.py"
        "${CMAKE_SOURCE_DIR}/test_one.py"
        "${CMAKE_SOURCE_DIR}/deploy.bat"
        "${CMAKE_SOURCE_DIR}/purge.bat"
        "${OUTDIR}/"
    COMMAND ${CMAKE_COMMAND} -E copy_directory
        "${CMAKE_SOURCE_DIR}/scripts"
        "${OUTDIR}/scripts"
    COMMENT "Copying test suite and scripts to output directory"
)
