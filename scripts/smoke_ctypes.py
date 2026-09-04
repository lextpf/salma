#!/usr/bin/env python3
"""
@brief verify the flat C ABI used by the MO2 plugin.
@author Alex (https://github.com/lextpf)

keep ctypes declarations synchronized with `scripts/mo2-salma.py`. the command
exits 0 on success, 1 on a failed check, and 2 on invalid usage. it does not
install a mod, but early calls can create `logs/salma.log` beside the DLL.
"""
import ctypes
import sys

# keep the callback and export list synchronized with `scripts/mo2-salma.py`.
CALLBACK_TYPE = ctypes.CFUNCTYPE(None, ctypes.c_char_p)

EXPORTS = (
    "getApiVersion",
    "freeResult",
    "installSucceeded",
    "setLogCallback",
    "install",
    "installWithConfig",
    "inferFomodSelections",
    "resolveModArchive",
)


def _configure_dll(lib):
    """
    @fn _configure_dll(lib)
    @brief match the production ctypes ownership declarations.
    @author Alex (https://github.com/lextpf)

    owned strings use `c_void_p` so `freeResult` receives the original address.
    """
    lib.getApiVersion.argtypes = []
    lib.getApiVersion.restype = ctypes.c_char_p  # static const char*, never freed

    lib.freeResult.argtypes = [ctypes.c_void_p]
    lib.freeResult.restype = None

    lib.installSucceeded.argtypes = []
    lib.installSucceeded.restype = ctypes.c_bool

    lib.setLogCallback.argtypes = [CALLBACK_TYPE]
    lib.setLogCallback.restype = None

    lib.install.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.install.restype = ctypes.c_void_p

    lib.installWithConfig.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.installWithConfig.restype = ctypes.c_void_p

    lib.inferFomodSelections.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.inferFomodSelections.restype = ctypes.c_void_p

    lib.resolveModArchive.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.resolveModArchive.restype = ctypes.c_void_p


def _call_owned_string(lib, fn, *args) -> str:
    """
    @fn _call_owned_string(lib, fn, *args) -> str
    @brief copy and release one DLL-owned string.
    @author Alex (https://github.com/lextpf)

    @return the decoded value, or an empty string for a null pointer.
    """
    addr = fn(*args)
    if not addr:
        return ""
    try:
        return ctypes.string_at(addr).decode("utf-8")
    finally:
        lib.freeResult(addr)


def main(argv):
    if len(argv) != 2:
        print(f"usage: {argv[0]} <path-to-mo2_salma_rs.dll>", file=sys.stderr)
        return 2

    dll_path = argv[1]
    lib = ctypes.CDLL(dll_path)
    _configure_dll(lib)

    failures = []

    def check(name, cond):
        print(f"[{'PASS' if cond else 'FAIL'}] {name}")
        if not cond:
            failures.append(name)

    for name in EXPORTS:
        check(f"export present: {name}", hasattr(lib, name))

    version = lib.getApiVersion()
    version_str = version.decode("utf-8") if version else ""
    check(f'getApiVersion() == "1.2.0" (got "{version_str}")', version_str == "1.2.0")

    infer_null = _call_owned_string(lib, lib.inferFomodSelections, None, None)
    check(
        'inferFomodSelections(None, None) == null message '
        f'(got "{infer_null}")',
        infer_null == "archivePath and modPath must not be null",
    )
    # every inference failure becomes an empty string across the C ABI.
    infer_empty = _call_owned_string(lib, lib.inferFomodSelections, b"", b"")
    check(f'inferFomodSelections(b"", b"") == "" (got "{infer_empty}")', infer_empty == "")

    check("installSucceeded() is False", lib.installSucceeded() is False)

    @CALLBACK_TYPE
    def _on_log(_msg):
        pass

    lib.setLogCallback(_on_log)
    lib.setLogCallback(CALLBACK_TYPE())
    check("setLogCallback register + clear did not crash", True)

    if failures:
        print(f"\n{len(failures)} check(s) FAILED: {failures}", file=sys.stderr)
        return 1
    print("\nAll smoke checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
