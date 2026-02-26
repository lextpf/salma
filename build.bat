@echo off
REM ===========================================================================================
REM build.bat - Complete build pipeline for salma
REM ===========================================================================================
REM This script:
REM   1. clang-format - in-place formatting of src/*.cpp / src/*.hpp / tests/*.cpp
REM   2. cmake        - CMake configure with vcpkg manifest install and VS 17 2022 generator
REM   3. clang-tidy   - static analysis (sequential; fails the build on any reported issue)
REM   4. build        - release build of the Release configuration via cmake --build
REM   5. doxide       - API documentation generation via doxide + mkdocs build
REM ===========================================================================================

setlocal enabledelayedexpansion

echo ============================================================================
echo                            SALMA BUILD PIPELINE
echo ============================================================================
echo.

REM ============================================================================
REM STEP 1: Run clang-format
REM ============================================================================
echo [1/5] Running clang-format...
echo ----------------------------------------------------------------------------

where clang-format >nul 2>&1
if errorlevel 1 (
    echo SKIP: clang-format not found in PATH
) else (
    for %%f in (src\*.cpp src\*.hpp tests\*.cpp) do (
        if exist "%%f" clang-format -i "%%f"
    )
    echo Formatting complete.
)
echo.

REM ============================================================================
REM STEP 2: CMake Configuration
REM ============================================================================
echo [2/5] Configuring with CMake...
echo ----------------------------------------------------------------------------
cmake --preset default
if %ERRORLEVEL% neq 0 (
    echo ERROR: CMake configuration failed
    exit /b %ERRORLEVEL%
)
echo.

REM ============================================================================
REM STEP 3: Run clang-tidy
REM ============================================================================
echo [3/5] Running clang-tidy...
echo ----------------------------------------------------------------------------

where clang-tidy >nul 2>&1
if errorlevel 1 (
    echo SKIP: clang-tidy not found in PATH
) else (
    if not exist "build-cdb\compile_commands.json" (
        echo   Generating compile_commands.json via Ninja sidecar...
        cmake --preset compile-db >nul
        if !ERRORLEVEL! neq 0 (
            echo ERROR: compile-db configure failed
            exit /b 1
        )
    )

    for %%f in (src\*.cpp tests\*.cpp) do (
        if exist "%%f" (
            echo   tidy: %%f
            clang-tidy --quiet --header-filter="[/\\]%%~nf\.hpp$" -p build-cdb "%%f"
            if !ERRORLEVEL! neq 0 (
                echo ERROR: clang-tidy reported issues in %%f
                exit /b 1
            )
        )
    )
    echo clang-tidy complete.
)
echo.

REM ============================================================================
REM STEP 4: Build Release
REM ============================================================================
echo [4/5] Building Release...
echo ----------------------------------------------------------------------------
cmake --build build --config Release
if %ERRORLEVEL% neq 0 (
    echo ERROR: Build failed
    exit /b %ERRORLEVEL%
)
echo.

REM ============================================================================
REM STEP 5: Generate Documentation (doxide + mkdocs)
REM ============================================================================
echo [5/5] Generating documentation...
echo ----------------------------------------------------------------------------
where doxide >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo SKIP: doxide not found in PATH
) else (
    doxide build
    if !ERRORLEVEL! neq 0 (
        echo ERROR: doxide build failed [exit code !ERRORLEVEL!]
        exit /b 1
    )
    python scripts/_promote_subgroups.py
    if !ERRORLEVEL! neq 0 (
        echo ERROR: _promote_subgroups.py failed [exit code !ERRORLEVEL!]
        exit /b 1
    )
    python scripts/_clean_docs.py
    if !ERRORLEVEL! neq 0 (
        echo ERROR: _clean_docs.py failed [exit code !ERRORLEVEL!]
        exit /b 1
    )
)

where mkdocs >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo SKIP: mkdocs not found in PATH
) else (
    mkdocs build
    if !ERRORLEVEL! neq 0 (
        echo ERROR: mkdocs build failed [exit code !ERRORLEVEL!]
        exit /b 1
    )
)
echo.

REM ============================================================================
REM SUMMARY
REM ============================================================================
echo ============================================================================
echo                           BUILD PIPELINE COMPLETE
echo ============================================================================
echo.
echo Build Output:
echo   Release: build\bin\Release\mo2-salma.dll
echo            build\bin\Release\mo2-server.exe
echo   Linkage: Dynamic (/MD, x64-windows-static-md)
echo.
echo Documentation:
echo   - Md:   docs\  (if doxide available)
echo   - Html: site\  (if mkdocs available)
echo.
echo  *** Run deploy.bat to install the plugin as an MO2 mod ***
echo.
echo ============================================================================

endlocal
exit /b 0
