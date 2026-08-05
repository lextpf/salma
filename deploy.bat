@echo off
REM ============================================================================
REM deploy.bat - Deploy salma plugin to MO2
REM ============================================================================
REM This script:
REM   1. Verifies the build exists (prefers a DLL beside this script, then the
REM      packaged Rust artifact)
REM   2. Copies the DLL to the MO2 plugins/salma subdirectory
REM   3. Copies the Python plugin to the MO2 plugins directory
REM
REM Requires SALMA_DEPLOY_PATH. SALMA_NO_PAUSE=1 skips the trailing pause, and is
REM what the dashboard sets when it runs this headless.
REM ============================================================================

setlocal

echo ============================================================================
echo                           SALMA DEPLOY SCRIPT
echo ============================================================================
echo.

:: MO2 plugins folder. Required - run setup.bat once to configure.
if not defined SALMA_DEPLOY_PATH (
    echo ERROR: SALMA_DEPLOY_PATH is not set.
    echo Run setup.bat once to configure paths,
    echo or set the variable manually for this shell.
    exit /b 1
)
set "DEPLOY_PATH=%SALMA_DEPLOY_PATH%"

REM ============================================================================
REM STEP 1: Verify Build
REM ============================================================================
REM Candidates resolve against this script (%~dp0), not the calling shell's cwd,
REM and this script runs from two places.
echo [1/3] Verifying build...
echo ----------------------------------------------------------------------------
REM From the repo root, in precedence order:
REM   1. %~dp0mo2-salma.dll     - hand-placed override, wins over everything
REM   2. target\package\...     - the packaged Rust artifact build.bat stages
REM   3. build\bin\Release\...  - only when the packaged artifact is absent
REM
REM From <exe dir>, where CMake stages a copy of this script beside
REM mo2-server.exe and the dashboard runs it, candidates 2 and 3 resolve under
REM <exe dir> and cannot exist. Only candidate 1 hits, and there it is the
REM engine CMake copied next to the exe. Dropping it breaks /api/plugin/deploy.
REM
REM No candidate is a C++ build of the engine: no C++ engine exists. Candidate 3
REM is the SAME Rust DLL, put there by a CMake POST_BUILD step so mo2-server.exe
REM can load it from beside itself, so it can be STALE if the C++ was built
REM before the last package.py run.
set "DLL_SOURCE=%~dp0target\package\mo2-salma.dll"
if not exist "%DLL_SOURCE%" set "DLL_SOURCE=%~dp0build\bin\Release\mo2-salma.dll"
if exist "%~dp0mo2-salma.dll" set "DLL_SOURCE=%~dp0mo2-salma.dll"
if not exist "%DLL_SOURCE%" (
    echo ERROR: %DLL_SOURCE% not found
    echo Run build.bat first
    exit /b 1
)
echo   Found: %DLL_SOURCE%
echo.
echo   NOTE: MO2 loads whatever sits at that path, and getApiVersion reports
echo         1.2.0 for every build, so the deployed file carries no marker of
echo         which bytes it is. Back up the DLL you are replacing and verify the
echo         new one against the SHA-256 package.py printed. CUTOVER.md has the
echo         full procedure and the rollback.
echo.

REM ============================================================================
REM STEP 2: Copy DLL
REM ============================================================================
echo [2/3] Copying DLL...
echo ----------------------------------------------------------------------------
if not exist "%DEPLOY_PATH%" (
    echo ERROR: Deploy path not found: %DEPLOY_PATH%
    exit /b 1
)
if not exist "%DEPLOY_PATH%\salma" mkdir "%DEPLOY_PATH%\salma"
copy /Y "%DLL_SOURCE%" "%DEPLOY_PATH%\salma\mo2-salma.dll"
if %ERRORLEVEL% neq 0 (
    echo ERROR: Failed to copy DLL
    exit /b %ERRORLEVEL%
)
echo.

REM ============================================================================
REM STEP 3: Copy Python Plugin
REM ============================================================================
echo [3/3] Copying Python plugin...
echo ----------------------------------------------------------------------------
REM Same override as the DLL: a mo2-salma.py beside this script wins over
REM scripts\mo2-salma.py. Run from beside mo2-server.exe the default candidate
REM still resolves, because CMake copies the whole scripts directory there.
set "PY_SOURCE=%~dp0scripts\mo2-salma.py"
if exist "%~dp0mo2-salma.py" set "PY_SOURCE=%~dp0mo2-salma.py"
copy /Y "%PY_SOURCE%" "%DEPLOY_PATH%\mo2-salma.py"
if %ERRORLEVEL% neq 0 (
    echo ERROR: Failed to copy Python plugin
    exit /b %ERRORLEVEL%
)
echo.

REM ============================================================================
REM SUMMARY
REM ============================================================================
echo ============================================================================
echo                            DEPLOY COMPLETE
echo ============================================================================
echo.
echo Target: %DEPLOY_PATH%
echo.
echo Deployed Files:
echo   - salma\mo2-salma.dll
echo   - mo2-salma.py
echo.
echo  *** Restart MO2 to load the updated plugin ***
echo.
echo ============================================================================

endlocal
if "%SALMA_NO_PAUSE%"=="1" exit /b 0
pause
