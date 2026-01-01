@echo off
REM ============================================================================
REM build.bat - Complete build pipeline for salma
REM ============================================================================
REM This script:
REM   1. Formats source code with clang-format (if available)
REM   2. Configures the project using CMake with the default preset
REM   3. Builds the Release configuration
REM   4. Generates API documentation with doxide (if available)
REM   5. Builds the documentation site with mkdocs (if available)
REM ============================================================================

setlocal enabledelayedexpansion

echo ============================================================================
echo                            SALMA BUILD PIPELINE
echo ============================================================================
echo.

REM ============================================================================
REM STEP 1: Format Source Code (clang-format)
REM ============================================================================
echo [1/5] Formatting source code...
echo ----------------------------------------------------------------------------
where clang-format >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo SKIP: clang-format not found in PATH
) else (
    for %%f in (src\*.cpp src\*.h src\*.hpp src\*.c) do (
        if exist "%%f" (
            clang-format -i "%%f"
        )
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
REM STEP 3: Build Release
REM ============================================================================
echo [3/5] Building Release...
echo ----------------------------------------------------------------------------
cmake --build build --config Release
if %ERRORLEVEL% neq 0 (
    echo ERROR: Build failed
    exit /b %ERRORLEVEL%
)
echo.

REM ============================================================================
REM STEP 4: Generate API Documentation (doxide)
REM ============================================================================
echo [4/5] Generating API documentation...
echo ----------------------------------------------------------------------------
where doxide >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo SKIP: doxide not found in PATH
) else (
    doxide build
    if %ERRORLEVEL% neq 0 (
        echo ERROR: Doxide failed
        exit /b %ERRORLEVEL%
    )
    python scripts/_promote_subgroups.py
    python scripts/_clean_docs.py
)
echo.

REM ============================================================================
REM STEP 5: Build Documentation Site (mkdocs)
REM ============================================================================
echo [5/5] Building documentation site...
echo ----------------------------------------------------------------------------
where mkdocs >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo SKIP: mkdocs not found in PATH
) else (
    mkdocs build
    if %ERRORLEVEL% neq 0 (
        echo ERROR: MkDocs failed
        exit /b %ERRORLEVEL%
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
echo   - DLL:    build\bin\Release\mo2-salma.dll
echo   - Server: build\bin\Release\mo2-server.exe
echo.
echo Documentation:
echo   - API:  docs\  (if doxide available)
echo   - Site: site\  (if mkdocs available)
echo.
echo ============================================================================

endlocal
exit /b 0
