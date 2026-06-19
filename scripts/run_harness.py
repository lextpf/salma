#!/usr/bin/env python
"""
@brief run the round-trip harness against a verified staged DLL.
@author Alex (https://github.com/lextpf)

### :material-cog-outline: child environment

the child receives a staging-only `SALMA_DEPLOY_PATH`. the harness-reported path
and SHA-256 verify the loaded file. other environment changes stay in the child.
`SALMA_MODS_PATH` is required. relative archive paths also require
`SALMA_DOWNLOADS_PATH`.

### :material-alert-circle-outline: scratch lifetime

the scratch directory is deleted before and after each run. never use a directory
that contains data to keep. verification failures exit 1 even after a passing run.
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
# this server copy is the same engine and cannot serve as a baseline.
CPP_DLL = REPO / "build" / "bin" / "Release" / "mo2-salma.dll"
STAGING = REPO / "target" / "harness"
DLL_NAME = "mo2-salma.dll"

# keep large extraction scratch on the mods drive when possible.
def _default_tmp_base() -> Path:
    mods = os.environ.get("SALMA_MODS_PATH", "")
    if mods:
        drive = os.path.splitdrive(os.path.abspath(mods))[0]
        if drive:
            return Path(drive + os.sep) / "salma_harness_tmp"
    import tempfile
    return Path(tempfile.gettempdir()) / "salma_harness_tmp"


# harness output prefixes the DLL record with a timestamp.
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
    """
    @fn stage_dll(src: Path) -> Path
    @brief stage the selected binary under the plugin DLL name.
    @author Alex (https://github.com/lextpf)

    @return the staged path.
    """
    if not src.is_file():
        raise SystemExit(f"[harness] DLL not found: {src}")
    dest_dir = STAGING / "salma"
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest = dest_dir / DLL_NAME
    # remove first because Windows cannot overwrite a mapped DLL.
    dest.unlink(missing_ok=True)
    shutil.copy2(src, dest)
    return dest


def fingerprint(dll_path: Path) -> dict:
    """
    @fn fingerprint(dll_path: Path) -> dict
    @brief probe loading, callbacks, and owned-string release.
    @author Alex (https://github.com/lextpf)

    the API version and log count do not identify build bytes; SHA-256 does.
    the callback is cleared before return, and the probe writes no log file.
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
    # use a missing archive to reach the empty-result path without disk output.
    addr = lib.inferFomodSelections(b"harness-probe-absent.7z", b"harness-probe-absent")
    if addr:
        lib.freeResult(addr)
    lib.setLogCallback(cb_type(0))

    return {
        "version": (lib.getApiVersion() or b"").decode("utf-8", "replace"),
        "callback_lines": len(seen),
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
    # preserve the log before another run truncates it.
    src_log = REPO / "test.log"
    if src_log.is_file():
        shutil.copy2(src_log, STAGING / f"test-{label}.log")
    print(f"[harness] finished in {elapsed:.1f}s (exit {proc.returncode})")
    return proc.returncode, out


def verify_loaded(out: str, expected_dll: Path, expected_hash: str,
                  *, expect_dll_line: bool) -> None:
    # `test_all.py` reports its path. `test_one.py` relies on search precedence.
    # both modes verify the staged content hash.
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
        # `test_one.py` does not report the loaded path.
        print(f"[harness] VERIFIED staged DLL: {expected_dll}  "
              f"(find_dll candidate 1; test_one.py logs no path)")
    print(f"[harness] VERIFIED sha256:     {actual}")


def main() -> int:
    ap = argparse.ArgumentParser(description="Run the repo harness against a staged DLL")
    ap.add_argument("--baseline", action="store_true",  # reject self-comparison explicitly
                    help="Requires --dll to supply an independent binary")
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
        # both candidate paths contain the same engine.
        sys.exit(
            "--baseline requires an independent binary supplied with --dll.\n"
            "The CMake and release paths contain the same engine, so using them\n"
            "would compare the binary with itself.")

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
    # the child search order selects this staged copy first.
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
        # reclaim large extraction scratch after failure or Ctrl-C.
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
