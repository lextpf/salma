# Cutover: swapping the C++ `mo2-salma.dll` for the Rust build

How to deploy the Rust engine in place of the C++ one, how to verify it took,
and how to get back. Read the [Status](#status) and
[Before you cut over](#before-you-cut-over) sections first: there is one open
issue that should gate the decision.

Detailed parity findings live in [PARITY-NOTES.md](PARITY-NOTES.md), organised
by task.

## Status

The Rust DLL (`mo2_salma_rs.dll`) exports the same eight C ABI symbols as the
C++ `mo2-salma.dll`, and all eight are backed by real engine code.

| Area | State |
| --- | --- |
| `getApiVersion`, `freeResult` | parity |
| `inferFomodSelections` | parity: 197 corpus fixtures, 0 DIVERGE |
| `install`, `installWithConfig` | parity: 16/16 identical install trees vs the C++ DLL |
| `resolveModArchive` | parity |
| `installSucceeded` | parity (reproduces the CODE predicate, not the header's) |
| `setLogCallback` + `logs/salma.log` | parity; `[archive]` lines name this build's real backends, see below |

Evidence, all reproducible from this repo:

- `cargo test --release` - 594 tests, 0 failures.
- `python tools/compare_infer.py target/release/mo2_salma_rs.dll --curated`
  - 1 EXACT / 15 METRICS_EQUAL / 0 DIVERGE. Full corpus: 197 fixtures, 0 DIVERGE.
- `python tools/run_harness.py` - the repo's own `test_all.py` over corpus
  mods 1-300: 52 passed, 0 failed, and **zero per-mod disagreements** against
  the C++ baseline. `--one` runs `test_one.py --full` (byte-for-byte) and passes
  on one mod per archive format.
- `python tools/smoke_ctypes.py target/release/mo2_salma_rs.dll` - raw
  ABI surface.
- `python tools/smoke_plugin.py` - the MO2 plugin's OWN `find_dll` /
  `load_dll` / `_configure_dll` / `_check_api_version`, run verbatim against the
  packaged DLL. 12 checks.

## Before you cut over

### Blocker: memory use on archives that expand enormously

The C++ streams contested archive entries to disk through bit7z; the Rust
backends buffer whole entries in memory. On a real corpus mod
(`Zaki Tattoos General 8K Addon`, 43 MB compressed, >63 GB expanded) this was
measured at **39.4 GB resident and still climbing** before the run was killed.

The C++ degrades into a disk-space failure that its own `disk_full_encountered`
guard converts into a clean `Install aborted` error. The Rust has **no
equivalent guard for memory** and would take the host process (MO2) down with
it. See PARITY-NOTES "Task 16".

Judge this against your own corpus. Ordinary FOMOD mods are unaffected; the
risk is specific to archives whose uncompressed size dwarfs available RAM.

### Slower

End to end the port is **~1.57x slower** than the C++ (inference 1.57x, install
replay 1.86x) over 52 tested mods. Correct, just slower. Two known causes are
recorded in PARITY-NOTES (Tasks 9 and 11) and neither has been profiled.

### Logging: complete, with `[archive]` lines naming different libraries

The logging MECHANISM is at parity: same file location (next to the DLL), same
`YYYY-MM-DD HH:MM:SS.mmm LEVEL message` format, same INFO/WARNING/ERROR tags,
same 10 MiB rotation keeping `salma.log.1`-`.3`, same callback routing (a
registered callback REPLACES file logging), same re-entrancy guard.

Message coverage is complete across all fourteen engine modules, including the
full `[infer]` 0/9-to-9/9 pipeline narrative and the `[solver]` phase narrative
with its tqdm-style progress bar. MO2's log window shows what it always did.

One deliberate difference is user-visible. The C++ `[archive]` lines name their
backends ("via bit7z", "falling back to libarchive"); this build links neither,
so those lines name the crate that actually ran instead - `zip`, `sevenz_rust2`
or `unrar`. The line shapes and tags are unchanged. Emitting the C++ strings
verbatim would have made the DLL report libraries it does not contain. Lines
describing machinery this port has no equivalent for (the `7z.dll` discovery
narrative, libarchive's per-entry write warnings, the bit7z-to-libarchive
fallback) are simply not emitted.

That substitution is also the most reliable way to tell WHICH engine a deployed
DLL is, since `getApiVersion` reports `1.2.0` on both: grep `logs/salma.log` for
`via sevenz_rust2` (Rust) versus `via bit7z` (C++).

The full exception list - sites with no counterpart, and sites whose trailing
text differs because the C++ interpolates an exception's `what()` - is in
PARITY-NOTES "Task 17".

### Other accepted divergences

Non-ASCII paths, XML strictness, `.001` multi-volume archives, and several
reproduced C++ bugs. All catalogued in PARITY-NOTES under "Task 15 - accepted
divergences" and the per-task divergence sections.

## Cutover

### 1. Build and stage

```powershell
.\build.bat
```

Formats, lints, builds release, and stages `target\package\mo2-salma.dll`,
printing its SHA-256. The rename from `mo2_salma_rs.dll` happens in
`package.py` and only there: during the parity phase the two names are kept
distinct so a stray copy can never be mistaken for the C++ build.

To stage without the fmt/clippy/build steps (an existing release build):

```powershell
python tools\package.py --no-build
```

### 2. Back up the deployed C++ DLL

```powershell
copy "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll" "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll.cpp-backup"
```

Do not skip this. It is the rollback.

### 3. Deploy

```powershell
.\deploy.bat
```

`deploy.bat` now prefers `target\package\mo2-salma.dll` over the C++
`build\bin\Release\mo2-salma.dll`, so a plain run ships the Rust engine and also
refreshes `scripts\mo2-salma.py` in the plugins directory. It prints a cutover
warning before copying.

The equivalent direct copy, if you want to leave the Python plugin alone:

```powershell
copy /Y target\package\mo2-salma.dll "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll"
```

**To deploy the C++ build instead**, delete or rename the staged Rust artifact
first, since it takes precedence:

```powershell
del target\package\mo2-salma.dll
.\deploy.bat
```

A `mo2-salma.dll` at the REPO ROOT still overrides both sources. That path is
git-ignored by an anchored `/mo2-salma.dll` rule so a 2.5 MB binary cannot be
committed by accident, but only that exact path is covered: a copy under any
other name or directory is still visible to `git add`.

### 4. Verify it took

```powershell
python tools\smoke_plugin.py --dll "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll"
```

Then confirm by hash and by log, which is what actually distinguishes the two
builds - `getApiVersion` reports `1.2.0` on BOTH and proves nothing on its own:

- **Hash**: compare the deployed file's SHA-256 against the one `package.py`
  printed.
- **Log**: delete `%SALMA_DEPLOY_PATH%\salma\logs\salma.log`, run an install
  from MO2, and check the new file. Both engines write the same format, so read
  the CONTENT: the Rust build currently emits the install skeleton
  (`=== Starting mod installation ===`, `Archive:`, `Extracting archive...`,
  `Moving unfomod files...`) but none of the `[fomod]`-tagged per-plugin lines
  the C++ emits. Their absence is the Rust build; their presence is the C++.

### 5. Smoke test in MO2

Install one small FOMOD mod through MO2 and confirm the file tree matches what
you expect. `installSucceeded()` gates the plugin's failure reporting, so a
silent wrong answer there is the thing to watch for.

## Rollback

```powershell
copy /Y "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll.cpp-backup" "%SALMA_DEPLOY_PATH%\salma\mo2-salma.dll"
```

Or rebuild and redeploy the C++ side, which is untouched by this branch:

```powershell
cmake --build build --config Release --target mo2-core
.\deploy.bat
```

Restart MO2 either way: the plugin caches the loaded DLL handle for the
process's lifetime (`_dll_cache` in `scripts/mo2-salma.py`), so a swap on disk
does not take effect until the host restarts.

## What is NOT covered

- `mo2-server.exe` and `salma_tests.exe` link the C++ `mo2core` symbols
  directly. The Rust cdylib exports only the 8 C entry points, not the 88
  mangled C++ ones, so it is a drop-in for the **MO2 Python plugin only**. The
  server and the GoogleTest binary still need the C++ build.
- Corpus mods 301-309 were not exercised by the round-trip harness; both
  engines stop at the archive described under [Blocker](#blocker-memory-use-on-archives-that-expand-enormously).
- The port has not been run inside MO2 itself in this repo's automation; the
  plugin-loader path is verified headlessly by `smoke_plugin.py`.
