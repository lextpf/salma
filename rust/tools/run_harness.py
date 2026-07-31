#!/usr/bin/env python
"""Drive the repo's round-trip harness against the Rust DLL, unmodified.

`test_all.py` / `test_one.py` locate the engine through
`scripts/common.py::find_dll`, whose FIRST candidate is
`$SALMA_DEPLOY_PATH/salma/mo2-salma.dll`. On a developer box that path holds the
DEPLOYED C++ DLL, so a naive run silently validates the wrong binary - the
stale-DLL trap.

This script stages the DLL under test into its own directory as
`mo2-salma.dll`, points `SALMA_DEPLOY_PATH` at that staging root for the child
process only, and then PROVES which binary the harness actually loaded by
re-hashing the file at the path `test_all.py` reports. No repo script is
modified and no environment change escapes the subprocess.

Usage:
  python rust/tools/run_harness.py                      # Rust DLL, full corpus
  python rust/tools/run_harness.py --baseline           # C++ DLL, for comparison
  python rust/tools/run_harness.py --limit 25           # first 25 testable mods
  python rust/tools/run_harness.py --one <archive> <mod>   # test_one.py --full
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

REPO = Path(__file__).resolve().parent.parent.parent
RUST_DLL = REPO / "rust" / "target" / "release" / "mo2_salma_rs.dll"
CPP_DLL = REPO / "build" / "bin" / "Release" / "mo2-salma.dll"
STAGING = REPO / "rust" / "target" / "harness"
DLL_NAME = "mo2-salma.dll"

# test_all.py prefixes every line with a timestamp ("16:40:28  DLL: ..."), so
# this deliberately does NOT anchor at the start of the line.
# The DLL's bit7z path extracts into %TEMP%\salma-bit7z-batch-* and can balloon
# to tens of GB for large texture archives; test_all.py additionally stages every
# reinstall under %TEMP%. Over a 300-mod corpus that fills the system drive, and
# a run aborted mid-extraction leaks the scratch (a 22 GB orphan was observed).
# TEMP/TMP are therefore pinned to a base on the mods drive, which has room, and
# the base is wiped before each run. Same rule as gen_golden.py's --tmp-base:
# never point this under a read-only corpus root.
def _default_tmp_base() -> Path:
    mods = os.environ.get("SALMA_MODS_PATH", "")
    if mods:
        drive = os.path.splitdrive(os.path.abspath(mods))[0]
        if drive:
            return Path(drive + os.sep) / "salma_harness_tmp"
    import tempfile
    return Path(tempfile.gettempdir()) / "salma_harness_tmp"


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
    """Load the staged DLL directly and record a behavioral fingerprint.

    `getApiVersion` cannot tell the two implementations apart (both report
    1.2.0), so the discriminator is the LOG CALLBACK: the C++ `Logger` invokes a
    registered callback on essentially every install/infer step, while the Rust
    port has no logger yet (Task 17) and never calls it. A callback count of 0
    is therefore a positive signature of the Rust DLL, and any non-zero count is
    a positive signature of the C++ DLL.
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
    # A deliberately bogus pair: both engines take their failure path, which the
    # C++ still logs. Nothing is written to disk either way.
    addr = lib.inferFomodSelections(b"harness-probe-absent.7z", b"harness-probe-absent")
    if addr:
        lib.freeResult(addr)
    lib.setLogCallback(cb_type(0))

    return {
        "version": (lib.getApiVersion() or b"").decode("utf-8", "replace"),
        "callback_lines": len(seen),
        "looks_like": "rust (no logger)" if not seen else "c++ (logger active)",
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
    # Persist the harness's own log next to the staging dir, per run, so a
    # baseline run does not clobber the Rust run's test.log.
    src_log = REPO / "test.log"
    if src_log.is_file():
        shutil.copy2(src_log, STAGING / f"test-{label}.log")
    print(f"[harness] finished in {elapsed:.1f}s (exit {proc.returncode})")
    return proc.returncode, out


def verify_loaded(out: str, expected_dll: Path, expected_hash: str,
                  *, expect_dll_line: bool) -> None:
    """Assert the harness loaded the DLL we staged, by path AND by content.

    `test_all.py` logs a `DLL: <path>` line, which pins the exact file the
    harness opened. `test_one.py` logs nothing, so for that mode the proof is
    the staging precedence (our path is find_dll's first candidate and it
    exists) plus the content hash. `expect_dll_line` distinguishes the two so a
    MISSING line is a hard failure where one was due, rather than a silent skip.
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
    ap.add_argument("--baseline", action="store_true",
                    help="Stage the C++ DLL instead of the Rust one")
    ap.add_argument("--dll", type=Path, default=None,
                    help="Explicit DLL to stage (overrides --baseline)")
    ap.add_argument("--limit", type=int, default=0, help="Max mods to test")
    ap.add_argument("--separator", default=None, help="Only mods under this separator")
    ap.add_argument("--no-full", action="store_true",
                    help="Skip the byte-for-byte content compare")
    ap.add_argument("--one", nargs=2, metavar=("ARCHIVE", "MOD"), default=None,
                    help="Run test_one.py --full on one archive/mod pair")
    ap.add_argument("--tmp-base", default="",
                    help="Scratch base for the DLL's TEMP/TMP "
                         "(default: <mods drive>\\salma_harness_tmp)")
    args = ap.parse_args()

    src = args.dll or (CPP_DLL if args.baseline else RUST_DLL)
    label = "baseline" if args.baseline else "rust"
    print(f"[harness] source DLL: {src}")
    print(f"[harness] source sha256: {sha256(src)}")

    staged = stage_dll(src)
    staged_hash = sha256(staged)
    print(f"[harness] staged as: {staged}")

    fp = fingerprint(staged)
    print(f"[harness] fingerprint: getApiVersion={fp['version']!r} "
          f"log-callback-lines={fp['callback_lines']} -> {fp['looks_like']}")

    env = dict(os.environ)
    # find_dll's FIRST candidate is $SALMA_DEPLOY_PATH/salma/mo2-salma.dll, so
    # pointing it here makes the staged copy win over the deployed C++ DLL.
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
        # Always reclaim the scratch, including on Ctrl-C: an aborted run
        # otherwise leaks the DLL's bit7z batch dir, which reaches tens of GB.
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
