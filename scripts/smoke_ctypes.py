#!/usr/bin/env python3
"""ctypes smoke test for mo2_salma_rs.dll.

Loads the DLL from an explicit path and exercises the flat C ABI with the same
argtypes and restype declarations as scripts/mo2-salma.py::_configure_dll, then
asserts the eight-export contract: every export is present, the version string
matches, an owned string survives the round trip through freeResult,
installSucceeded is False before any install, and callback registration and
clearing do not crash.

Those are production contracts, not scaffolding around a stub. Keep the
declarations here identical to the plugin's, or the test stops saying anything
about the loader MO2 actually uses.

Exit code 0 on success, 1 on a failed check, 2 on a usage error. Nothing is
installed: every call is given a path that does not exist. The engine still
writes its own `logs/salma.log` next to the DLL, because the checks that call
into it run before any log callback is registered.

Usage:
    python smoke_ctypes.py <path-to-mo2_salma_rs.dll>
"""
import ctypes
import sys

# The log-callback signature, as scripts/mo2-salma.py declares it.
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
    """Declare the ABI exactly as scripts/mo2-salma.py::_configure_dll does.

    Owned-string returns are declared as c_void_p (raw heap pointer) so the
    bytes can be copied and the pointer handed to freeResult, which is what the
    plugin does to avoid leaking the allocation.
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
    """Invoke an owned-string export the way the plugin's helper does.

    Copies the bytes and always frees the underlying pointer through
    freeResult, which proves the round trip neither crashes nor leaks. Returns
    "" on a null pointer.
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
    #    Empty (non-null) inputs -> "". The empty string is the engine's
    #    permanent failure contract, not a placeholder: inferFomodSelections
    #    collapses every failure to "" because no error type can cross the ABI.
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
