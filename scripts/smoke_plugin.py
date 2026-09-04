#!/usr/bin/env python
"""
@brief verify a packaged DLL through the MO2 plugin loader.
@author Alex (https://github.com/lextpf)

the command stages copies under `target/plugin-smoke` and does not access the
live MO2 installation. it supplies the host-only import classes.
"""

import argparse
import ctypes
import re
import shutil
import sys
import types
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PLUGIN_SRC = REPO / "scripts" / "mo2-salma.py"
PACKAGED = REPO / "target" / "package" / "mo2-salma.dll"
STAGING = REPO / "target" / "plugin-smoke"

_checks = []


def check(label, ok, detail=""):
    _checks.append(ok)
    print(f"[{'PASS' if ok else 'FAIL'}] {label}" + (f" ({detail})" if detail else ""))


def install_stubs():
    """
    @fn install_stubs()
    @brief provide host-only classes needed to import the plugin.
    @author Alex (https://github.com/lextpf)

    `__getattr__` creates empty classes for MO2 interface bases.
    """

    class _Stub(types.ModuleType):
        def __getattr__(self, name):
            cls = type(name, (), {})
            setattr(self, name, cls)
            return cls

    for name in ("mobase", "PyQt6", "PyQt6.QtCore", "PyQt6.QtWidgets", "PyQt6.QtGui"):
        sys.modules.setdefault(name, _Stub(name))


def main() -> int:
    ap = argparse.ArgumentParser(description="Load the Rust DLL via the MO2 plugin loader")
    ap.add_argument("--dll", type=Path, default=PACKAGED,
                    help=f"DLL to stage (default: {PACKAGED})")
    args = ap.parse_args()

    if not args.dll.is_file():
        raise SystemExit(
            f"[smoke] {args.dll} not found. Run `python scripts/package.py` first."
        )

    # place the DLL first in the plugin's search order.
    shutil.rmtree(STAGING, ignore_errors=True)
    (STAGING / "salma").mkdir(parents=True, exist_ok=True)
    staged_plugin = STAGING / "mo2-salma.py"
    shutil.copy2(PLUGIN_SRC, staged_plugin)
    staged_dll = STAGING / "salma" / "mo2-salma.dll"
    shutil.copy2(args.dll, staged_dll)
    check("plugin copied verbatim",
          staged_plugin.read_bytes() == PLUGIN_SRC.read_bytes())

    install_stubs()

    # the hyphenated file name requires path-based import.
    import importlib.util

    spec = importlib.util.spec_from_file_location("salma_plugin_under_test", staged_plugin)
    plugin = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(plugin)
    except Exception as exc:  # noqa: BLE001 - report, do not mask
        check("plugin module imports standalone", False, repr(exc))
        return 1
    check("plugin module imports standalone", True)

    found = plugin.find_dll()
    check("plugin find_dll() locates the staged DLL",
          found.resolve() == staged_dll.resolve(), str(found))

    # `load_dll` checks required exports and the ABI major.
    lib = plugin.load_dll()
    check("plugin load_dll() configured and version-checked the DLL", lib is not None)

    version = lib.getApiVersion().decode("utf-8")
    check("getApiVersion matches the plugin's expectation",
          version.split(".")[0] == plugin.EXPECTED_API_MAJOR,
          f"{version}, expects major {plugin.EXPECTED_API_MAJOR}")

    # exercise the plugin's owned-string release path.
    out = plugin._call_owned_string(
        lib, lib.inferFomodSelections, b"smoke-absent.7z", b"smoke-absent")
    check("plugin _call_owned_string round-trips inferFomodSelections",
          out == "", f"returned {out!r}")

    check("resolveModArchive is exported (plugin gates on hasattr)",
          hasattr(lib, "resolveModArchive"))
    resolved = plugin._call_owned_string(
        lib, lib.resolveModArchive, b"smoke-absent.7z", str(STAGING).encode(), b"")
    check("resolveModArchive answers through the plugin helper",
          resolved == "", f"returned {resolved!r}")

    check("installSucceeded is False before any install", lib.installSucceeded() is False)

    # logs must resolve from the DLL location, not the process location.
    log = staged_dll.parent / "logs" / "salma.log"
    addr = lib.install(b"smoke-absent.7z", str(STAGING / "out").encode())
    if addr:
        lib.freeResult(addr)
    check("logs/salma.log created beside the DLL", log.is_file(), str(log))
    if log.is_file():
        lines = log.read_text(encoding="utf-8", errors="replace").splitlines()
        first = lines[0] if lines else ""
        # inference runs first, so validate the record shape rather than its tag.
        shape = re.match(
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3} (INFO|WARNING|ERROR) \[\w[\w-]*\] ",
            first)
        check("log line carries the C++ timestamp+level+tag shape",
              bool(shape), first or "<empty>")
        check("install narrative reached the log",
              any(" [install] " in line for line in lines),
              f"{len(lines)} line(s) written")

    print()
    failed = _checks.count(False)
    if failed:
        print(f"{failed} of {len(_checks)} plugin-loader checks FAILED.")
        return 1
    print(f"All {len(_checks)} plugin-loader checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
