"""Shared utilities for salma test scripts.

Config constants, DLL loading, path helpers, and file-tree comparison.
"""

import configparser
import ctypes
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

def _required_env(name: str) -> Path:
    """Read a required env var as a Path; raise with setup guidance if unset."""
    val = os.environ.get(name)
    if not val:
        raise RuntimeError(
            f"Required env var {name} is not set. "
            f"Run scripts\\setup-env.bat once to configure your MO2 paths, "
            f"or set the variable manually for this shell."
        )
    return Path(val)


# Reading the env vars at import time is intentional: callers (test_all.py,
# scripts/scan.py, scripts/install.py) pass these around as constants.
# To keep error messages helpful when somebody runs the harness without
# having configured the environment, we let _required_env raise the
# RuntimeError at module-import time. The CLI prints it and exits cleanly.
try:
    MODS_PATH = _required_env("SALMA_MODS_PATH")
    DEPLOY_PATH = _required_env("SALMA_DEPLOY_PATH")
except RuntimeError as exc:
    print(f"[setup] {exc}", file=sys.stderr)
    raise SystemExit(2)
DOWNLOADS_PATH_ENV = os.environ.get("SALMA_DOWNLOADS_PATH", "")

IGNORED_FILES = {"meta.ini", "mo_salma.log", "salma-install.log", "mujointfix.log"}

DLL_NAME = "mo2-salma.dll"

# Major version of the DLL ABI these scripts are written against. Bumped only
# on incompatible C-API changes. A mismatch refuses to use the DLL rather
# than risk silent ABI drift between a stale test harness and a newer build.
EXPECTED_API_MAJOR = "1"


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
    """Load mo2-salma.dll and declare argtypes for all C API functions.

    Owned-string returns (install / installWithConfig / inferFomodSelections)
    are declared as ``c_void_p`` so we get the raw heap pointer back from
    ctypes; callers must pass each return value through ``call_owned_string``
    (or remember to call ``lib.freeResult(addr)``) so the ``_strdup`` buffer
    is not leaked. Across the integration suite (test_all.py runs hundreds of
    mods) the previous c_char_p declaration leaked tens of MB per run.
    """
    lib = ctypes.CDLL(str(dll_path))

    lib.getApiVersion.argtypes = []
    lib.getApiVersion.restype = ctypes.c_char_p  # static const char*, never freed

    lib.freeResult.argtypes = [ctypes.c_void_p]
    lib.freeResult.restype = None

    lib.installSucceeded.argtypes = []
    lib.installSucceeded.restype = ctypes.c_bool

    lib.setLogCallback.argtypes = [ctypes.CFUNCTYPE(None, ctypes.c_char_p)]
    lib.setLogCallback.restype = None

    lib.install.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.install.restype = ctypes.c_void_p

    lib.installWithConfig.argtypes = [
        ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.installWithConfig.restype = ctypes.c_void_p

    lib.inferFomodSelections.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.inferFomodSelections.restype = ctypes.c_void_p

    # resolveModArchive was added in salma DLL 1.1.0. Configure it only when
    # the DLL exports it so test scripts can still load older builds.
    if hasattr(lib, "resolveModArchive"):
        lib.resolveModArchive.argtypes = [
            ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
        lib.resolveModArchive.restype = ctypes.c_void_p

    _check_api_version(lib)

    return lib


def _check_api_version(lib) -> None:
    """Refuse to use a DLL whose ABI major does not match the scripts'."""
    try:
        version_bytes = lib.getApiVersion()
    except AttributeError as exc:
        raise RuntimeError(
            "salma DLL is missing getApiVersion(); refusing to use to avoid ABI "
            "drift. Update to a newer DLL build that ships alongside these scripts."
        ) from exc
    if not version_bytes:
        raise RuntimeError("salma DLL returned empty getApiVersion(); refusing to use.")
    version_str = version_bytes.decode("utf-8")
    major = version_str.split(".", 1)[0]
    if major != EXPECTED_API_MAJOR:
        raise RuntimeError(
            f"salma DLL ABI mismatch: scripts expect major {EXPECTED_API_MAJOR}, "
            f"DLL reports {version_str}. Update the scripts or DLL to match."
        )


def call_owned_string(lib, fn, *args) -> str:
    """Invoke a DLL function returning an owned C string (`_strdup` result).

    Decodes UTF-8 and always calls ``freeResult`` on the underlying pointer
    -- including if decoding throws -- so the heap allocation is not leaked.
    Returns "" if the function returned a null pointer.
    """
    addr = fn(*args)
    if not addr:
        return ""
    try:
        return ctypes.string_at(addr).decode("utf-8")
    finally:
        lib.freeResult(addr)


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


def _find_7z() -> str | None:
    """Locate the 7z command-line tool, or None if unavailable."""
    for cand in ("7z", "7z.exe", r"C:\Program Files\7-Zip\7z.exe",
                 r"C:\Program Files (x86)\7-Zip\7z.exe"):
        path = shutil.which(cand) if not Path(cand).is_absolute() else cand
        if path and Path(path).exists():
            return path
    return None


def archive_entries_by_tail(archive_path: Path) -> dict[str, list[int]]:
    """Build a {lowercased_path_tail -> [sizes]} index of archive entries.

    Used by compare_trees to recognise user files that have been modified
    externally (e.g. by another mod overwriting CBBE/SoS body NIFs after
    install). The list of sizes preserves duplicates so callers can tell
    "1 archive entry at this size" (no contested variant - user must be
    external if content differs) from "multiple archive entries at this
    size" (genuine FOMOD-disambiguable variant - a real failure if inference
    picked the wrong one).

    Returns an empty dict if 7z isn't on PATH; callers should treat that
    as "external-file detection disabled" and proceed without filtering.
    """
    seven_zip = _find_7z()
    if not seven_zip:
        return {}

    try:
        result = subprocess.run(
            [seven_zip, "l", "-slt", str(archive_path)],
            capture_output=True, text=True, timeout=120, check=False,
        )
    except (subprocess.TimeoutExpired, OSError):
        return {}

    sizes_by_tail: dict[str, list[int]] = {}
    cur_path: str | None = None
    cur_size: int | None = None
    for line in result.stdout.splitlines():
        if line.startswith("Path = "):
            cur_path = line[len("Path = "):].strip()
            cur_size = None
        elif line.startswith("Size = "):
            try:
                cur_size = int(line[len("Size = "):].strip())
            except ValueError:
                cur_size = None
        elif not line.strip():
            if cur_path and cur_size is not None and cur_size > 0:
                norm = cur_path.replace("\\", "/").lower()
                # Index by every relative-tail suffix of the entry, so
                # comparisons don't depend on the FOMOD source-folder prefix.
                parts = norm.split("/")
                for i in range(len(parts)):
                    tail = "/".join(parts[i:])
                    sizes_by_tail.setdefault(tail, []).append(cur_size)
            cur_path = None
            cur_size = None
    return sizes_by_tail


def compare_trees(expected: Path, actual: Path, full: bool,
                  archive_path: Path | None = None) -> CompareResult:
    """Compare two file trees and return a CompareResult.

    If `archive_path` is provided, files in `expected` (the user's installed
    mod folder) that cannot have come from this archive are treated as
    externally modified (e.g. a body replacer overwrote the FOMOD's NIFs)
    and excluded from mismatch reporting. Two heuristics:

    - Size mismatch: skipped if the user's size doesn't appear in the
      archive's size-set for that path tail (e.g. user's 1.4MB malebody_0.nif
      vs the Schlongs archive's only 186KB version).
    - Content mismatch: skipped if exactly one archive entry exists at this
      path tail with the user's size. A FOMOD with one source per dest cannot
      produce two different contents, so a content divergence at a uniquely-
      sized dest must be an external override.
    """
    exp_files = list_files(expected)
    act_files = list_files(actual)

    exp_set = set(exp_files)
    act_set = set(act_files)

    archive_entries: dict[str, list[int]] = {}
    if archive_path is not None:
        archive_entries = archive_entry_sizes_by_tail = archive_entries_by_tail(
            archive_path)

    def is_external_size(rel: str, exp_size: int) -> bool:
        if not archive_entries:
            return False
        sizes = archive_entries.get(rel)
        return bool(sizes) and exp_size not in sizes

    def is_external_content(rel: str, exp_size: int) -> bool:
        if not archive_entries:
            return False
        sizes = archive_entries.get(rel)
        if not sizes:
            return False
        # Count archive entries that match the user's size at this tail.
        # If there's exactly one, the FOMOD has no variant to disambiguate;
        # any content difference must be from an external mod.
        return sum(1 for s in sizes if s == exp_size) <= 1

    def is_external_missing(rel: str) -> bool:
        # User has the file but test reinstall doesn't; if no archive entry
        # could produce this path tail, the user's file is external (e.g.
        # Heel Sound's user folder has 35 sound files manually merged from a
        # walk-patch mod that aren't in the FOMOD archive).
        if not archive_entries:
            return False
        return rel not in archive_entries

    raw_missing = sorted(exp_set - act_set)
    if archive_entries:
        missing = [rel for rel in raw_missing if not is_external_missing(rel)]
    else:
        missing = raw_missing
    extra = sorted(act_set - exp_set)
    common = exp_set & act_set

    size_mismatch = []
    content_mismatch = []
    for rel in sorted(common):
        if exp_files[rel] != act_files[rel]:
            if is_external_size(rel, exp_files[rel]):
                continue
            size_mismatch.append(rel)
        elif full:
            exp_actual = find_actual_file(expected, rel)
            act_actual = find_actual_file(actual, rel)
            if (exp_actual and act_actual
                    and not files_equal(exp_actual, act_actual)):
                if is_external_content(rel, exp_files[rel]):
                    continue
                content_mismatch.append(rel)

    return CompareResult(
        missing=missing,
        extra=extra,
        size_mismatch=size_mismatch,
        content_mismatch=content_mismatch,
        total_files=len(exp_files),
    )
