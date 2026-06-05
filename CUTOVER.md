# Cutover: deploying the engine DLL to MO2

How to put a freshly built `mo2-salma.dll` into a live MO2 install, prove which
build is running, and get back to the previous one. Read
[Before you deploy](#before-you-deploy) first: one open issue should gate the
decision. [Cutover record](#cutover-record) holds the state the engine was
signed off in.

`PARITY-NOTES.md` carries the detailed behavior notes this page summarises.

## Before you deploy

### Check the build

Four things exercise a build before it reaches MO2:

- `cargo test --release` - the engine suite.
- `python scripts/smoke_ctypes.py target/release/mo2_salma_rs.dll` - the raw ABI
  surface: eight exports, the version string, one owned string freed through
  `freeResult`.
- `python scripts/smoke_plugin.py` - the MO2 plugin's own `find_dll` /
  `load_dll` / `_configure_dll` / `_check_api_version`, run verbatim against the
  packaged DLL. 12 checks.
- `python scripts/run_harness.py` - the round-trip harness (`test_all.py`)
  against a live MO2 install. The byte-for-byte content compare is on by default
  (`--no-full` turns it off), and the harness verifies by SHA-256 which DLL
  actually loaded.

### Blocker: memory use on archives that expand enormously

The archive backends buffer whole entries in memory rather than streaming them
to disk. On one corpus mod (`Zaki Tattoos General 8K Addon`, 43 MB compressed,
over 63 GB expanded) this reached 39.4 GB resident and was still climbing when
the run was killed.

There is no memory guard on that path, so the failure mode is the host process
(MO2) dying, not an error message. Ordinary FOMOD mods are unaffected: the risk
is specific to archives whose uncompressed size dwarfs available RAM. Judge it
against your own mod list. See `PARITY-NOTES.md`.

### Logging

The mechanism is fixed and matches what MO2 users already expect: the log file
sits next to the DLL, lines are
`YYYY-MM-DD HH:MM:SS.mmm LEVEL message` with INFO, WARNING and ERROR levels,
rotation is at 10 MiB keeping `salma.log.1` through `.3`, a registered callback
replaces file logging for as long as it stays registered, and a re-entrancy
guard protects the callback path.

Coverage spans all fourteen engine modules, including the `[infer]` 0/9-to-9/9
pipeline narrative and the `[solver]` phase narrative with its progress bar.

`[archive]` lines name the crate that actually ran: `zip`, `sevenz_rust2` or
`unrar`. Keep them honest, because any other library named there would be a
backend this DLL does not link.

That wording is also the reliable way to tell a current DLL from a pre-cutover
one, since `getApiVersion` reports `1.2.0` on both: grep `logs/salma.log` for
`via sevenz_rust2` (current) against `via bit7z` (pre-cutover).

### Other accepted divergences

Non-ASCII paths, XML strictness, `.001` multi-volume archives, and several
odd-looking behaviors reproduced on purpose. All catalogued in
`PARITY-NOTES.md`.

## Deploying

### 1. Build and stage

```powershell
.\build.bat
```

Formats, lints, builds release, and stages `target\package\mo2-salma.dll`,
printing its SHA-256. The rename from `mo2_salma_rs.dll` happens in
`package.py` and only there, so a file under the deploy name has provably been
through that step.

To stage an existing release build without the fmt, clippy and build steps:

```powershell
python scripts\package.py --no-build
```

### 2. Back up the deployed DLL

```powershell
copy "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll" "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll.backup"
```

Do not skip this. That copy is the rollback.

### 3. Deploy

```powershell
.\deploy.bat
```

`deploy.bat` requires `SALMA_DEPLOY_PATH`; run `setup.bat` once if it is unset.
It copies the DLL to `%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll` and the Python
plugin to `%SALMA_DEPLOY_PATH%\mo2-salma.py`.

It takes the DLL from one of two sources, in this order:

1. `target\package\mo2-salma.dll` - the artifact `scripts\package.py` stages.
2. `build\bin\Release\mo2-salma.dll` - the copy CMake places next to
   `mo2-server.exe`, which is source 1 as it stood at the last C++ build. Same
   engine, possibly older.

Deleting the staged artifact does not select a different engine: it falls back
to an older copy of the same DLL, or fails when that copy is absent.

The equivalent direct copy, if you want to leave the Python plugin alone:

```powershell
copy /Y target\package\mo2-salma.dll "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll"
```

A `mo2-salma.dll` at the repository root overrides both sources, and nothing
ignores it: `.gitignore` carries no rule matching that name at any anchoring, so
a DLL staged there shows up as untracked and a 2.5 MB binary can be committed by
accident. Check `git status` before committing after a root-override deploy, or
add an anchored `/mo2-salma.dll` rule to close the hole for that exact path.

### 4. Verify it took

```powershell
python scripts\smoke_plugin.py --dll "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll"
```

Then confirm by hash and by log. `getApiVersion` reports `1.2.0` for every build
of the engine and proves nothing on its own.

- **Hash**: compare the deployed file's SHA-256 against the one `package.py`
  printed.
- **Log**: delete `%SALMA_DEPLOY_PATH%\salma\logs\salma.log`, run an install
  from MO2, and read the new file. The format alone does not identify a build,
  so look at the archive backend named on the `[archive]` lines, as described
  under [Logging](#logging).

### 5. Smoke test in MO2

Install one small FOMOD mod through MO2 and confirm the file tree matches what
you expect. `installSucceeded()` gates the plugin's failure reporting, so a
silent wrong answer there is the thing to watch for.

## Rollback

```powershell
copy /Y "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll.backup" "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll"
```

The backup you took in step 2 is the only rollback this repository can perform.
`CMakeLists.txt` defines `salma-support`, `mo2-server` and `salma_tests`, and no
target that produces an engine DLL, so there is nothing here to rebuild an
older engine from. Going further back means checking out an earlier commit in a
separate working tree and building there.

Restart MO2 after any swap. The plugin caches the loaded DLL handle for the
lifetime of the process (`_dll_cache` in `scripts/mo2-salma.py`), so a file
replaced on disk has no effect until the host restarts. Windows also keeps the
loaded DLL locked, so a deploy over a running MO2 fails outright.

## Cutover record

The table records how each export was signed off when this engine replaced its
predecessor. It is a record, not a suite you can re-run: the fixture corpus and
the comparison gate behind these numbers are both gone.

| Export | State at sign-off |
| --- | --- |
| `getApiVersion`, `freeResult` | parity |
| `inferFomodSelections` | parity over 197 corpus fixtures, no divergence |
| `install`, `installWithConfig` | parity: 16 of 16 identical install trees |
| `resolveModArchive` | parity |
| `installSucceeded` | parity with the predicate the old engine ran, which its own header described inaccurately |
| `setLogCallback` and `logs/salma.log` | parity, except that `[archive]` lines name this build's real backends (see [Logging](#logging)) |

End to end the engine measured about 1.57x slower than the one it replaced
(inference 1.57x, install replay 1.86x) over 52 mods. Correct, just slower. Two
suspected causes are recorded in `PARITY-NOTES.md` and neither has been profiled.

## What this does not cover

- Drift between `mo2-server.exe` and the DLL. The cdylib is the engine for every
  consumer, not only the MO2 plugin: `mo2-server.exe` and `salma_tests.exe` both
  compile `src/SalmaEngine.cpp`, which loads `mo2-salma.dll` at runtime with
  `LoadLibraryW` and binds the same eight C exports the Python plugin uses.
  Neither binary links engine symbols; `mo2-server` links only `salma-support`,
  Crow and nlohmann-json. The server and the DLL are therefore separate files
  that can run different versions, and deploying to MO2 does not touch the
  server's copy. `run.bat` byte-compares the copy beside `mo2-server.exe`
  against `target\package\mo2-salma.dll` and warns when they differ.
- Mods 301-309 of the cutover corpus were never exercised by the round-trip
  harness; the run stops at the archive described under
  [Blocker](#blocker-memory-use-on-archives-that-expand-enormously).
- Nothing in this repository's automation runs the engine inside MO2 itself. The
  plugin-loader path is verified headlessly by `smoke_plugin.py`.
