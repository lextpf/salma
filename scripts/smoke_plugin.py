#!/usr/bin/env python
"""Load the packaged DLL through the MO2 plugin's own loader, unmodified.

`scripts/smoke_ctypes.py` checks the raw ABI surface. This checks the layer
above it: that `scripts/mo2-salma.py`'s real `find_dll` / `load_dll` /
`_configure_dll` / `_check_api_version` accept the DLL, which is what decides
whether MO2 can load it.

The plugin runs verbatim. It is copied, never edited, into a staging tree; the
packaged DLL is placed where the plugin's own search order finds it first
(`<plugin dir>/salma/mo2-salma.dll`); and the module is imported with `mobase`
and `PyQt6` stubbed, because those exist only inside MO2.

Nothing here touches the live MO2 installation. Deploying for real is a separate
step, documented in CUTOVER.md.

Usage:
  python scripts/smoke_plugin.py [--dll PATH]
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
    """Provide the MO2-only imports the plugin performs at module scope.

    `__getattr__` hands back a fresh empty class for any name, so the plugin's
    `class X(mobase.IPluginTool)` definitions evaluate. Only the host is stubbed:
    every line of salma's own code still runs as written.
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

    # Stage: the plugin verbatim, and the DLL where its find_dll looks first.
    shutil.rmtree(STAGING, ignore_errors=True)
    (STAGING / "salma").mkdir(parents=True, exist_ok=True)
    staged_plugin = STAGING / "mo2-salma.py"
    shutil.copy2(PLUGIN_SRC, staged_plugin)
    staged_dll = STAGING / "salma" / "mo2-salma.dll"
    shutil.copy2(args.dll, staged_dll)
    check("plugin copied verbatim",
          staged_plugin.read_bytes() == PLUGIN_SRC.read_bytes())

    install_stubs()

    # Import the staged plugin by path. The hyphen makes it non-importable by
    # name, so go through the loader API directly.
    import importlib.util

    spec = importlib.util.spec_from_file_location("salma_plugin_under_test", staged_plugin)
    plugin = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(plugin)
    except Exception as exc:  # noqa: BLE001 - report, do not mask
        check("plugin module imports standalone", False, repr(exc))
        return 1
    check("plugin module imports standalone", True)

    # The plugin's own search order must find the staged DLL first.
    found = plugin.find_dll()
    check("plugin find_dll() locates the staged DLL",
          found.resolve() == staged_dll.resolve(), str(found))

    # load_dll runs both _configure_dll and _check_api_version, so a missing
    # export or an ABI-major mismatch fails right here.
    lib = plugin.load_dll()
    check("plugin load_dll() configured and version-checked the DLL", lib is not None)

    version = lib.getApiVersion().decode("utf-8")
    check("getApiVersion matches the plugin's expectation",
          version.split(".")[0] == plugin.EXPECTED_API_MAJOR,
          f"{version}, expects major {plugin.EXPECTED_API_MAJOR}")

    # Exercise an owned-string round trip through the plugin's own helper, which
    # is where a freeResult mismatch would surface.
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

    # The logger must write next to the DLL, not next to python.exe.
    log = staged_dll.parent / "logs" / "salma.log"
    addr = lib.install(b"smoke-absent.7z", str(STAGING / "out").encode())
    if addr:
        lib.freeResult(addr)
    check("logs/salma.log created beside the DLL", log.is_file(), str(log))
    if log.is_file():
        lines = log.read_text(encoding="utf-8", errors="replace").splitlines()
        first = lines[0] if lines else ""
        # Assert the shape of the first line rather than a specific subsystem
        # tag. The inferFomodSelections round trip above runs before the install
        # and narrates, so the first line belongs to [infer], not [install].
        shape = re.match(
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3} (INFO|WARNING|ERROR) \[\w[\w-]*\] ",
            first)
        check("log line carries the C++ timestamp+level+tag shape",
              bool(shape), first or "<empty>")
        # The install path must still be represented somewhere in the file.
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
