"""Shared utilities for salma test scripts.

Config constants, DLL loading, path helpers, and file-tree comparison.
"""

import configparser
import ctypes
import os
from dataclasses import dataclass, field
from pathlib import Path


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

MODS_PATH = Path(os.environ.get(
    "SALMA_MODS_PATH", r"D:\Nolvus\Instance\MODS\mods"))
DEPLOY_PATH = Path(os.environ.get(
    "SALMA_DEPLOY_PATH", r"D:\Nolvus\Instance\MO2\plugins"))
DOWNLOADS_PATH_ENV = os.environ.get("SALMA_DOWNLOADS_PATH", "")

IGNORED_FILES = {"meta.ini", "mo_salma.log", "salma-install.log"}

DLL_NAME = "mo2-salma.dll"


# ---------------------------------------------------------------------------
# CompareResult
# ---------------------------------------------------------------------------

@dataclass
class CompareResult:
    missing: list[str] = field(default_factory=list)
    extra: list[str] = field(default_factory=list)
    size_mismatch: list[str] = field(default_factory=list)
    content_mismatch: list[str] = field(default_factory=list)
    total_files: int = 0

    @property
    def ok(self) -> bool:
        return not (self.missing or self.extra
                    or self.size_mismatch or self.content_mismatch)


# ---------------------------------------------------------------------------
# DLL helpers
# ---------------------------------------------------------------------------

def find_dll() -> Path:
    """Locate mo2-salma.dll using the same search order as the plugin."""
    candidates = [
        DEPLOY_PATH / "salma" / DLL_NAME,
        Path("build/bin/Release") / DLL_NAME,
        Path.cwd() / DLL_NAME,
    ]
    for p in candidates:
        try:
            if p.exists():
                return p.resolve()
        except OSError:
            continue
    raise FileNotFoundError(
        f"Could not find {DLL_NAME} in: "
        + ", ".join(str(c.parent) for c in candidates))


def load_dll(dll_path: Path):
    """Load mo2-salma.dll and declare argtypes for all C API functions."""
    lib = ctypes.CDLL(str(dll_path))

    lib.setLogCallback.argtypes = [ctypes.CFUNCTYPE(None, ctypes.c_char_p)]
    lib.setLogCallback.restype = None

    lib.install.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.install.restype = ctypes.c_char_p

    lib.installWithConfig.argtypes = [
        ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.installWithConfig.restype = ctypes.c_char_p

    lib.inferFomodSelections.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.inferFomodSelections.restype = ctypes.c_char_p

    return lib


# ---------------------------------------------------------------------------
# Mod / archive helpers
# ---------------------------------------------------------------------------

def get_archive_path(mod_folder: Path) -> str:
    """Read meta.ini to get the installationFile for this mod."""
    meta = mod_folder / "meta.ini"
    if not meta.exists():
        return ""
    config = configparser.ConfigParser()
    try:
        config.read(str(meta), encoding="utf-8")
    except Exception:
        return ""
    return config.get("General", "installationFile", fallback="")


def resolve_archive(raw_path: str) -> Path | None:
    """Turn installationFile value into an absolute Path, or None."""
    if not raw_path:
        return None
    p = Path(raw_path)
    if p.is_absolute():
        return p if p.exists() else None
    # Try relative to downloads dir
    if DOWNLOADS_PATH_ENV:
        candidate = Path(DOWNLOADS_PATH_ENV) / p
        if candidate.exists():
            return candidate
    return None


def find_mod_folder(mod_name: str, mods_dir: Path) -> Path | None:
    """Find the installed mod folder matching a name.

    Exact match first, then check if any folder name starts with mod_name.
    """
    exact = mods_dir / mod_name
    if exact.is_dir():
        return exact
    lower = mod_name.lower()
    for d in sorted(mods_dir.iterdir()):
        if d.is_dir() and d.name.lower().startswith(lower):
            return d
    return None


def parse_separator_mods(separator: str) -> set[str]:
    """Parse modlist.txt to find mod names under the given separator."""
    profiles_dir = MODS_PATH.parent / "profiles"
    if not profiles_dir.is_dir():
        raise FileNotFoundError(
            f"Profiles directory not found: {profiles_dir}")

    # Find first profile that has a modlist.txt
    modlist = None
    for profile in sorted(profiles_dir.iterdir()):
        candidate = profile / "modlist.txt"
        if candidate.is_file():
            modlist = candidate
            break
    if modlist is None:
        raise FileNotFoundError(
            f"No modlist.txt found in any profile under {profiles_dir}")

    lines = modlist.read_text(encoding="utf-8").splitlines()

    target = f"{separator}_separator"
    current_section: set[str] = set()
    found = False
    for line in lines:
        stripped = line.strip()
        if not stripped:
            continue
        # Strip +/- prefix to get the entry name
        name = stripped[1:] if stripped[0] in "+-" else stripped
        if name.endswith("_separator"):
            if name == target:
                found = True
                break
            current_section = set()
            continue
        current_section.add(name)

    if not found:
        raise ValueError(f"Separator '{separator}' not found in {modlist}")
    return current_section


# ---------------------------------------------------------------------------
# File-tree comparison
# ---------------------------------------------------------------------------

def list_files(root: Path) -> dict[str, int]:
    """Return {relative_path_lower: size} for all files under root."""
    files = {}
    for f in root.rglob("*"):
        if not f.is_file():
            continue
        rel = f.relative_to(root).as_posix().lower()
        if rel.split("/")[-1] in IGNORED_FILES:
            continue
        files[rel] = f.stat().st_size
    return files


def files_equal(a: Path, b: Path) -> bool:
    """Byte-for-byte comparison of two files."""
    CHUNK = 1 << 16
    with open(a, "rb") as fa, open(b, "rb") as fb:
        while True:
            ca = fa.read(CHUNK)
            cb = fb.read(CHUNK)
            if ca != cb:
                return False
            if not ca:
                return True


def find_actual_file(root: Path, rel_lower: str) -> Path | None:
    """Given a lowercased relative path, find the real file on disk."""
    parts = rel_lower.split("/")
    current = root
    for part in parts:
        found = None
        try:
            for child in current.iterdir():
                if child.name.lower() == part:
                    found = child
                    break
        except OSError:
            return None
        if found is None:
            return None
        current = found
    return current if current.is_file() else None


def compare_trees(expected: Path, actual: Path, full: bool) -> CompareResult:
    """Compare two file trees and return a CompareResult."""
    exp_files = list_files(expected)
    act_files = list_files(actual)

    exp_set = set(exp_files)
    act_set = set(act_files)

    missing = sorted(exp_set - act_set)
    extra = sorted(act_set - exp_set)
    common = exp_set & act_set

    size_mismatch = []
    content_mismatch = []
    for rel in sorted(common):
        if exp_files[rel] != act_files[rel]:
            size_mismatch.append(rel)
        elif full:
            exp_actual = find_actual_file(expected, rel)
            act_actual = find_actual_file(actual, rel)
            if (exp_actual and act_actual
                    and not files_equal(exp_actual, act_actual)):
                content_mismatch.append(rel)

    return CompareResult(
        missing=missing,
        extra=extra,
        size_mismatch=size_mismatch,
        content_mismatch=content_mismatch,
        total_files=len(exp_files),
    )
