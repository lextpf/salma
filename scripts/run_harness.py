#!/usr/bin/env python
"""Drive the repo's round-trip harness against a chosen DLL, unmodified.

`test_all.py` and `test_one.py` locate the engine through
`scripts/common.py::find_dll`, whose first candidate is
`$SALMA_DEPLOY_PATH/salma/mo2-salma.dll`. On a developer box that path holds the
build deployed last, not the one just compiled, so a run started by hand can
silently validate the wrong binary. That is the stale-DLL trap.

This script stages the DLL under test into its own directory as `mo2-salma.dll`,
points `SALMA_DEPLOY_PATH` at that staging root for the child process only, and
then proves which binary the harness loaded by re-hashing the file at the path
`test_all.py` reports. No repo script is modified and no environment change
escapes the subprocess.

Requires SALMA_MODS_PATH: the child harness imports `scripts/common.py`, which
exits 2 without it, and the default tmp base is derived from it. SALMA_DEPLOY_PATH
is supplied here for the child, so an inherited value is ignored. Without
SALMA_DOWNLOADS_PATH every mod whose `installationFile` is relative skips as
"archive not found".

Usage:
  python scripts/run_harness.py                        # release build, every testable mod
  python scripts/run_harness.py --dll <path>           # stage an explicit DLL instead
  python scripts/run_harness.py --limit 25             # first 25 mods that reach inference
  python scripts/run_harness.py --separator NAME       # only mods under that MO2 separator
  python scripts/run_harness.py --no-full              # skip the byte-for-byte compare
  python scripts/run_harness.py --one <archive> <mod>  # test_one.py --full
  python scripts/run_harness.py --tmp-base <dir>       # scratch base for TEMP/TMP

`--tmp-base` (and the default it replaces) is deleted before the run and again
after it. Never point it at a directory holding anything you want to keep.

`--baseline` stages nothing. On its own it explains why there is no second
engine to compare against and exits; with `--dll` it is ignored and the explicit
DLL is staged. See the refusal message in `main`.

Exit code: the child harness's own status when verification passes (1 if any mod
failed). Also 1 for a missing source DLL, for `--baseline` without `--dll`, and
for any verification failure, which runs after the harness finishes and so can
turn a passing run into exit 1.
"""

import argparse
import ctypes
import hashlib
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
RUST_DLL = REPO / "target" / "release" / "mo2_salma_rs.dll"
# The name misleads: this path holds the copy of the packaged engine DLL that
# CMake places beside mo2-server.exe. Nothing reads the constant, and there is
# no second engine for it to point at, so do not wire it up as one.
CPP_DLL = REPO / "build" / "bin" / "Release" / "mo2-salma.dll"
STAGING = REPO / "target" / "harness"
DLL_NAME = "mo2-salma.dll"

# The DLL extracts each archive into %TEMP%\fomod-<8 hex chars> (see
# src/installation_service.rs), which reaches tens of GB for large texture
# archives, and test_all.py stages every reinstall under %TEMP% as well. Over a
# 300-mod corpus that fills the system drive, and a run aborted mid-extraction
# leaks the scratch (a 22 GB orphan was observed). TEMP and TMP are therefore
# pinned to a base on the mods drive, which has room. That base is wiped before
# each run, so never point it at a directory holding anything you want to keep.
def _default_tmp_base() -> Path:
    mods = os.environ.get("SALMA_MODS_PATH", "")
    if mods:
        drive = os.path.splitdrive(os.path.abspath(mods))[0]
        if drive:
            return Path(drive + os.sep) / "salma_harness_tmp"
    import tempfile
    return Path(tempfile.gettempdir()) / "salma_harness_tmp"


# test_all.py prefixes every line with a timestamp ("16:40:28  DLL: ..."), so
# this deliberately does not anchor at the start of the line.
DLL_LINE = re.compile(r"\bDLL:\s*(\S.*?)\s*$", re.MULTILINE)
RESULT_LINE = re.compile(
    r"Tested:\s*(\d+)\s+Passed:\s*(\d+)\s+Failed:\s*(\d+)\s+Skipped:\s*(\d+)"
)
TIME_LINE = re.compile(r"Total time:\s*([\d.]+)s")


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def stage_dll(src: Path) -> Path:
    """Copy `src` into the staging tree as mo2-salma.dll and return that path."""
    if not src.is_file():
        raise SystemExit(f"[harness] DLL not found: {src}")
    dest_dir = STAGING / "salma"
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest = dest_dir / DLL_NAME
    # Remove first: overwriting a DLL that a previous run still has mapped
    # fails with a sharing violation on Windows.
    dest.unlink(missing_ok=True)
    shutil.copy2(src, dest)
    return dest


def fingerprint(dll_path: Path) -> dict:
    """Load the staged DLL directly and record what a cheap ABI probe can see.

    Reports three facts about the exact file staged: the `getApiVersion` string,
    the number of log-callback lines one failing `inferFomodSelections` call
    produced, and a plain-language summary of that count.

    Despite the name this does not identify which build is loaded. Every build
    reports `getApiVersion` 1.2.0 and every build narrates the infer path, so
    neither field separates one from another. The discriminator that does work
    is the archive backend named on the `[archive]` log lines (see CUTOVER.md),
    and this probe cannot reach it: the deliberately bogus argument pair fails
    before any archive is opened.

    What the probe does prove: the staged file loads, the four exports it
    touches resolve, callback registration and clearing do not crash, and one
    owned-string return survives the round trip through `freeResult`.

    Side effects: a callback is registered for the duration of the call and
    replaced with a null callback before returning. While a callback is
    registered the engine's logger routes to it instead of to the log file, so
    the probe writes nothing to disk.
    """
    lib = ctypes.CDLL(str(dll_path))
    lib.getApiVersion.argtypes = []
    lib.getApiVersion.restype = ctypes.c_char_p
    lib.inferFomodSelections.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.inferFomodSelections.restype = ctypes.c_void_p
    lib.freeResult.argtypes = [ctypes.c_void_p]
    lib.freeResult.restype = None
    cb_type = ctypes.CFUNCTYPE(None, ctypes.c_char_p)
    lib.setLogCallback.argtypes = [cb_type]
    lib.setLogCallback.restype = None

    seen = []

    def on_log(msg):
        seen.append(msg)

    cb = cb_type(on_log)
    lib.setLogCallback(cb)
    # A deliberately bogus pair: the engine takes its "archive not found" exit
    # and returns an empty string. Its log lines go to the callback registered
    # above, so the probe writes nothing to disk.
    addr = lib.inferFomodSelections(b"harness-probe-absent.7z", b"harness-probe-absent")
    if addr:
        lib.freeResult(addr)
    lib.setLogCallback(cb_type(0))

    return {
        "version": (lib.getApiVersion() or b"").decode("utf-8", "replace"),
        "callback_lines": len(seen),
        # Text only. It describes the count above and identifies no build:
        # every build logs on this path, so a claim here would be false.
        "looks_like": ("silent on the infer path" if not seen
                       else "logs on the infer path"),
    }


def run_harness(cmd: list[str], env: dict, label: str) -> tuple[int, str]:
    print(f"\n[harness] running: {' '.join(cmd)}", flush=True)
    t0 = time.perf_counter()
    proc = subprocess.run(
        cmd, cwd=str(REPO), env=env, capture_output=True, text=True,
        encoding="utf-8", errors="replace",
    )
    elapsed = time.perf_counter() - t0
    out = (proc.stdout or "") + (proc.stderr or "")
    # Copy the harness's test.log next to the staging dir before the next run
    # truncates it. The name carries the run label, so a rust run and an
    # explicit run coexist; two runs of the same kind do not.
    src_log = REPO / "test.log"
    if src_log.is_file():
        shutil.copy2(src_log, STAGING / f"test-{label}.log")
    print(f"[harness] finished in {elapsed:.1f}s (exit {proc.returncode})")
    return proc.returncode, out


def verify_loaded(out: str, expected_dll: Path, expected_hash: str,
                  *, expect_dll_line: bool) -> None:
    """Assert the harness loaded the DLL we staged, by path and by content.

    `test_all.py` logs a `DLL: <path>` line, which pins the exact file the
    harness opened. `test_one.py` logs nothing, so there the proof is staging
    precedence (our path is find_dll's first candidate and it exists) plus the
    content hash. `expect_dll_line` separates the two cases, so an absent line
    fails hard where one was due instead of being skipped silently.
    """
    # The staged bytes must still be the ones we put there, in both modes.
    actual = sha256(expected_dll)
    if actual != expected_hash:
        raise SystemExit(
            f"[harness] FATAL: the staged DLL changed underneath the run.\n"
            f"  expected sha256 {expected_hash}\n  got      sha256 {actual}"
        )

    m = DLL_LINE.search(out)
    if m:
        loaded = Path(m.group(1))
        if loaded.resolve() != expected_dll.resolve():
            raise SystemExit(
                f"[harness] FATAL: wrong DLL loaded.\n"
                f"  expected: {expected_dll}\n  got:      {loaded}\n"
                f"  The staging override did not win find_dll()."
            )
        print(f"[harness] VERIFIED loaded DLL: {loaded}  (reported by the harness)")
    elif expect_dll_line:
        raise SystemExit("[harness] FATAL: test_all.py never reported a DLL path")
    else:
        # test_one.py does not log the path; find_dll is deterministic given
        # SALMA_DEPLOY_PATH, and candidate 1 is the staged file.
        print(f"[harness] VERIFIED staged DLL: {expected_dll}  "
              f"(find_dll candidate 1; test_one.py logs no path)")
    print(f"[harness] VERIFIED sha256:     {actual}")


def main() -> int:
    ap = argparse.ArgumentParser(description="Run the repo harness against a staged DLL")
    ap.add_argument("--baseline", action="store_true",  # retained so it errors, not silently self-compares
                    help="Removed. Without --dll it explains why the C++ "
                         "oracle comparison is gone and exits")
    ap.add_argument("--dll", type=Path, default=None,
                    help="Explicit DLL to stage (overrides --baseline)")
    ap.add_argument("--limit", type=int, default=0,
                    help="Max mods to carry to the inference stage; forwarded "
                         "to test_all.py --limit")
    ap.add_argument("--separator", default=None, help="Only mods under this separator")
    ap.add_argument("--no-full", action="store_true",
                    help="Skip the byte-for-byte content compare")
    ap.add_argument("--one", nargs=2, metavar=("ARCHIVE", "MOD"), default=None,
                    help="Run test_one.py --full on one archive/mod pair")
    ap.add_argument("--tmp-base", default="",
                    help="Scratch base for the DLL's TEMP/TMP. WIPED before "
                         "and after the run "
                         "(default: <mods drive>\\salma_harness_tmp)")
    args = ap.parse_args()

    if args.baseline and not args.dll:
        # There is no second engine to compare against:
        # build/bin/Release/mo2-salma.dll is where CMake copies this same engine
        # DLL for mo2-server. Honouring --baseline would stage that copy, label
        # it "baseline", and report a flawless self-comparison.
        sys.exit(
            "--baseline is no longer available: the C++ oracle engine was removed.\n"
            "build/bin/Release/mo2-salma.dll is the RUST engine now (copied there\n"
            "for mo2-server), so a baseline run would compare Rust against itself.\n"
            "To compare against the C++ again, check out a commit that still has\n"
            "it and pass that DLL explicitly with --dll.")

    src = args.dll or RUST_DLL
    label = "explicit" if args.dll else "rust"
    print(f"[harness] source DLL: {src}")
    print(f"[harness] source sha256: {sha256(src)}")

    staged = stage_dll(src)
    staged_hash = sha256(staged)
    print(f"[harness] staged as: {staged}")

    fp = fingerprint(staged)
    print(f"[harness] fingerprint: getApiVersion={fp['version']!r} "
          f"log-callback-lines={fp['callback_lines']} -> {fp['looks_like']}")

    env = dict(os.environ)
    # find_dll's first candidate is $SALMA_DEPLOY_PATH/salma/mo2-salma.dll, so
    # pointing it here makes the staged copy win over the deployed DLL.
    # Scoped to the subprocess; the caller's environment is untouched.
    env["SALMA_DEPLOY_PATH"] = str(STAGING)

    tmp_base = Path(args.tmp_base).resolve() if args.tmp_base else _default_tmp_base()
    shutil.rmtree(tmp_base, ignore_errors=True)
    tmp_base.mkdir(parents=True, exist_ok=True)
    env["TEMP"] = str(tmp_base)
    env["TMP"] = str(tmp_base)
    print(f"[harness] tmp base: {tmp_base}  (DLL + reinstall scratch)")

    if args.one:
        cmd = [sys.executable, "test_one.py", args.one[0], args.one[1], "--full"]
    else:
        cmd = [sys.executable, "test_all.py"]
        if args.limit:
            cmd += ["--limit", str(args.limit)]
        if args.separator:
            cmd += ["--separator", args.separator]
        if args.no_full:
            cmd.append("--no-full")

    try:
        rc, out = run_harness(cmd, env, label)
    finally:
        # Always reclaim the scratch, Ctrl-C included: an aborted run otherwise
        # leaks the DLL's %TEMP%\fomod-<hex> extraction dir, which reaches tens
        # of GB.
        shutil.rmtree(tmp_base, ignore_errors=True)
    print(out)
    verify_loaded(out, staged, staged_hash, expect_dll_line=not args.one)

    m = RESULT_LINE.search(out)
    if m:
        tested, passed, failed, skipped = (int(g) for g in m.groups())
        t = TIME_LINE.search(out)
        secs = float(t.group(1)) if t else 0.0
        print(f"\n=== harness summary ({label}) ===")
        print(f"  tested {tested}  passed {passed}  failed {failed}  "
              f"skipped {skipped}  in {secs:.1f}s")
    return rc


if __name__ == "__main__":
    sys.exit(main())
