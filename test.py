"""
FOMOD Round-Trip Test Script

For each mod with an archive in meta.ini, infers FOMOD selections, reinstalls
into a temp folder using installWithConfig, then compares the resulting file
tree against the original installed mod.  A mismatch means the scan or install
has a bug.

Environment variables (same as deploy.bat / purge.bat):
  SALMA_MODS_PATH      - mods directory (default: D:\\Nolvus\\Instance\\MODS\\mods)
  SALMA_DEPLOY_PATH    - MO2 plugins dir (default: D:\\Nolvus\\Instance\\MO2\\plugins)
  SALMA_DOWNLOADS_PATH - downloads dir for resolving relative archive paths

Usage:
  python test.py [--no-full] [--limit N] [--separator NAME]

  --no-full         Skip byte-for-byte content compare (faster, less strict)
  --limit N         Max mods to actually test, skips don't count (0 = all, default: all)
  --separator NAME  Only test mods under the given separator in modlist.txt
"""

import argparse
import logging
import re
import shutil
import sys
import tempfile
import time
from pathlib import Path

from scripts.common import (
    MODS_PATH, DEPLOY_PATH, DOWNLOADS_PATH_ENV,
    find_dll, load_dll,
    get_archive_path, resolve_archive,
    parse_separator_mods, compare_trees,
)
from scripts.scan import scan
from scripts.install import install_mod


# ---------------------------------------------------------------------------
# Logging -- writes to both console and test.log
# ---------------------------------------------------------------------------

LOG_FILE = Path(__file__).with_name("test.log")
SALMA_LOG = Path(__file__).with_name("logs") / "salma.log"
STATUS_DOT_COLUMN = 104
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")
COLOR_RESET = "\x1b[0m"
COLOR_GREEN = "\x1b[32m"

logger = logging.getLogger("salma-test")
logger.setLevel(logging.DEBUG)

_formatter = logging.Formatter("%(asctime)s  %(message)s", datefmt="%H:%M:%S")


class StripAnsiFormatter(logging.Formatter):
    def format(self, record):
        return ANSI_RE.sub("", super().format(record))

_console = logging.StreamHandler(sys.stdout)
_console.setLevel(logging.INFO)
_console.setFormatter(_formatter)
logger.addHandler(_console)

_fileh = logging.FileHandler(str(LOG_FILE), mode="w", encoding="utf-8")
_fileh.setLevel(logging.DEBUG)
_fileh.setFormatter(StripAnsiFormatter(
    "%(asctime)s.%(msecs)03.0f  %(levelname)-5s  %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S"))
logger.addHandler(_fileh)


def log(msg: str):
    logger.info(msg)


def log_debug(msg: str):
    logger.debug(msg)


def log_salma(msg: str):
    """Append a line to logs/salma.log using the same format as the C++ Logger."""
    from datetime import datetime
    now = datetime.now()
    ts = now.strftime("%Y-%m-%d %H:%M:%S") + f".{now.microsecond // 1000:03d}"
    with open(SALMA_LOG, "a", encoding="utf-8") as f:
        f.write(f"{ts} INFO {msg}\n")


def colorize(text: str, color: str | None = None) -> str:
    if color and sys.stdout.isatty():
        return f"{color}{text}{COLOR_RESET}"
    return text


def status_line(label: str, status: str, detail: str = "",
                color: str | None = None) -> str:
    dots = "." * max(4, STATUS_DOT_COLUMN - len(label))
    status_text = colorize(status, color)
    suffix = f" {detail}" if detail else ""
    return f"{label} {dots} {status_text}{suffix}"


def normalize_install_result(value: str) -> str:
    """Make install result log-friendly (plain path/text)."""
    text = value.strip()
    if len(text) >= 2 and text[0] == text[-1] and text[0] in {"'", '"'}:
        text = text[1:-1]
    return text.replace("\\\\", "\\")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="FOMOD round-trip test")
    parser.add_argument(
        "--full",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Compare file contents byte-for-byte (default: enabled)",
    )
    parser.add_argument("--limit", type=int, default=0,
                        help="Max mods to actually test, skips don't count "
                             "(0 = all, default: all)")
    parser.add_argument("--separator", type=str, default=None, metavar="NAME",
                        help="Only test mods under the given separator in "
                             "modlist.txt (e.g. CUSTOM)")
    args = parser.parse_args()

    # Parse separator mods if requested
    separator_mods: set[str] | None = None
    if args.separator:
        separator_mods = parse_separator_mods(args.separator)

    log(f"Log file: {LOG_FILE}")
    log(f"Mods:     {MODS_PATH}")
    log(f"Deploy:   {DEPLOY_PATH}")
    if DOWNLOADS_PATH_ENV:
        log(f"Downloads: {DOWNLOADS_PATH_ENV}")
    if separator_mods is not None:
        log(f"Separator: {len(separator_mods)} mods under {args.separator}")

    # Locate and load DLL
    dll_path = find_dll()
    log(f"DLL: {dll_path}")
    lib = load_dll(dll_path)

    # Enumerate mod folders that have archives in meta.ini
    mod_folders = sorted(
        d for d in MODS_PATH.iterdir()
        if d.is_dir() and get_archive_path(d)
    )

    total = len(mod_folders)
    if not total:
        log("No mod folders with archives found")
        sys.exit(0)

    passed = 0
    failed = 0
    skipped = 0
    tested = 0
    failures = []

    log(f"Found {total} mods with archives\n")

    t_start = time.perf_counter()

    for i, mod_folder in enumerate(mod_folders, 1):
        mod_name = mod_folder.name

        # Filter to separator mods only
        if separator_mods is not None and mod_name not in separator_mods:
            continue

        label = f"[{i}/{total}] {mod_name}"
        log_debug(f"--- {label} ---")

        # Resolve archive
        raw_archive = get_archive_path(mod_folder)
        archive = resolve_archive(raw_archive)
        if archive is None:
            reason = ("no meta.ini entry" if not raw_archive
                      else "archive not found")
            log(status_line(label, "SKIP", f"({reason})"))
            log_debug(f"  installationFile = {raw_archive!r}")
            skipped += 1
            continue

        # Check if we've hit the test limit (skips don't count)
        if args.limit > 0 and tested >= args.limit:
            break

        log_debug(f"  Archive: {archive}")
        log_debug(f"  Archive size: {archive.stat().st_size / (1024*1024):.1f} MB")

        tested += 1

        # Scan -> install -> compare
        tmp = tempfile.mkdtemp(prefix="salma_test_")
        try:
            t0 = time.perf_counter()

            # Step 1: infer FOMOD selections
            log_debug(f"  [scan] Starting FOMOD inference...")
            json_str = scan(archive, mod_folder, dll=lib)
            t_scan = time.perf_counter() - t0
            if not json_str:
                log(status_line(label, "SKIP",
                                "(no FOMOD / scan returned empty)"))
                log_debug(f"  [scan] Returned empty after {t_scan:.2f}s")
                skipped += 1
                continue
            log_debug(f"  [scan] Done in {t_scan:.2f}s "
                      f"({len(json_str)} chars)")
            log_salma(status_line(
                f"[infer] {label}",
                "INFERRED",
                f"({len(json_str)} chars, {t_scan:.1f}s)",
            ))

            # Step 2: write JSON to temp file (outside install dir)
            json_file = Path(tmp + "_config.json")
            json_file.write_text(json_str, encoding="utf-8")
            log_debug(f"  [config] Written to {json_file}")

            # Step 3: install
            t_install_start = time.perf_counter()
            log_debug(f"  [install] Installing to {tmp}...")
            result = install_mod(archive, Path(tmp), json_file, dll=lib)
            t_install = time.perf_counter() - t_install_start
            result_text = normalize_install_result(result)
            log_debug(f"  [install] Done in {t_install:.2f}s: "
                      f"{result_text:.200}")

            # Clean up config file
            json_file.unlink(missing_ok=True)

            # Step 4: compare file trees
            t_cmp_start = time.perf_counter()
            log_debug(f"  [compare] Comparing trees "
                      f"(full={args.full})...")
            diff = compare_trees(mod_folder, Path(tmp), args.full)
            t_cmp = time.perf_counter() - t_cmp_start
            elapsed = time.perf_counter() - t0
            log_debug(f"  [compare] Done in {t_cmp:.2f}s -- "
                      f"{diff.total_files} files")

            if diff.ok:
                log(status_line(
                    label, "PASS",
                    f"({diff.total_files} files, {elapsed:.1f}s)"))
                log_debug(f"  Timing: scan={t_scan:.2f}s "
                          f"install={t_install:.2f}s "
                          f"compare={t_cmp:.2f}s "
                          f"total={elapsed:.2f}s")
                passed += 1
            else:
                log(status_line(label, "FAIL", f"({elapsed:.1f}s)"))
                log_debug(f"  Timing: scan={t_scan:.2f}s "
                          f"install={t_install:.2f}s "
                          f"compare={t_cmp:.2f}s "
                          f"total={elapsed:.2f}s")
                parts = []
                if diff.missing:
                    parts.append(("Missing in test", diff.missing))
                if diff.extra:
                    parts.append(("Extra in test", diff.extra))
                if diff.size_mismatch:
                    parts.append(("Size mismatch", diff.size_mismatch))
                if diff.content_mismatch:
                    parts.append(("Content mismatch", diff.content_mismatch))
                for heading, items in parts:
                    line = f"  {heading}: {', '.join(items[:10])}"
                    log(line)
                    if len(items) > 10:
                        log(f"    ... and {len(items) - 10} more")
                failed += 1
                failures.append(mod_name)
        except Exception as e:
            elapsed = time.perf_counter() - t0
            log(status_line(label, "ERROR", f"({e})"))
            log_debug(f"  Exception: {e!r}")
            failed += 1
            failures.append(mod_name)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    total_time = time.perf_counter() - t_start

    # Summary
    sep = "=" * 60
    log("")
    log(sep)
    log("RESULTS")
    log(sep)
    log(f"Tested: {passed + failed}  Passed: {passed}  "
        f"Failed: {failed}  Skipped: {skipped}")
    log(f"Total time: {total_time:.1f}s")
    if failures:
        log("")
        log("Failed mods:")
        for name in failures:
            log(f"  - {name}")
    log("")
    log(f"Full log: {LOG_FILE}")

    sys.exit(1 if failed > 0 else 0)


if __name__ == "__main__":
    main()
