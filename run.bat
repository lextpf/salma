@echo off
REM ===========================================================================================
REM run.bat - Run the salma dashboard: HTTP server + Vite dev server
REM ===========================================================================================
REM Opens two windows and then the browser:
REM   1. build\bin\Release\mo2-server.exe  - the Crow HTTP server on :5000 (the /api/* backend)
REM   2. web\npm run dev                   - the Vite dev server on :3000 (the React SPA, HMR)
REM   3. http://localhost:3000             - once both answer
REM
REM Vite proxies /api -> http://localhost:5000, so the SPA is served from :3000 and the
REM backend from :5000. Both windows stay open after the process ends so a startup error
REM is readable; close a window or press Ctrl-C in it to stop that half.
REM ===========================================================================================

setlocal

echo ============================================================================
echo                          SALMA DASHBOARD
echo ============================================================================
echo.

set "REPO_ROOT=%~dp0"
if "%REPO_ROOT:~-1%"=="\" set "REPO_ROOT=%REPO_ROOT:~0,-1%"

set "SERVER_DIR=%REPO_ROOT%\build\bin\Release"
set "SERVER_EXE=%SERVER_DIR%\mo2-server.exe"
set "API_PROBE=http://127.0.0.1:5000/api/csrf-token"
set "WEB_PROBE=http://localhost:3000/"
set "WEB_URL=http://localhost:3000"

REM ============================================================================
REM Preflight
REM ============================================================================
if not exist "%SERVER_EXE%" (
    echo ERROR: mo2-server.exe not found at
    echo   %SERVER_EXE%
    echo.
    echo Run build.bat - its step 5 builds the server. It skips silently when cmake is
    echo missing or VCPKG_ROOT is unset, which is the likely reason it is not here. The
    echo commands it runs are:
    echo   cmake --preset default
    echo   cmake --build build --config Release --target mo2-server
    exit /b 1
)

set "ENGINE_DLL=%SERVER_DIR%\mo2-salma.dll"
set "ENGINE_PACKAGED=%REPO_ROOT%\target\package\mo2-salma.dll"
if not exist "%ENGINE_DLL%" (
    echo WARNING: mo2-salma.dll is not next to mo2-server.exe.
    echo          The dashboard will start but every install/infer request will fail.
    echo          Fix: run build.bat, then copy target\package\mo2-salma.dll into
    echo          %SERVER_DIR%\ ^(or rebuild mo2-server, which copies it^).
    echo.
    goto :engine_checked
)

REM CMake only refreshes that copy when the C++ is rebuilt, so a plain build.bat leaves
REM the server loading whatever engine was current the last time mo2-server was built.
REM That mismatch is silent at runtime and easy to chase for an hour, so compare bytes.
if not exist "%ENGINE_PACKAGED%" goto :engine_checked
fc /b "%ENGINE_DLL%" "%ENGINE_PACKAGED%" >nul 2>&1
if not errorlevel 1 goto :engine_checked
echo WARNING: the engine next to mo2-server.exe differs from the packaged build.
echo          The dashboard would run a STALE engine. Refresh it with:
echo            copy /y "%ENGINE_PACKAGED%" "%SERVER_DIR%\"
echo.

:engine_checked

where npm >nul 2>&1
if errorlevel 1 (
    echo ERROR: npm not found in PATH. Install Node.js from https://nodejs.org
    exit /b 1
)

if not exist "%REPO_ROOT%\web\node_modules" (
    echo web\node_modules missing, running npm install...
    pushd "%REPO_ROOT%\web"
    call npm install
    if errorlevel 1 goto :install_failed
    popd
    echo.
)

REM ============================================================================
REM Launch
REM ============================================================================
echo Starting the HTTP server on :5000...
start "salma server (:5000)" /D "%SERVER_DIR%" cmd /k ""%SERVER_EXE%""

echo Starting the Vite dev server on :3000...
start "salma web (:3000)" /D "%REPO_ROOT%\web" cmd /k npm run dev

echo.
call :wait_for "%API_PROBE%" "HTTP server" "5000"
call :wait_for "%WEB_PROBE%" "Vite dev server" "3000"

echo.
echo Opening %WEB_URL% ...
start "" "%WEB_URL%"

REM ============================================================================
REM Summary
REM ============================================================================
echo.
echo ============================================================================
echo                              SALMA RUNNING
echo ============================================================================
echo.
echo   Server:  http://localhost:5000   ^(salma server^)
echo   Web UI:  %WEB_URL%   ^(salma UI^)
echo.
echo   Ctrl-C in either window stops that half.
echo.
echo ============================================================================

endlocal
exit /b 0

REM ============================================================================
REM Reached only by GOTO from the preflight
REM ============================================================================
:install_failed
echo ERROR: npm install failed
popd
endlocal
exit /b 1

REM ============================================================================
REM :wait_for <url> <label> <port>
REM Polls the URL up to 10 times, about two seconds apart. Any HTTP response counts
REM as up, including 403 - the API is CSRF/origin-gated and answering at all proves
REM the listener is live.
REM ============================================================================
:wait_for
setlocal
set "URL=%~1"
set "LABEL=%~2"
set "PORT=%~3"
REM ping, not timeout, is the one-second sleep: timeout aborts with "Input redirection
REM is not supported" whenever stdin is redirected, which would spin this loop instantly.
where curl >nul 2>&1
if errorlevel 1 (
    ping -n 6 127.0.0.1 >nul 2>&1
    endlocal
    exit /b 0
)

echo Waiting for the %LABEL% on :%PORT% ...
for /l %%i in (1,1,10) do (
    curl -s -o NUL --max-time 1 "%URL%" >nul 2>&1
    if not errorlevel 1 (
        echo   %LABEL% is up.
        endlocal
        exit /b 0
    )
    ping -n 2 127.0.0.1 >nul 2>&1
)
echo WARNING: the %LABEL% did not answer on :%PORT% after 10 attempts ^(~20s^).
echo          Check its window for the error; the browser opens anyway.
endlocal
exit /b 0
