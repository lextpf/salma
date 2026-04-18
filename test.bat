@echo off
REM ============================================================================
REM test.bat - Run the salma Rust test suite and the ABI smoke tests
REM ============================================================================
REM This script:
REM   1. cargo test        - the full unit + golden-fixture suite (release)
REM   2. smoke_ctypes.py   - raw C ABI surface through ctypes
REM   3. smoke_plugin.py   - the MO2 plugin's OWN find_dll / load_dll /
REM                          _configure_dll / _check_api_version, run verbatim
REM                          against the packaged DLL
REM
REM All three are corpus-free. The corpus-backed parity checks are separate and
REM need the SALMA_* env vars (scripts\setup-env.bat):
REM   python rust\tools\compare_infer.py rust\target\release\mo2_salma_rs.dll --curated
REM   python rust\tools\run_harness.py
REM
REM CARGO_BUILD_JOBS is capped for the same reason as in build.bat.
REM ============================================================================

setlocal enabledelayedexpansion

echo ============================================================================
echo                       SALMA TEST RUNNER (Rust)
echo ============================================================================
echo.

set "REPO_ROOT=%~dp0"
if "%REPO_ROOT:~-1%"=="\" set "REPO_ROOT=%REPO_ROOT:~0,-1%"
set "RUST_DIR=%REPO_ROOT%\rust"

if not defined CARGO_BUILD_JOBS set "CARGO_BUILD_JOBS=4"

where cargo >nul 2>&1
if errorlevel 1 (
    echo ERROR: cargo not found in PATH. Install Rust from https://rustup.rs
    exit /b 1
)

set ALL_PASSED=1

REM ============================================================================
REM STEP 1: Cargo test
REM ============================================================================
echo [1/3] Running cargo test --release...
echo ----------------------------------------------------------------------------
cargo test --manifest-path "%RUST_DIR%\Cargo.toml" --release
if errorlevel 1 set ALL_PASSED=0
echo.

REM ============================================================================
REM STEP 2: Raw ABI smoke test
REM ============================================================================
echo [2/3] Running the C ABI smoke test...
echo ----------------------------------------------------------------------------
where python >nul 2>&1
if errorlevel 1 (
    echo SKIP: python not found in PATH
) else (
    if not exist "%RUST_DIR%\target\release\mo2_salma_rs.dll" (
        echo ERROR: rust\target\release\mo2_salma_rs.dll not found. Run build.bat first.
        set ALL_PASSED=0
    ) else (
        python "%RUST_DIR%\tools\smoke_ctypes.py" "%RUST_DIR%\target\release\mo2_salma_rs.dll"
        if errorlevel 1 set ALL_PASSED=0
    )
)
echo.

REM ============================================================================
REM STEP 3: Plugin-loader smoke test
REM ============================================================================
echo [3/3] Running the MO2 plugin-loader smoke test...
echo ----------------------------------------------------------------------------
where python >nul 2>&1
if errorlevel 1 (
    echo SKIP: python not found in PATH
) else (
    if not exist "%RUST_DIR%\target\package\mo2-salma.dll" (
        echo   Staging the packaged DLL first...
        python "%RUST_DIR%\tools\package.py" --no-build
        if errorlevel 1 set ALL_PASSED=0
    )
    python "%RUST_DIR%\tools\smoke_plugin.py"
    if errorlevel 1 set ALL_PASSED=0
)
echo.

REM ============================================================================
REM SUMMARY
REM ============================================================================
echo ============================================================================
if %ALL_PASSED%==1 (
    echo                            ALL TESTS PASSED
) else (
    echo                            SOME TESTS FAILED
)
echo ============================================================================

endlocal & set "SALMA_TESTS_PASSED=%ALL_PASSED%"
if "%SALMA_NO_PAUSE%"=="1" goto :result
pause
:result
if "%SALMA_TESTS_PASSED%"=="1" (
    exit /b 0
) else (
    exit /b 1
)
