@echo off
REM ===========================================================================================
REM build.bat - Complete build pipeline for salma (Rust engine)
REM ===========================================================================================
REM This script:
REM   1. cargo fmt    - in-place formatting of src and tests
REM   2. cargo clippy - static analysis over all targets; any warning fails the build
REM   3. cargo build  - release build of the cdylib -> target\release\mo2_salma_rs.dll
REM   4. package.py   - stage the deployable artifact as mo2-salma.dll with its SHA-256
REM
REM The C++ engine is no longer built here. It still lives in src/ as the parity
REM oracle for tools\gen_golden.py and run_harness.py; build it directly with
REM   cmake --preset default
REM   cmake --build build --config Release
REM
REM CARGO_BUILD_JOBS is capped below. Cargo defaults to one job per core, and on a
REM high-core machine the resulting parallel rustc + link peak has been observed to
REM take the toolchain down (rustc STATUS_HEAP_CORRUPTION). Set CARGO_BUILD_JOBS
REM before calling this script to override.
REM ===========================================================================================

setlocal enabledelayedexpansion

echo ============================================================================
echo                       SALMA BUILD PIPELINE (Rust)
echo ============================================================================
echo.

set "REPO_ROOT=%~dp0"
if "%REPO_ROOT:~-1%"=="\" set "REPO_ROOT=%REPO_ROOT:~0,-1%"
REM Cargo and CMake share the repo root; cargo runs from here directly.

if not defined CARGO_BUILD_JOBS set "CARGO_BUILD_JOBS=4"
echo Using CARGO_BUILD_JOBS=%CARGO_BUILD_JOBS%
echo.

where cargo >nul 2>&1
if errorlevel 1 (
    echo ERROR: cargo not found in PATH. Install Rust from https://rustup.rs
    exit /b 1
)

REM ============================================================================
REM STEP 1: Format
REM ============================================================================
echo [1/4] Running cargo fmt...
echo ----------------------------------------------------------------------------
cargo fmt
if errorlevel 1 (
    echo ERROR: cargo fmt failed
    exit /b 1
)
echo Formatting complete.
echo.

REM ============================================================================
REM STEP 2: Clippy
REM ============================================================================
echo [2/4] Running cargo clippy...
echo ----------------------------------------------------------------------------
cargo clippy --all-targets --release -- -D warnings
if errorlevel 1 (
    echo ERROR: clippy reported issues
    exit /b 1
)
echo Clippy clean.
echo.

REM ============================================================================
REM STEP 3: Build Release
REM ============================================================================
echo [3/4] Building Release...
echo ----------------------------------------------------------------------------
cargo build --release
if errorlevel 1 (
    echo ERROR: Build failed
    exit /b 1
)
echo.

REM ============================================================================
REM STEP 4: Package the deployable artifact
REM ============================================================================
echo [4/4] Staging the deployable DLL...
echo ----------------------------------------------------------------------------
where python >nul 2>&1
if errorlevel 1 (
    echo SKIP: python not found in PATH, artifact not staged
) else (
    python "%REPO_ROOT%\tools\package.py" --no-build
    if errorlevel 1 (
        echo ERROR: package.py failed
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
echo   Built:      target\release\mo2_salma_rs.dll
echo   Deployable: target\package\mo2-salma.dll
echo.
echo  *** Run test.bat to verify, deploy.bat to install into MO2 ***
echo.
echo ============================================================================

endlocal
exit /b 0
