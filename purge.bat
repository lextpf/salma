@echo off
REM ============================================================================
REM purge.bat - Remove salma plugin from MO2 and clean fomod choices
REM ============================================================================
REM This script removes the salma plugin files:
REM   - salma\mo2-salma.dll (and the salma subdirectory)
REM   - mo2-salma.py
REM   - Salma FOMODs Output mod folder
REM   - logs\mo_salma.log
REM ============================================================================

setlocal enabledelayedexpansion

echo ============================================================================
echo                           SALMA PURGE SCRIPT
echo ============================================================================
echo.

:: MO2 plugins/mods folders. Required - run scripts\setup-env.bat once to configure.
if not defined SALMA_DEPLOY_PATH (
    echo ERROR: SALMA_DEPLOY_PATH is not set.
    echo Run scripts\setup-env.bat once to configure paths,
    echo or set the variable manually for this shell.
    exit /b 1
)
if not defined SALMA_MODS_PATH (
    echo ERROR: SALMA_MODS_PATH is not set.
    echo Run scripts\setup-env.bat once to configure paths,
    echo or set the variable manually for this shell.
    exit /b 1
)
set "DEPLOY_PATH=%SALMA_DEPLOY_PATH%"
set "MODS_PATH=%SALMA_MODS_PATH%"

echo Plugins: %DEPLOY_PATH%
echo Mods:    %MODS_PATH%
echo.
echo ============================================================================
echo.

REM ============================================================================
REM Remove DLL and subdirectory
REM ============================================================================
echo [1/4] Removing salma\mo2-salma.dll...
if exist "%DEPLOY_PATH%\salma\mo2-salma.dll" (
    del /Q "%DEPLOY_PATH%\salma\mo2-salma.dll"
    echo   Removed: salma\mo2-salma.dll
) else (
    echo   Skipped: salma\mo2-salma.dll not found
)
if exist "%DEPLOY_PATH%\salma" (
    rmdir "%DEPLOY_PATH%\salma" 2>nul
    if not exist "%DEPLOY_PATH%\salma" (
        echo   Removed: salma\
    )
)

REM ============================================================================
REM Remove Python Plugin
REM ============================================================================
echo [2/4] Removing mo2-salma.py...
if exist "%DEPLOY_PATH%\mo2-salma.py" (
    del /Q "%DEPLOY_PATH%\mo2-salma.py"
    echo   Removed: mo2-salma.py
) else (
    echo   Skipped: mo2-salma.py not found
)

REM ============================================================================
REM Remove Salma FOMOD Output mod folder
REM ============================================================================
echo [3/4] Removing "Salma FOMODs Output" mod folder...
if exist "%MODS_PATH%\Salma FOMODs Output" (
    rmdir /S /Q "%MODS_PATH%\Salma FOMODs Output"
    echo   Removed: Salma FOMODs Output
) else (
    echo   Skipped: "Salma FOMODs Output" not found
)

REM ============================================================================
REM Remove log file
REM ============================================================================
REM The MO2 plugin writes to <MO2 plugins>/logs/mo_salma.log (anchored to
REM the .py file), so purge looks under DEPLOY_PATH, not its parent.
set "LOG_DIR=%DEPLOY_PATH%\logs"
echo [4/4] Removing mo_salma.log...
if exist "%LOG_DIR%\mo_salma.log" (
    del /Q "%LOG_DIR%\mo_salma.log"
    echo   Removed: mo_salma.log
) else (
    echo   Skipped: mo_salma.log not found
)

:done
echo.
echo ============================================================================
echo                            PURGE COMPLETE
echo ============================================================================
echo.

endlocal
if "%SALMA_NO_PAUSE%"=="1" exit /b 0
pause
