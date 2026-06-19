"""
@brief provide DLL and file-tree helpers for test scripts.
@author Alex (https://github.com/lextpf)

### :material-cog-outline: configuration

import exits 2 unless `SALMA_MODS_PATH` and `SALMA_DEPLOY_PATH` are set.
`SALMA_DOWNLOADS_PATH` is optional; without it, only absolute archive paths resolve.

### :material-filter-outline: comparison exclusions

comparison ignores these basenames on both sides:

| name                | reason                              |
|---------------------|-------------------------------------|
| `meta.ini`          | MO2 writes install metadata.        |
| `mo_salma.log`      | salma writes a per-mod log.         |
| `salma-install.log` | salma install log.                  |
| `mujointfix.log`    | a runtime plugin rewrites the file. |
"""

import configparser
import ctypes
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path


def _required_env(name: str) -> Path:
    val = os.environ.get(name)
    if not val:
        raise RuntimeError(
            f"Required env var {name} is not set. "
            f"Run setup.bat once to configure your MO2 paths, "
            f"or set the variable manually for this shell."
        )
    return Path(val)


# fail at import so all commands report missing configuration consistently.
try:
    MODS_PATH = _required_env("SALMA_MODS_PATH")
    DEPLOY_PATH = _required_env("SALMA_DEPLOY_PATH")
except RuntimeError as exc:
    print(f"[setup] {exc}", file=sys.stderr)
    raise SystemExit(2)
DOWNLOADS_PATH_ENV = os.environ.get("SALMA_DOWNLOADS_PATH", "")

IGNORED_FILES = {"meta.ini", "mo_salma.log", "salma-install.log", "mujointfix.log"}

DLL_NAME = "mo2-salma.dll"

# refuse incompatible C ABI majors before calling other exports.
EXPECTED_API_MAJOR = "1"

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

def find_dll() -> Path:
    """
    @fn find_dll() -> Path
    @brief select the first harness DLL from the fixed search order.
    @author Alex (https://github.com/lextpf)

    ### :material-format-list-numbered: lookup order

    search order is deployed DLL, CMake copy, then current-directory DLL.
    manual runs do not verify that the deployed build is current.
    `run_harness.py` redirects `SALMA_DEPLOY_PATH` to a verified staged copy.

    @return the absolute DLL path.
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
    """
    @fn load_dll(dll_path: Path)
    @brief bind the C ABI and reject incompatible DLL versions.
    @author Alex (https://github.com/lextpf)

    each call maps a new handle. owned strings use `c_void_p`; callers must use
    `call_owned_string` or call `freeResult` on the original address.
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

    # callers support DLLs that do not export `resolveModArchive`.
    if hasattr(lib, "resolveModArchive"):
        lib.resolveModArchive.argtypes = [
            ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
        lib.resolveModArchive.restype = ctypes.c_void_p

    _check_api_version(lib)

    return lib


def _check_api_version(lib) -> None:
    """
    @fn _check_api_version(lib) -> None
    @brief reject a missing, empty, or incompatible ABI version.
    @author Alex (https://github.com/lextpf)

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
    """
    @fn call_owned_string(lib, fn, *args) -> str
    @brief copy and release one DLL-owned UTF-8 string.
    @author Alex (https://github.com/lextpf)

    `freeResult` runs even when decoding fails.
    @return the decoded value, or an empty string for a null pointer.
    """
    addr = fn(*args)
    if not addr:
        return ""
    try:
        return ctypes.string_at(addr).decode("utf-8")
    finally:
        lib.freeResult(addr)


def get_archive_path(mod_folder: Path) -> str:
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
    if not raw_path:
        return None
    p = Path(raw_path)
    if p.is_absolute():
        return p if p.exists() else None
    if DOWNLOADS_PATH_ENV:
        candidate = Path(DOWNLOADS_PATH_ENV) / p
        if candidate.exists():
            return candidate
    return None


def find_mod_folder(mod_name: str, mods_dir: Path) -> Path | None:
    """
    @fn find_mod_folder(mod_name: str, mods_dir: Path) -> Path | None
    @brief use an exact folder name before a case-insensitive prefix match.
    @author Alex (https://github.com/lextpf)

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
    """
    @fn parse_separator_mods(separator: str) -> set[str]
    @brief collect the mods shown below one MO2 separator.
    @author Alex (https://github.com/lextpf)

    the first sorted profile containing `modlist.txt` is used. MO2 stores highest
    priority first, which reverses the UI order. collect entries above the target
    and ignore intervening separators without resetting the result.
    """
    profiles_dir = MODS_PATH.parent / "profiles"
    if not profiles_dir.is_dir():
        raise FileNotFoundError(
            f"Profiles directory not found: {profiles_dir}")

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
        name = stripped[1:] if stripped[0] in "+-" else stripped
        if name == target:
            if not collected:
                raise ValueError(
                    f"Separator '{separator}' has no mods above it in {modlist}. "
                    f"It is the first entry, so there is nothing under it in the "
                    f"MO2 UI.")
            return collected
        if name.endswith("_separator"):
            continue
        collected.add(name)

    raise ValueError(f"Separator '{separator}' not found in {modlist}")


def list_files(root: Path) -> dict[str, int]:
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
    for cand in ("7z", "7z.exe", r"C:\Program Files\7-Zip\7z.exe",
                 r"C:\Program Files (x86)\7-Zip\7z.exe"):
        path = shutil.which(cand) if not Path(cand).is_absolute() else cand
        if path and Path(path).exists():
            return path
    return None


def archive_entries_by_tail(archive_path: Path) -> dict[str, list[int]]:
    """
    @fn archive_entries_by_tail(archive_path: Path) -> dict[str, list[int]]
    @brief index archive entry sizes under every relative path suffix.
    @author Alex (https://github.com/lextpf)

    duplicate sizes remain distinct so comparison can detect selectable variants.
    zero-byte entries are excluded. listing blocks for at most 120 seconds.

    @return an empty map when 7-Zip is unavailable, cannot start, or times out.
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
                # index every suffix to ignore the FOMOD source-folder prefix.
                parts = norm.split("/")
                for i in range(len(parts)):
                    tail = "/".join(parts[i:])
                    sizes_by_tail.setdefault(tail, []).append(cur_size)
            cur_path = None
            cur_size = None
    return sizes_by_tail


def compare_trees(expected: Path, actual: Path, full: bool,
                  archive_path: Path | None = None) -> CompareResult:
    # paths are relative, slash-separated, and case-insensitive.
    # archive exclusions require 7-Zip; a failed listing makes comparison strict.
    # | difference         | exclusion rule                                  |
    # |--------------------|-------------------------------------------------|
    # | missing            | no archive entry has the path suffix.           |
    # | size mismatch      | no entry at that suffix has the installed size. |
    # | content mismatch   | at most one entry has that suffix and size.     |
    # the missing rule can hide inference failures. omit `archive_path` for strict mode.
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
        # one matching source cannot produce two contents.
        return sum(1 for s in sizes if s == exp_size) <= 1

    def is_external_missing(rel: str) -> bool:
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
