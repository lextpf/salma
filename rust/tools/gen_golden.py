#!/usr/bin/env python3
"""Golden vector generator for the salma Rust port.

Generates parity ground-truth fixtures from the CURRENT C++ DLL
(build/bin/Release/mo2-salma.dll). Every later Rust stage (Tasks 3-16) tests
its output against these vectors, so they must be produced by the authoritative
C++ engine and captured byte-for-byte.

Stdlib only. No pip dependencies. 7z-family archives (.7z/.rar/.001) are listed
and extracted by shelling out to 7z.exe ONLY when it is on PATH; .zip is handled
by the stdlib zipfile module. If neither the DLL nor a local 7z can process a
format, that case is skipped rather than committing a half-filled fixture.

Subcommands
-----------
  generate   Run inferFomodSelections over the full local corpus and write one
             fixture per mod under rust/tests/golden/full/ plus a manifest.json.
  candidates Print a diversity table (format, size, entry/file counts, group
             types) for curation. Reads the full/ fixtures produced by generate.
  curate     Materialize one committed case under rust/tests/golden/cases/<name>/
             (expected.json, ModuleConfig.xml, archive_entries.json,
             target_tree.json, case.json).
  selftest   Verify the FNV-1a-64 implementation against known vectors.
  _worker    Internal: run a single inference in an isolated subprocess.

The DLL is ALWAYS loaded from an explicit path (default
build/bin/Release/mo2-salma.dll). It is never located via any deployed-DLL
search: a stale deployed DLL would silently poison the ground truth.
"""

import argparse
import configparser
import ctypes
import datetime
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path


# ---------------------------------------------------------------------------
# Paths and constants
# ---------------------------------------------------------------------------

# rust/tools/gen_golden.py -> parents[2] is the repo root.
REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DLL = REPO_ROOT / "build" / "bin" / "Release" / "mo2-salma.dll"
GOLDEN_DIR = REPO_ROOT / "rust" / "tests" / "golden"
DEFAULT_FULL_DIR = GOLDEN_DIR / "full"
DEFAULT_CASES_DIR = GOLDEN_DIR / "cases"

# ABI major this harness is written against (matches scripts/common.py).
EXPECTED_API_MAJOR = "1"

# FNV-1a-64 parameters, identical to src/Utils.hpp::fnv1a_hash.
FNV_OFFSET_BASIS = 14695981039346656037
FNV_PRIME = 1099511628211
U64_MASK = 0xFFFFFFFFFFFFFFFF

# ctypes log-callback signature, mirror of scripts/mo2-salma.py.
CALLBACK_TYPE = ctypes.CFUNCTYPE(None, ctypes.c_char_p)

# Per-mod inference timeout. The C++ engine enforces a 600s internal deadline,
# so a subprocess that exceeds this is genuinely wedged and is killed.
DEFAULT_TIMEOUT_S = 660

# The DLL's bit7z path extracts contested entries into %TEMP%\salma-bit7z-batch-*
# and can balloon to tens of GB for large texture archives, leaking the scratch
# if it crashes mid-extraction. We redirect the worker's TEMP/TMP to a scratch
# base (default the mods drive, which has room) and rmtree it after every mod so
# nothing accumulates. Override with --tmp-base. Never point this under a
# read-only corpus root.
def _default_tmp_base() -> Path:
    mods = os.environ.get("SALMA_MODS_PATH", "")
    if mods:
        drive = os.path.splitdrive(os.path.abspath(mods))[0]
        if drive:
            return Path(drive + os.sep) / "salma_golden_tmp"
    import tempfile
    return Path(tempfile.gettempdir()) / "salma_golden_tmp"


# ---------------------------------------------------------------------------
# FNV-1a-64 (byte-exact port of src/Utils.hpp::fnv1a_hash)
# ---------------------------------------------------------------------------

def random_token(n: int = 12) -> str:
    """Short random hex token for unique scratch dir names."""
    return os.urandom(n // 2).hex()


def fnv1a_hash(data: bytes) -> int:
    """FNV-1a-64 over a byte buffer. Matches src/Utils.hpp exactly.

    h0 = 0xCBF29CE484222325; h_{i+1} = (h_i XOR b_i) * 0x100000001B3, mod 2^64.
    """
    h = FNV_OFFSET_BASIS
    for b in data:
        h ^= b
        h = (h * FNV_PRIME) & U64_MASK
    return h


def fnv1a_hex(data: bytes) -> str:
    """Lowercase 16-char hex of the FNV-1a-64 hash (no 0x prefix)."""
    return f"{fnv1a_hash(data):016x}"


def fnv1a_file(path: Path) -> str:
    """FNV-1a-64 hex of a file's full byte content."""
    return fnv1a_hex(path.read_bytes())


# ---------------------------------------------------------------------------
# normalize_path (port of src/Utils.cpp::normalize_path)
# ---------------------------------------------------------------------------

def normalize_path(p: str) -> str:
    """Port of mo2core::normalize_path. Used for target-tree keys.

    Lowercase, backslashes to slashes, strip leading ./ and /, strip trailing /,
    collapse repeated slashes, and drop "." / ".." segments.
    """
    out = p.lower().replace("\\", "/")
    while out.startswith("./"):
        out = out[2:]
    while out.startswith("/"):
        out = out[1:]
    while out.endswith("/"):
        out = out[:-1]
    # Collapse consecutive slashes.
    collapsed = []
    for c in out:
        if c == "/" and collapsed and collapsed[-1] == "/":
            continue
        collapsed.append(c)
    out = "".join(collapsed)
    # Drop "." and ".." segments.
    parts = [seg for seg in out.split("/") if seg and seg != "." and seg != ".."]
    return "/".join(parts)


# ---------------------------------------------------------------------------
# DLL loading (explicit path only, mirror of scripts/mo2-salma.py::_configure_dll)
# ---------------------------------------------------------------------------

def load_dll(dll_path: Path):
    """Load and configure the C++ DLL from an explicit path.

    Never uses any deployed-DLL search: a stale deployed DLL would corrupt the
    ground truth. Owned-string returns are declared as c_void_p so the raw heap
    pointer can be copied and released via freeResult.
    """
    dll_path = Path(dll_path).resolve()
    if not dll_path.exists():
        raise FileNotFoundError(f"C++ DLL not found at explicit path: {dll_path}")
    lib = ctypes.CDLL(str(dll_path))

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

    _check_api_version(lib)
    return lib


def _check_api_version(lib) -> None:
    version_bytes = lib.getApiVersion()
    if not version_bytes:
        raise RuntimeError("salma DLL returned empty getApiVersion(); refusing to use.")
    version_str = version_bytes.decode("utf-8")
    major = version_str.split(".", 1)[0]
    if major != EXPECTED_API_MAJOR:
        raise RuntimeError(
            f"salma DLL ABI mismatch: harness expects major {EXPECTED_API_MAJOR}, "
            f"DLL reports {version_str}."
        )


def call_owned_bytes(lib, fn, *args) -> bytes:
    """Invoke an owned-string export and return the RAW bytes (no decode).

    Always frees the underlying pointer via freeResult. Returns b"" on a null
    pointer. Keeping the bytes undecoded is what makes expected.json byte-exact.
    """
    addr = fn(*args)
    if not addr:
        return b""
    try:
        return ctypes.string_at(addr)
    finally:
        lib.freeResult(addr)


def call_owned_string(lib, fn, *args) -> str:
    return call_owned_bytes(lib, fn, *args).decode("utf-8")


# ---------------------------------------------------------------------------
# Mod / archive enumeration (mirror of scripts/common.py + the plugin)
# ---------------------------------------------------------------------------

def get_installation_file(mod_folder: Path) -> str:
    """Read meta.ini [General] installationFile for a mod folder, or ''."""
    meta = mod_folder / "meta.ini"
    if not meta.exists():
        return ""
    config = configparser.ConfigParser(interpolation=None)
    try:
        config.read(str(meta), encoding="utf-8")
    except Exception:
        return ""
    return config.get("General", "installationFile", fallback="")


def resolve_archive(lib, installation_file: str, mod_folder: Path, mods_dir: Path) -> str:
    """Resolve an archive path via the DLL's own resolveModArchive.

    This is the exact resolution the plugin and dashboard use, so the corpus is
    enumerated the same way the real engine would see it.
    """
    if not installation_file:
        return ""
    return call_owned_string(
        lib,
        lib.resolveModArchive,
        installation_file.encode("utf-8"),
        str(mod_folder).encode("utf-8"),
        str(mods_dir).encode("utf-8"),
    )


def enumerate_mods(mods_dir: Path):
    """Yield (mod_folder, installation_file) for every mod dir with an
    installationFile in meta.ini, sorted by folder name."""
    for d in sorted(mods_dir.iterdir(), key=lambda p: p.name.lower()):
        if not d.is_dir():
            continue
        inst = get_installation_file(d)
        if inst:
            yield d, inst


# ---------------------------------------------------------------------------
# Fixture file naming
# ---------------------------------------------------------------------------

def safe_fixture_name(mod_name: str) -> str:
    """A collision-free, filesystem-safe fixture filename stem for a mod name.

    Sanitizes to a readable stem and appends a short hash of the full name so
    two mods that sanitize to the same stem never collide, and the mapping is
    stable across runs (so --resume works)."""
    stem = "".join(c if (c.isalnum() or c in "._- ") else "_" for c in mod_name).strip()
    stem = stem[:120].rstrip(". ")
    if not stem:
        stem = "mod"
    digest = hashlib.sha1(mod_name.encode("utf-8")).hexdigest()[:8]
    return f"{stem}__{digest}"


# ---------------------------------------------------------------------------
# generate
# ---------------------------------------------------------------------------

def _concise_error(returncode: int, stderr: str) -> str:
    """One-line error summary from a failed worker."""
    line = stderr.strip().splitlines()[-1].strip() if stderr.strip() else ""
    if returncode in (0xC0000005, 0xC0000005 - (1 << 32), 3221225477):
        return f"native access violation (0xC0000005) in DLL: {line}".strip()
    if line:
        return line[:200]
    return f"worker exit {returncode}"


def run_worker(dll_path: Path, archive: str, mod_path: str, timeout_s: int,
               tmp_base: Path):
    """Run one inference in an isolated subprocess. Returns
    (status, raw_bytes, elapsed_s, error). status is one of
    'non_empty' | 'empty' | 'error' | 'timeout'.

    The worker's TEMP/TMP are pinned to a fresh per-mod scratch dir under
    tmp_base which is deleted afterwards, so the DLL's bit7z extraction scratch
    cannot accumulate or leak (it leaks on native crash otherwise)."""
    work = tmp_base / f"m_{random_token()}"
    work.mkdir(parents=True, exist_ok=True)
    raw_file = work / "out.raw"
    env = os.environ.copy()
    env["TEMP"] = str(work)
    env["TMP"] = str(work)
    try:
        t0 = time.perf_counter()
        try:
            proc = subprocess.run(
                [sys.executable, str(Path(__file__).resolve()), "_worker",
                 str(dll_path), archive, mod_path, str(raw_file)],
                capture_output=True,
                timeout=timeout_s,
                env=env,
            )
        except subprocess.TimeoutExpired:
            return "timeout", b"", timeout_s, f"exceeded {timeout_s}s"
        elapsed = time.perf_counter() - t0
        if proc.returncode != 0:
            err = _concise_error(
                proc.returncode, proc.stderr.decode("utf-8", "replace"))
            return "error", b"", elapsed, err
        raw = raw_file.read_bytes() if raw_file.exists() else b""
        # The worker prints the DLL-measured elapsed seconds to stdout.
        try:
            elapsed = float(proc.stdout.decode("utf-8", "replace").strip())
        except ValueError:
            pass
        status = "non_empty" if raw else "empty"
        return status, raw, elapsed, ""
    finally:
        shutil.rmtree(work, ignore_errors=True)


def cmd_generate(args) -> int:
    dll_path = Path(args.dll).resolve()
    mods_dir = Path(args.mods).resolve()
    out_dir = Path(args.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    tmp_base = Path(args.tmp_base).resolve() if args.tmp_base else _default_tmp_base()
    tmp_base.mkdir(parents=True, exist_ok=True)

    if not mods_dir.is_dir():
        print(f"ERROR: mods path not found: {mods_dir}", file=sys.stderr)
        return 2

    lib = load_dll(dll_path)
    api_version = lib.getApiVersion().decode("utf-8")
    dll_sha = hashlib.sha256(dll_path.read_bytes()).hexdigest()
    git_rev = _git_head()

    print(f"DLL:      {dll_path}")
    print(f"DLL sha:  {dll_sha}")
    print(f"API ver:  {api_version}")
    print(f"git HEAD: {git_rev}")
    print(f"Mods:     {mods_dir}")
    print(f"Out:      {out_dir}")
    print(f"Tmp base: {tmp_base}  (DLL scratch, wiped per mod)")

    mods = list(enumerate_mods(mods_dir))
    total = len(mods)
    if args.limit > 0:
        mods = mods[: args.limit]
    print(f"Found {total} mods with an installationFile "
          f"({len(mods)} to process this run)\n")

    counts = {"total_with_installation_file": total, "processed": 0,
              "archives_resolved": 0, "archives_unresolved": 0,
              "inferred_non_empty": 0, "inferred_empty": 0,
              "errors": 0, "timeouts": 0}
    fixtures = []
    t_start = time.perf_counter()

    for i, (mod_folder, inst) in enumerate(mods, 1):
        mod_name = mod_folder.name
        fixture_path = out_dir / f"{safe_fixture_name(mod_name)}.json"

        if args.resume and fixture_path.exists():
            try:
                prev = json.loads(fixture_path.read_text(encoding="utf-8"))
            except Exception:
                prev = None
            if isinstance(prev, dict) and prev.get("mod_name") == mod_name:
                _tally(counts, prev.get("status", ""))
                counts["processed"] += 1
                fixtures.append({"mod_name": mod_name,
                                 "fixture_file": fixture_path.name,
                                 "status": prev.get("status", "")})
                print(f"[{i}/{len(mods)}] {mod_name} ... RESUMED "
                      f"({prev.get('status', '')})")
                continue

        archive = resolve_archive(lib, inst, mod_folder, mods_dir)
        if not archive:
            counts["archives_unresolved"] += 1
            print(f"[{i}/{len(mods)}] {mod_name} ... SKIP (archive unresolved)")
            continue
        counts["archives_resolved"] += 1

        try:
            archive_size = Path(archive).stat().st_size
        except OSError:
            archive_size = 0

        status, raw, elapsed, error = run_worker(
            dll_path, archive, str(mod_folder), args.timeout, tmp_base)
        counts["processed"] += 1
        _tally(counts, status)

        text = ""
        out_sha = ""
        if raw:
            out_sha = hashlib.sha256(raw).hexdigest()
            try:
                text = raw.decode("utf-8")
            except UnicodeDecodeError:
                # DLL emits valid UTF-8 JSON; fall back losslessly if not.
                text = raw.decode("utf-8", "surrogateescape")

        fixture = {
            "mod_name": mod_name,
            "archive_path": archive,
            "archive_size": archive_size,
            "elapsed_s": round(elapsed, 3),
            "status": status,
            "output_len": len(raw),
            "output_sha256": out_sha,
            "output_json": text,
            "error": error,
        }
        fixture_path.write_text(
            json.dumps(fixture, ensure_ascii=False, indent=2), encoding="utf-8",
            newline="\n")
        fixtures.append({"mod_name": mod_name,
                         "fixture_file": fixture_path.name, "status": status})

        tag = {"non_empty": "INFERRED", "empty": "EMPTY",
               "error": "ERROR", "timeout": "TIMEOUT"}.get(status, status.upper())
        extra = f" ({len(raw)} bytes, {elapsed:.1f}s)" if status == "non_empty" \
            else f" ({elapsed:.1f}s)"
        if error:
            extra += f" [{error[:80]}]"
        print(f"[{i}/{len(mods)}] {mod_name} ... {tag}{extra}")

    total_time = time.perf_counter() - t_start

    manifest = {
        "generated_at": datetime.datetime.now(datetime.timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%SZ"),
        "cpp_dll_git_rev": git_rev,
        "cpp_dll_path": str(dll_path.relative_to(REPO_ROOT))
        if _is_relative_to(dll_path, REPO_ROOT) else str(dll_path),
        "cpp_dll_sha256": dll_sha,
        "cpp_dll_api_version": api_version,
        "mods_path": str(mods_dir),
        "fnv1a": {"offset_basis": FNV_OFFSET_BASIS, "prime": FNV_PRIME,
                  "bits": 64, "note": "matches src/Utils.hpp::fnv1a_hash"},
        "per_mod_timeout_s": args.timeout,
        "elapsed_total_s": round(total_time, 1),
        "counts": counts,
        "fixtures": sorted(fixtures, key=lambda f: f["mod_name"].lower()),
    }
    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8",
        newline="\n")

    _print_summary(counts, total_time, manifest_path)
    return 0


def _tally(counts, status):
    key = {"non_empty": "inferred_non_empty", "empty": "inferred_empty",
           "error": "errors", "timeout": "timeouts"}.get(status)
    if key:
        counts[key] += 1


def _print_summary(counts, total_time, manifest_path):
    print("\n" + "=" * 64)
    print("GOLDEN GENERATION SUMMARY")
    print("=" * 64)
    rows = [
        ("Mods with installationFile", counts["total_with_installation_file"]),
        ("Processed this run", counts["processed"]),
        ("Archives resolved", counts["archives_resolved"]),
        ("Archives unresolved", counts["archives_unresolved"]),
        ("Inferred (non-empty)", counts["inferred_non_empty"]),
        ("Inferred (empty)", counts["inferred_empty"]),
        ("Errors", counts["errors"]),
        ("Timeouts", counts["timeouts"]),
    ]
    for label, value in rows:
        print(f"  {label:<32} {value}")
    print(f"  {'Total runtime (s)':<32} {total_time:.1f}")
    print(f"\nManifest: {manifest_path}")


# ---------------------------------------------------------------------------
# _worker (internal single-shot inference)
# ---------------------------------------------------------------------------

def cmd_worker(argv) -> int:
    # argv: [dll, archive, mod_path, out_raw_file]
    if len(argv) != 4:
        print("usage: _worker <dll> <archive> <mod_path> <out_raw_file>",
              file=sys.stderr)
        return 2
    dll_path, archive, mod_path, out_raw = argv
    lib = load_dll(Path(dll_path))
    t0 = time.perf_counter()
    raw = call_owned_bytes(
        lib, lib.inferFomodSelections,
        archive.encode("utf-8"), mod_path.encode("utf-8"))
    elapsed = time.perf_counter() - t0
    Path(out_raw).write_bytes(raw)
    sys.stdout.write(f"{elapsed:.3f}")
    return 0


# ---------------------------------------------------------------------------
# Archive helpers (zipfile for .zip, 7z.exe for .7z/.rar/.001)
# ---------------------------------------------------------------------------

def find_7z():
    for cand in ("7z", "7z.exe", r"C:\Program Files\7-Zip\7z.exe",
                 r"C:\Program Files (x86)\7-Zip\7z.exe"):
        path = shutil.which(cand) if not Path(cand).is_absolute() else cand
        if path and Path(path).exists():
            return path
    return None


def archive_format(archive: Path) -> str:
    return archive.suffix.lower().lstrip(".")


def _is_zip(archive: Path) -> bool:
    return archive.suffix.lower() == ".zip"


def list_entries(archive: Path):
    """Return [{'path': original, 'size': int}, ...] for FILE entries.

    Directory entries are excluded, matching the bit7z path of
    ArchiveService::list_entries_with_sizes (the C++ engine skips dirs for
    7z-family archives). zip is read with the stdlib; 7z-family via 7z.exe.
    Raises RuntimeError if the format cannot be processed on this machine.
    """
    if _is_zip(archive):
        import zipfile
        entries = []
        with zipfile.ZipFile(archive) as zf:
            for info in zf.infolist():
                if info.is_dir():
                    continue
                entries.append({"path": info.filename, "size": info.file_size})
        return entries
    seven = find_7z()
    if not seven:
        raise RuntimeError(f"7z.exe not available to list {archive.name}")
    return _list_entries_7z(seven, archive)


def _list_entries_7z(seven: str, archive: Path):
    result = subprocess.run(
        [seven, "l", "-slt", "-sccUTF-8", str(archive)],
        capture_output=True, text=True, encoding="utf-8", errors="replace",
        timeout=300, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"7z list failed for {archive.name}: "
                           f"{result.stderr.strip()[:200]}")
    entries = []
    in_items = False
    cur_path = None
    cur_size = None
    is_dir = False
    for line in result.stdout.splitlines():
        if not in_items:
            # The file list begins after the first "----------" separator.
            if line.strip() == "----------":
                in_items = True
            continue
        if line.startswith("Path = "):
            cur_path = line[len("Path = "):]
            cur_size = None
            is_dir = False
        elif line.startswith("Size = "):
            try:
                cur_size = int(line[len("Size = "):].strip())
            except ValueError:
                cur_size = None
        elif line.startswith("Folder = "):
            is_dir = line[len("Folder = "):].strip() == "+"
        elif line.startswith("Attributes = "):
            if "D" in line[len("Attributes = "):].strip().split():
                is_dir = True
        elif line.strip() == "":
            if cur_path is not None and not is_dir:
                entries.append({"path": cur_path,
                                "size": cur_size if cur_size is not None else 0})
            cur_path = None
            cur_size = None
            is_dir = False
    # Flush a trailing block with no blank line after it.
    if cur_path is not None and not is_dir:
        entries.append({"path": cur_path,
                        "size": cur_size if cur_size is not None else 0})
    return entries


def find_module_config(entries):
    """Return the entry dict for fomod/ModuleConfig.xml, shallowest first.

    Matches the C++ engine's 'preferring the shallowest path' rule."""
    matches = [e for e in entries
               if normalize_path(e["path"]).endswith("fomod/moduleconfig.xml")]
    if not matches:
        return None
    matches.sort(key=lambda e: (normalize_path(e["path"]).count("/"),
                                normalize_path(e["path"])))
    return matches[0]


def extract_entry(archive: Path, entry_path: str) -> bytes:
    """Extract a single entry's bytes. entry_path is the original-cased path as
    returned by list_entries."""
    if _is_zip(archive):
        import zipfile
        with zipfile.ZipFile(archive) as zf:
            return zf.read(entry_path)
    seven = find_7z()
    if not seven:
        raise RuntimeError(f"7z.exe not available to extract from {archive.name}")
    import tempfile
    tmp = Path(tempfile.mkdtemp(prefix="salma_golden_x_"))
    try:
        result = subprocess.run(
            [seven, "x", "-y", f"-o{tmp}", str(archive), entry_path],
            capture_output=True, text=True, encoding="utf-8", errors="replace",
            timeout=300, check=False)
        if result.returncode != 0:
            raise RuntimeError(f"7z extract failed for {entry_path}: "
                               f"{result.stderr.strip()[:200]}")
        # 7z preserves the entry's relative path under tmp.
        target = tmp / entry_path.replace("\\", "/")
        if not target.exists():
            # Fall back to a case-insensitive search under tmp.
            want = normalize_path(entry_path)
            for p in tmp.rglob("*"):
                if p.is_file() and normalize_path(
                        str(p.relative_to(tmp))) == want:
                    target = p
                    break
        return target.read_bytes()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def scan_target_tree(mod_folder: Path):
    """Scan an installed mod folder into a sorted target-tree list.

    Mirrors FomodInferenceService::scan_installed_files: every regular file,
    keyed by normalize_path of its mod-relative path, with size and FNV-1a-64
    content hash. Hashes are computed in Python with the same offset basis and
    prime as src/Utils.cpp."""
    tree = []
    for p in sorted(mod_folder.rglob("*")):
        if not p.is_file():
            continue
        rel = normalize_path(str(p.relative_to(mod_folder)))
        try:
            data = p.read_bytes()
        except OSError:
            continue
        tree.append({"path": rel, "size": len(data), "fnv1a": fnv1a_hex(data)})
    tree.sort(key=lambda e: e["path"])
    return tree


# ---------------------------------------------------------------------------
# candidates
# ---------------------------------------------------------------------------

def _load_fixtures(full_dir: Path):
    fixtures = []
    for p in sorted(full_dir.glob("*.json")):
        if p.name == "manifest.json":
            continue
        try:
            fixtures.append(json.loads(p.read_text(encoding="utf-8")))
        except Exception:
            continue
    return fixtures


def _parse_group_types(module_config_bytes: bytes):
    """Return a set of group selection types found in a ModuleConfig.xml."""
    import xml.etree.ElementTree as ET
    types = set()
    try:
        root = ET.fromstring(module_config_bytes)
    except ET.ParseError:
        return types
    for group in root.iter("group"):
        t = group.get("type")
        if t:
            types.add(t)
    return types


def cmd_candidates(args) -> int:
    full_dir = Path(args.full_dir).resolve()
    mods_dir = Path(args.mods).resolve()
    fixtures = _load_fixtures(full_dir)
    non_empty = [f for f in fixtures if f.get("status") == "non_empty"]
    non_empty.sort(key=lambda f: f.get("archive_size", 0))
    empties = [f for f in fixtures if f.get("status") == "empty"]

    shortlist = non_empty[: args.limit]
    print(f"Loaded {len(fixtures)} fixtures "
          f"({len(non_empty)} non-empty, {len(empties)} empty). "
          f"Inspecting {len(shortlist)} smallest non-empty.\n")
    header = (f"{'fmt':<5} {'arc_KB':>8} {'entries':>7} {'files':>6} "
              f"{'exp_KB':>7} {'steps':>5}  {'group_types':<40} mod")
    print(header)
    print("-" * len(header))

    rows = []
    for f in shortlist:
        archive = Path(f["archive_path"])
        fmt = archive_format(archive)
        try:
            entries = list_entries(archive)
        except Exception as e:
            print(f"{fmt:<5} {'?':>8}  list failed: {str(e)[:50]}  {f['mod_name']}")
            continue
        mc = find_module_config(entries)
        gtypes = set()
        if mc is not None:
            try:
                gtypes = _parse_group_types(extract_entry(archive, mc["path"]))
            except Exception:
                gtypes = set()
        mod_folder = mods_dir / f["mod_name"]
        n_files = sum(1 for p in mod_folder.rglob("*") if p.is_file()) \
            if mod_folder.is_dir() else 0
        try:
            steps = len(json.loads(f["output_json"]).get("steps", []))
        except Exception:
            steps = 0
        exp_kb = f.get("output_len", 0) / 1024
        arc_kb = f.get("archive_size", 0) / 1024
        rows.append((f, fmt, entries, mc, gtypes, n_files))
        print(f"{fmt:<5} {arc_kb:>8.1f} {len(entries):>7} {n_files:>6} "
              f"{exp_kb:>7.1f} {steps:>5}  "
              f"{','.join(sorted(gtypes)):<40} {f['mod_name']}")

    print(f"\nEmpty-result fixtures available: {len(empties)}")
    for f in empties[:20]:
        archive = Path(f["archive_path"])
        has_mc = False
        try:
            has_mc = find_module_config(list_entries(archive)) is not None
        except Exception:
            has_mc = False
        print(f"  {archive_format(archive):<5} has_module_config={has_mc}  "
              f"{f['mod_name']}")
    return 0


# ---------------------------------------------------------------------------
# curate
# ---------------------------------------------------------------------------

def cmd_curate(args) -> int:
    full_dir = Path(args.full_dir).resolve()
    mods_dir = Path(args.mods).resolve()
    cases_dir = Path(args.out).resolve()

    fixture_path = full_dir / f"{safe_fixture_name(args.mod_name)}.json"
    if not fixture_path.exists():
        print(f"ERROR: no fixture for mod {args.mod_name!r} at {fixture_path}",
              file=sys.stderr)
        return 2
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))

    case_name = args.case_name or safe_fixture_name(args.mod_name)
    case_dir = cases_dir / case_name
    case_dir.mkdir(parents=True, exist_ok=True)

    archive = Path(fixture["archive_path"])
    fmt = archive_format(archive)

    # expected.json: the EXACT bytes the DLL returned, verified via sha256.
    expected_bytes = fixture["output_json"].encode("utf-8")
    if fixture.get("output_sha256"):
        got = hashlib.sha256(expected_bytes).hexdigest()
        if got != fixture["output_sha256"]:
            print(f"ERROR: expected.json round-trip mismatch for {args.mod_name} "
                  f"({got} != {fixture['output_sha256']})", file=sys.stderr)
            return 3
    (case_dir / "expected.json").write_bytes(expected_bytes)

    # archive_entries.json
    entries = list_entries(archive)
    entries_sorted = sorted(entries, key=lambda e: e["path"].lower())
    (case_dir / "archive_entries.json").write_text(
        json.dumps(entries_sorted, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8", newline="\n")

    # ModuleConfig.xml
    mc = find_module_config(entries)
    has_module_config = mc is not None
    module_config_path = ""
    if has_module_config:
        mc_bytes = extract_entry(archive, mc["path"])
        (case_dir / "ModuleConfig.xml").write_bytes(mc_bytes)
        module_config_path = mc["path"]

    # target_tree.json
    mod_folder = mods_dir / args.mod_name
    tree = scan_target_tree(mod_folder) if mod_folder.is_dir() else []
    (case_dir / "target_tree.json").write_text(
        json.dumps(tree, ensure_ascii=False, indent=2) + "\n", encoding="utf-8",
        newline="\n")

    # case.json
    group_types = sorted(_parse_group_types(mc_bytes)) if has_module_config else []
    try:
        steps = len(json.loads(fixture["output_json"]).get("steps", [])) \
            if fixture["output_json"] else 0
    except Exception:
        steps = 0
    case_meta = {
        "case_name": case_name,
        "mod_name": args.mod_name,
        "archive_filename": archive.name,
        "archive_format": fmt,
        "archive_size": fixture.get("archive_size", 0),
        "source_archive_path": str(archive),
        "module_config_entry": module_config_path,
        "has_module_config": has_module_config,
        "group_types": group_types,
        "step_count": steps,
        "entry_count": len(entries_sorted),
        "installed_file_count": len(tree),
        "expected_status": fixture.get("status", ""),
        "expected_bytes": len(expected_bytes),
        "expected_sha256": fixture.get("output_sha256", ""),
        "note": args.note or "",
    }
    (case_dir / "case.json").write_text(
        json.dumps(case_meta, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8", newline="\n")

    total = sum(p.stat().st_size for p in case_dir.iterdir() if p.is_file())
    print(f"Curated case {case_name!r} -> {case_dir}")
    for p in sorted(case_dir.iterdir()):
        if p.is_file():
            print(f"  {p.name:<24} {p.stat().st_size:>8} bytes")
    print(f"  {'TOTAL':<24} {total:>8} bytes")
    if not has_module_config:
        print("  NOTE: no fomod/ModuleConfig.xml in archive (empty-result case).")
    return 0


# ---------------------------------------------------------------------------
# selftest
# ---------------------------------------------------------------------------

def cmd_selftest(_args) -> int:
    vectors = {
        b"": "cbf29ce484222325",
        b"a": "af63dc4c8601ec8c",
        b"foobar": "85944171f73967e8",
    }
    ok = True
    for data, expected in vectors.items():
        got = fnv1a_hex(data)
        status = "PASS" if got == expected else "FAIL"
        if got != expected:
            ok = False
        print(f"[{status}] fnv1a({data!r}) = {got} (expected {expected})")
    print("\nAll FNV-1a vectors passed." if ok else "\nFNV-1a self-test FAILED.")
    return 0 if ok else 1


# ---------------------------------------------------------------------------
# misc
# ---------------------------------------------------------------------------

def _git_head() -> str:
    try:
        out = subprocess.run(["git", "rev-parse", "HEAD"], cwd=str(REPO_ROOT),
                             capture_output=True, text=True, check=False)
        return out.stdout.strip() or "unknown"
    except OSError:
        return "unknown"


def _is_relative_to(child: Path, parent: Path) -> bool:
    try:
        child.relative_to(parent)
        return True
    except ValueError:
        return False


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main(argv) -> int:
    # _worker is a positional fast-path invoked as a subprocess.
    if len(argv) >= 2 and argv[1] == "_worker":
        return cmd_worker(argv[2:])

    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)

    mods_default = os.environ.get("SALMA_MODS_PATH", "")

    g = sub.add_parser("generate", help="run the full corpus and write fixtures")
    g.add_argument("--dll", default=str(DEFAULT_DLL))
    g.add_argument("--mods", default=mods_default)
    g.add_argument("--out", default=str(DEFAULT_FULL_DIR))
    g.add_argument("--limit", type=int, default=0, help="process at most N mods")
    g.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT_S,
                   help="per-mod inference timeout in seconds")
    g.add_argument("--tmp-base", default="",
                   help="scratch base for the DLL's TEMP/TMP (default: mods "
                        "drive root/salma_golden_tmp); wiped per mod")
    g.add_argument("--resume", action="store_true",
                   help="skip mods whose fixture already exists")
    g.set_defaults(func=cmd_generate)

    c = sub.add_parser("candidates", help="print a diversity table for curation")
    c.add_argument("--full-dir", default=str(DEFAULT_FULL_DIR))
    c.add_argument("--mods", default=mods_default)
    c.add_argument("--limit", type=int, default=60,
                   help="inspect the N smallest non-empty fixtures")
    c.set_defaults(func=cmd_candidates)

    u = sub.add_parser("curate", help="materialize one committed case")
    u.add_argument("mod_name")
    u.add_argument("--case-name", default="")
    u.add_argument("--note", default="")
    u.add_argument("--full-dir", default=str(DEFAULT_FULL_DIR))
    u.add_argument("--mods", default=mods_default)
    u.add_argument("--out", default=str(DEFAULT_CASES_DIR))
    u.set_defaults(func=cmd_curate)

    s = sub.add_parser("selftest", help="verify the FNV-1a implementation")
    s.set_defaults(func=cmd_selftest)

    args = parser.parse_args(argv[1:])
    if getattr(args, "mods", None) == "" and args.cmd in ("generate", "candidates", "curate"):
        print("ERROR: --mods not set and SALMA_MODS_PATH is empty", file=sys.stderr)
        return 2
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
