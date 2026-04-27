@echo off
REM ===========================================================================================
REM build.bat - Complete build pipeline for salma
REM ===========================================================================================
REM This script:
REM   1. cargo fmt    - in-place formatting of src and tests
REM   2. cargo clippy - static analysis over all targets; any warning fails the build
REM   3. cargo build  - release build of the cdylib -> target\release\mo2_salma_rs.dll
REM   4. package.py   - stage the deployable artifact as mo2-salma.dll with its SHA-256
REM   5. cmake        - build mo2-server.exe, the Crow HTTP server behind the dashboard
REM   6. npm build    - type-check and bundle the React dashboard -> web\dist
REM   7. docs         - rustdoc for the engine + doxide/mkdocs for the C++ -> site\
REM
REM ===========================================================================================

setlocal enabledelayedexpansion

echo ============================================================================
echo                       SALMA BUILD PIPELINE
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
echo [1/7] Running cargo fmt...
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
echo [2/7] Running cargo clippy...
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
echo [3/7] Building Release...
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
echo [4/7] Staging the deployable DLL...
echo ----------------------------------------------------------------------------
where python >nul 2>&1
if errorlevel 1 (
    echo SKIP: python not found in PATH, artifact not staged
) else (
    python "%REPO_ROOT%\scripts\package.py" --no-build
    if errorlevel 1 (
        echo ERROR: package.py failed
        exit /b 1
    )
)

if exist "%REPO_ROOT%\build\bin\Release\mo2-server.exe" (
    if exist "%REPO_ROOT%\target\package\mo2-salma.dll" (
        copy /y "%REPO_ROOT%\target\package\mo2-salma.dll" "%REPO_ROOT%\build\bin\Release\" >nul 2>&1
        if errorlevel 1 (
            echo WARNING: could not refresh build\bin\Release\mo2-salma.dll.
            echo          Is mo2-server.exe running? Stop it and re-run build.bat.
        ) else (
            echo Refreshed the engine next to mo2-server.exe.
        )
    )
)
echo.

REM ============================================================================
REM STEP 5: Build the C++ server
REM ============================================================================
echo [5/7] Building the C++ server...
echo ----------------------------------------------------------------------------
set "CPP_BUILT="
where cmake >nul 2>&1
if errorlevel 1 (
    echo SKIP: cmake not found in PATH, mo2-server not built
    goto :cpp_done
)
if not defined VCPKG_ROOT (
    echo SKIP: VCPKG_ROOT not set, mo2-server not built
    goto :cpp_done
)
REM Configure only when the cache is absent.
if not exist "%REPO_ROOT%\build\CMakeCache.txt" (
    echo No CMake cache yet. Configuring - this is a one-time slow step ^(vcpkg + Crow^).
    cmake --preset default
    if errorlevel 1 goto :cpp_configure_failed
)
cmake --build "%REPO_ROOT%\build" --config Release --target mo2-server
if errorlevel 1 goto :cpp_build_failed
set "CPP_BUILT=1"

:cpp_done
echo.

REM ============================================================================
REM STEP 6: Build the web dashboard
REM ============================================================================
echo [6/7] Building the web dashboard...
echo ----------------------------------------------------------------------------
set "WEB_BUILT="
where npm >nul 2>&1
if errorlevel 1 (
    echo SKIP: npm not found in PATH, web dashboard not built
    goto :web_done
)

pushd "%REPO_ROOT%\web"
if not exist "node_modules" (
    echo node_modules missing, running npm install...
    call npm install
    if errorlevel 1 goto :web_install_failed
)
REM npm run build is "tsc" then "vite build"; a type error fails the build.
call npm run build
if errorlevel 1 goto :web_build_failed
popd
set "WEB_BUILT=1"
echo Web dashboard built.

:web_done
echo.

REM ============================================================================
REM STEP 7: Generate documentation
REM ============================================================================
REM Two generators, one site. rustdoc covers the engine (the large majority of
REM src/), doxide+mkdocs covers the C++ server that remains. They do not know
REM about each other; the landing page links across, injected by _clean_docs.py.
REM
REM cargo is a hard requirement by step 1, so a rustdoc lint failure is FATAL
REM here, matching clippy. doxide/mkdocs/python are optional and SKIP, matching
REM steps 4-6. Order is load-bearing: mkdocs clears site/ on every run, so the
REM rustdoc copy has to land after it, not before.
REM ============================================================================
echo [7/7] Generating documentation...
echo ----------------------------------------------------------------------------
set "DOCS_BUILT="
set "RUSTDOC_BUILT="

REM --document-private-items: this is a publish=false engine crate whose module
REM docs exist to explain internals, and they link to private helpers. Without
REM the flag those links break and most of what the docs discuss is unrendered.
cargo doc --no-deps --release --document-private-items
if errorlevel 1 goto :rustdoc_failed
set "RUSTDOC_BUILT=1"

where doxide >nul 2>&1
if errorlevel 1 (
    echo SKIP: doxide not found in PATH, C++ API docs not generated
    goto :docs_done
)
where mkdocs >nul 2>&1
if errorlevel 1 (
    echo SKIP: mkdocs not found in PATH, doc site not generated
    goto :docs_done
)

doxide build
if errorlevel 1 goto :doxide_failed

where python >nul 2>&1
if errorlevel 1 (
    echo SKIP: python not found in PATH, doxide markdown not post-processed
) else (
    python "%REPO_ROOT%\scripts\_clean_docs.py"
    if errorlevel 1 goto :clean_docs_failed
)

mkdocs build
if errorlevel 1 goto :mkdocs_failed
set "DOCS_BUILT=1"

REM mkdocs clears site/ before writing, which deletes the TRACKED site\.gitkeep
REM and leaves it staged as a deletion. Put it back.
if not exist "%REPO_ROOT%\site\.gitkeep" (
    type nul > "%REPO_ROOT%\site\.gitkeep"
)

REM rustdoc output goes in verbatim: it is a self-contained app with its own
REM theme and search index, and rewriting it would break on toolchain updates.
if exist "%REPO_ROOT%\target\doc" (
    xcopy /e /i /q /y "%REPO_ROOT%\target\doc" "%REPO_ROOT%\site\rust" >nul
    if errorlevel 1 (
        echo WARNING: could not copy the rustdoc output into site\rust.
    ) else (
        echo Engine API docs staged at site\rust\mo2_salma_rs\index.html
    )
)

:docs_done
echo.

REM ============================================================================
REM SUMMARY
REM ============================================================================
echo ============================================================================
echo                           BUILD PIPELINE COMPLETE
echo ============================================================================
echo.
echo Build Output:
echo   Engine:     target\package\mo2-salma.dll   ^(renamed for deploy^)
if defined CPP_BUILT (
    echo   Server:     build\bin\Release\mo2-server.exe
) else (
    echo   Server:     not built ^(cmake or VCPKG_ROOT unavailable^)
)
if defined WEB_BUILT (
    echo   Web UI:     web\dist\
) else (
    echo   Web UI:     not built ^(npm unavailable^)
)
if defined DOCS_BUILT (
    echo   Docs:       site\index.html   ^(engine API at site\rust\^)
) else if defined RUSTDOC_BUILT (
    echo   Docs:       target\doc\mo2_salma_rs\   ^(doxide or mkdocs unavailable^)
) else (
    echo   Docs:       not generated
)
echo.
echo  *** Run test.bat to verify, run.bat to run the dashboard, ***
echo  *** deploy.bat to install into MO2                        ***
echo.
echo ============================================================================

endlocal
exit /b 0

REM ============================================================================
REM Web dashboard failure exits. Reached only by GOTO from step 5.
REM ============================================================================
:cpp_configure_failed
echo ERROR: cmake configure failed. Check VCPKG_ROOT and the vcpkg manifest.
endlocal
exit /b 1

:cpp_build_failed
echo ERROR: mo2-server build failed
endlocal
exit /b 1

:web_install_failed
echo ERROR: npm install failed
popd
endlocal
exit /b 1

:web_build_failed
echo ERROR: web dashboard build failed
popd
endlocal
exit /b 1

REM ============================================================================
REM Documentation failure exits. Reached only by GOTO from step 7.
REM ============================================================================
:rustdoc_failed
echo ERROR: cargo doc failed. A rustdoc lint is denied in Cargo.toml
echo        ^([lints.rustdoc]^), so a broken intra-doc link fails the build.
endlocal
exit /b 1

:doxide_failed
echo ERROR: doxide build failed. Check doxide.yml against the @ingroup tags
echo        in src\*.hpp / src\*.cpp.
endlocal
exit /b 1

:clean_docs_failed
echo ERROR: scripts\_clean_docs.py failed
endlocal
exit /b 1

:mkdocs_failed
echo ERROR: mkdocs build failed. Check the nav in mkdocs.yml against the
echo        pages doxide actually generated under docs\.
endlocal
exit /b 1
