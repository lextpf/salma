@echo off
REM ============================================================================
REM setup-env.bat - One-time configuration of MO2 paths used by salma scripts
REM ============================================================================
REM Prompts for the three MO2-instance paths salma's tooling needs and writes
REM them to the user's environment via setx so they persist across new shells.
REM
REM This replaces the previous Nolvus-specific defaults that were baked into
REM deploy.bat / purge.bat / scripts\common.py. After running this once, open
REM a fresh shell and the scripts will pick up the configured paths.
REM ============================================================================

setlocal

echo ============================================================================
echo                       SALMA ENVIRONMENT SETUP
echo ============================================================================
echo.
echo This will set three persistent user-environment variables for salma:
echo   SALMA_MODS_PATH      - MO2 mods directory
echo   SALMA_DEPLOY_PATH    - MO2 plugins directory
echo   SALMA_DOWNLOADS_PATH - MO2 downloads directory
echo.
echo Tip: paste paths *without* surrounding quotes.
echo.

set /p MODS_PATH=MO2 mods path (e.g. D:\YourInstance\mods):
if not defined MODS_PATH (
    echo ERROR: mods path is required, aborting.
    exit /b 1
)
set /p DEPLOY_PATH=MO2 plugins path (e.g. D:\YourInstance\plugins):
if not defined DEPLOY_PATH (
    echo ERROR: plugins path is required, aborting.
    exit /b 1
)
set /p DOWNLOADS_PATH=MO2 downloads path (e.g. D:\YourInstance\downloads):
if not defined DOWNLOADS_PATH (
    echo ERROR: downloads path is required, aborting.
    exit /b 1
)

setx SALMA_MODS_PATH      "%MODS_PATH%"      >nul
setx SALMA_DEPLOY_PATH    "%DEPLOY_PATH%"    >nul
setx SALMA_DOWNLOADS_PATH "%DOWNLOADS_PATH%" >nul

echo.
echo ============================================================================
echo                       SETUP COMPLETE
echo ============================================================================
echo.
echo SALMA_MODS_PATH      = %MODS_PATH%
echo SALMA_DEPLOY_PATH    = %DEPLOY_PATH%
echo SALMA_DOWNLOADS_PATH = %DOWNLOADS_PATH%
echo.
echo Open a new shell to pick up the changes.
echo ============================================================================

endlocal
if "%SALMA_NO_PAUSE%"=="1" exit /b 0
pause
