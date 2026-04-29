"""Shared utilities for the salma test scripts.

Config constants, DLL loading, path helpers, and file-tree comparison.

Import-time precondition: SALMA_MODS_PATH and SALMA_DEPLOY_PATH must both be
set. Importing this module with either unset prints setup guidance and exits the
process with code 2, before any caller's argparse runs. Every importer inherits
that, including the scripts that take an explicit --dll path.
SALMA_DOWNLOADS_PATH is optional; without it only absolute `installationFile`
paths resolve.
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
            f"Run setup.bat once to configure your MO2 paths, "
            f"or set the variable manually for this shell."
        )
    return Path(val)


# Read at import time on purpose: callers (test_all.py, scripts/scan.py,
# scripts/install.py) treat these as module constants. Failing here turns a
# missing variable into one clear setup message and exit 2, instead of a
# confusing failure deep inside a run.
try:
    MODS_PATH = _required_env("SALMA_MODS_PATH")
    DEPLOY_PATH = _required_env("SALMA_DEPLOY_PATH")
except RuntimeError as exc:
    print(f"[setup] {exc}", file=sys.stderr)
    raise SystemExit(2)
DOWNLOADS_PATH_ENV = os.environ.get("SALMA_DOWNLOADS_PATH", "")

# Filenames dropped from both sides of every tree comparison. The match is on
# the basename only and the set is fixed: no pattern, extension or directory
# rule, so any other file rewritten after install must be added here by hand or
# it is reported as a mismatch.
#   meta.ini          - MO2 metadata. MO2 writes it into the mod folder after
#                       the install; no archive ever ships it.
#   mo_salma.log      - the per-mod install log written by
#                       scripts/mo2-salma.py::InstallMods._configure_mod_logger.
#   salma-install.log - a second salma install-log name. No current code path
#                       writes it; the entry is kept so a mod folder that still
#                       holds one does not fail the diff.
#   mujointfix.log    - a log a shipped plugin rewrites (truncates) at runtime,
#                       so it never matches the archive copy byte-for-byte.
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
    """Locate mo2-salma.dll for the test harness. Returns an absolute path.

    Candidates, in order; the first one that exists wins:

      1. ``$SALMA_DEPLOY_PATH/salma/mo2-salma.dll`` - the deployed plugin DLL.
      2. ``build/bin/Release/mo2-salma.dll``, relative to the current working
         directory. CMake copies the packaged DLL there for mo2-server, so this
         is a staged copy of the same engine, not a second engine.
      3. ``<cwd>/mo2-salma.dll``.

    Stale-DLL trap: candidate 1 is the file deployed last, not the one you just
    built, so a harness run started by hand reports a result for code you may
    not have compiled. ``scripts/run_harness.py`` closes that hole: it stages
    the DLL under test, redirects ``SALMA_DEPLOY_PATH`` for the child process
    only, and re-hashes the file the harness reports loading.

    This order is not the plugin's order. ``scripts/mo2-salma.py::find_dll``
    searches ``<plugin dir>/salma``, the working directory, ``<cwd>/dlls/salma``,
    the plugin directory, and then every existing entry of ``%PATH%``. Only
    candidate 1 has a counterpart there, so a run here proves nothing about
    which DLL MO2 would pick.

    Raises FileNotFoundError if no candidate exists.
    """
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

    Also checks the DLL's ABI major and raises RuntimeError on a mismatch or on
    an empty version string. No caching: each call maps the file again, so a
    caller that wants one handle must keep the one it got.

    Owned-string returns (install / installWithConfig / inferFomodSelections /
    resolveModArchive) are declared as ``c_void_p`` so ctypes hands back the raw
    heap pointer. Callers must route each one through ``call_owned_string`` (or
    call ``lib.freeResult(addr)`` themselves) or the allocation leaks. Declaring
    them ``c_char_p`` instead lets ctypes decode automatically and drops the
    pointer: over a test_all.py run of hundreds of mods that is tens of MB
    leaked.
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
    """Refuse to use a DLL whose ABI major does not match the scripts'.

    Raises RuntimeError on a major mismatch, on an empty version string, and on
    a DLL that does not export getApiVersion at all. That last case cannot be
    reached through ``load_dll``: it declares ``lib.getApiVersion.argtypes``
    first, and ctypes raises a bare AttributeError there when the export is
    missing. The handler below only fires for a caller that arrives some other
    way.
    """
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
    """Invoke a DLL function returning a C string the DLL allocated.

    Only ``freeResult`` may release that pointer. Decodes UTF-8 and always calls
    it, including when decoding throws, so the allocation cannot leak. Returns
    "" for a null pointer.
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
    """Mod names grouped under `separator` in the MO2 UI.

    Reads `<SALMA_MODS_PATH>/../profiles/<first profile in sorted order that has
    a modlist.txt>/modlist.txt`, not the profile MO2 currently has active.

    MO2 writes modlist.txt highest priority first, so the file is the UI list
    reversed: a separator's own mods sit above it in the file, and everything
    the UI shows beneath a separator is everything above it in the file.

    A separator used as a section header therefore has no mods of its own. In a
    Nolvus-style list a user's personal groups sit above a CUSTOM marker:

        <top of file, highest priority>
          148 mods
          SKELETON_separator ... PIERCINGS_separator   <- shown under CUSTOM in the UI
          PATCHES_separator
          CUSTOM_separator
          11. ENB & RESHADE ... 0. MASTER FILES        <- the base list
        <end of file, lowest priority>

    So this returns every mod entry above `separator`, across all intervening
    separator groups, which is what "test everything under CUSTOM" means. Do
    not reset the accumulator at each separator: for a header separator that
    yields an empty set, and the run then tests nothing without saying so.
    """
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
    collected: set[str] = set()
    for line in lines:
        stripped = line.strip()
        if not stripped:
            continue
        # Strip the +/- enabled marker to get the entry name.
        name = stripped[1:] if stripped[0] in "+-" else stripped
        if name == target:
            if not collected:
                raise ValueError(
                    f"Separator '{separator}' has no mods above it in {modlist}. "
                    f"It is the first entry, so there is nothing under it in the "
                    f"MO2 UI.")
            return collected
        if name.endswith("_separator"):
            # Intervening group headers are not mods, but their mods do belong
            # under the target, so keep accumulating rather than resetting.
            continue
        collected.add(name)

    raise ValueError(f"Separator '{separator}' not found in {modlist}")


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

    compare_trees uses it to recognise files a user changed outside the archive,
    for example another mod overwriting body meshes after install. The size list
    keeps duplicates, so a caller can tell one archive entry at a given size
    (no variant to choose between, so a content difference must have come from
    outside) from several at that size (a real FOMOD choice, so a difference
    means inference picked wrong).

    Every entry is indexed under each suffix of its own path, so a lookup by any
    relative tail hits regardless of the FOMOD source-folder prefix. Entries of
    size 0 are skipped, which also drops directory entries; a zero-byte file
    therefore never matches and always looks external.

    Runs `7z l -slt` in a subprocess and blocks for up to 120 seconds. Returns
    an empty dict when 7z is not on PATH, when the listing times out, and when
    the process cannot start. An empty dict means external-file detection is
    off, and callers must proceed without filtering.
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

    `expected` is the user's installed mod folder; `actual` is the tree the test
    reinstall produced. Paths are compared lowercased, with `/` separators and
    relative to each root, so case and separator differences never register.
    The basenames in IGNORED_FILES are dropped from both sides. `full` adds a
    byte-for-byte content compare, but only for files whose sizes already match.

    Given `archive_path`, files in `expected` that cannot have come from that
    archive count as externally modified (a body replacer overwrote the FOMOD's
    meshes, a patch mod's files were merged in by hand) and are excluded from
    the report. Three heuristics do this, all keyed on the lowercased path tail
    so the FOMOD source-folder prefix does not matter:

    | Diff class | Suppressed when | Why that is not an inference bug |
    | --- | --- | --- |
    | missing | the tail matches no archive entry at all | the user's file cannot have come from this archive, so it arrived from somewhere else |
    | size_mismatch | the archive has entries at that tail but none at the user's size | the archive ships no variant at that size, so the user's copy was overwritten after install |
    | content_mismatch | at most one archive entry at that tail carries the user's size | one source per destination cannot produce two contents, so the divergence came from outside the archive |

    Precondition: all three rows need 7-Zip on PATH. `archive_entries_by_tail`
    returns an empty dict when `_find_7z` finds nothing, which silently disables
    every row and makes the comparison strict. The same two trees can therefore
    pass on a machine that has 7-Zip and fail on one that does not.

    The `missing` row is the one to watch. It can hide a real inference failure
    by declaring an expected file external. Pass `archive_path=None` (which
    `scripts/compare.py` always does) for an unfiltered, strict comparison.
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
