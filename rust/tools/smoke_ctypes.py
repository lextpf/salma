#!/usr/bin/env python3
"""ctypes smoke test for the Rust mo2_salma_rs.dll.

Loads the DLL from an explicit path and exercises the flat C ABI with the SAME
argtypes/restype declarations as scripts/mo2-salma.py::_configure_dll, then
asserts the Milestone 1 stub contract. Exit code 0 on success, 1 on a failed
check, 2 on a usage error.

Usage:
    python smoke_ctypes.py <path-to-mo2_salma_rs.dll>
"""
import ctypes
import sys

# Mirror of scripts/mo2-salma.py: the log-callback signature.
CALLBACK_TYPE = ctypes.CFUNCTYPE(None, ctypes.c_char_p)

# The exact set of symbols the MO2 plugin needs - no more, no less.
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
    """Mirror of scripts/mo2-salma.py::_configure_dll.

    Owned-string returns are declared as c_void_p (raw heap pointer) so the
    bytes can be copied and the pointer handed to freeResult, exactly as the
    production plugin does to avoid leaking the allocation.
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
    """Mirror of scripts/mo2-salma.py::_call_owned_string.

    Invokes an owned-string export, copies the bytes, and always frees the
    underlying pointer via freeResult - proving the round-trip does not crash
    and does not leak. Returns "" on a null pointer.
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

    # 1. All 8 exports are present (and the plugin needs no others).
    for name in EXPORTS:
        check(f"export present: {name}", hasattr(lib, name))

    # 2. getApiVersion decodes to "1.2.0".
    version = lib.getApiVersion()
    version_str = version.decode("utf-8") if version else ""
    check(f'getApiVersion() == "1.2.0" (got "{version_str}")', version_str == "1.2.0")

    # 3. inferFomodSelections round-trips through freeResult without crashing.
    #    Null inputs -> the null-argument message.
    infer_null = _call_owned_string(lib, lib.inferFomodSelections, None, None)
    check(
        'inferFomodSelections(None, None) == null message '
        f'(got "{infer_null}")',
        infer_null == "archivePath and modPath must not be null",
    )
    #    Empty (non-null) inputs -> "" placeholder.
    infer_empty = _call_owned_string(lib, lib.inferFomodSelections, b"", b"")
    check(f'inferFomodSelections(b"", b"") == "" (got "{infer_empty}")', infer_empty == "")

    # 4. installSucceeded() is False before any successful install.
    check("installSucceeded() is False", lib.installSucceeded() is False)

    # 5. setLogCallback registers and clears without crashing.
    @CALLBACK_TYPE
    def _on_log(_msg):
        pass

    lib.setLogCallback(_on_log)          # register
    lib.setLogCallback(CALLBACK_TYPE())  # clear (null callback)
    check("setLogCallback register + clear did not crash", True)

    if failures:
        print(f"\n{len(failures)} check(s) FAILED: {failures}", file=sys.stderr)
        return 1
    print("\nAll smoke checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
