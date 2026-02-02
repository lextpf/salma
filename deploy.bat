@echo off
REM ============================================================================
REM deploy.bat - Deploy salma plugin to MO2
REM ============================================================================
REM This script:
REM   1. Verifies the build exists
REM   2. Copies the DLL to the MO2 plugins/salma subdirectory
REM   3. Copies the Python plugin to the MO2 plugins directory
REM ============================================================================

setlocal

echo ============================================================================
echo                           SALMA DEPLOY SCRIPT
echo ============================================================================
echo.

:: MO2 plugins folder. Required - run scripts\setup-env.bat once to configure.
if not defined SALMA_DEPLOY_PATH (
    echo ERROR: SALMA_DEPLOY_PATH is not set.
    echo Run scripts\setup-env.bat once to configure paths,
    echo or set the variable manually for this shell.
    exit /b 1
)
set "DEPLOY_PATH=%SALMA_DEPLOY_PATH%"

REM ============================================================================
REM STEP 1: Verify Build
REM ============================================================================
REM Resolve paths relative to this script (%~dp0), not the calling shell's cwd,
REM so the dashboard can invoke deploy.bat from any working directory.
echo [1/3] Verifying build...
echo ----------------------------------------------------------------------------
set "DLL_SOURCE=%~dp0build\bin\Release\mo2-salma.dll"
if exist "%~dp0mo2-salma.dll" set "DLL_SOURCE=%~dp0mo2-salma.dll"
if not exist "%DLL_SOURCE%" (
    echo ERROR: %DLL_SOURCE% not found
    echo Run build.bat first
    exit /b 1
)
echo   Found: %DLL_SOURCE%
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
