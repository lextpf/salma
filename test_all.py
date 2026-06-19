"""
@brief run FOMOD round-trip checks for installed MO2 mods.
@author Alex (https://github.com/lextpf)

each check infers a configuration, installs into a temporary directory, and compares the trees.
`compare_trees` owns the exclusions for files that the archive cannot reproduce.

### :material-cog-outline: configuration and logs

`SALMA_MODS_PATH` and `SALMA_DEPLOY_PATH` must be set before import. use
`scripts/run_harness.py` to force the current DLL. each run replaces `test.log`. a failed install
can leave its temporary configuration in `%TEMP%`.

### :material-check-circle-outline: results and cleanup

exit status 0 means all checks passed. status 1 means a check or the run failed. status 2 means a
required path was unset during import. each check removes its install directory and does not modify
`SALMA_MODS_PATH`.
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
    # the local path can differ from the engine log when the DLL is elsewhere.
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
    text = value.strip()
    if len(text) >= 2 and text[0] == text[-1] and text[0] in {"'", '"'}:
        text = text[1:-1]
    return text.replace("\\\\", "\\")


def main():
    parser = argparse.ArgumentParser(description="FOMOD round-trip test")
    parser.add_argument(
        "--full",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="compare file contents byte-for-byte (default: enabled)",
    )
    parser.add_argument("--limit", type=int, default=0,
                        help="stop after N mods reach the inference stage; "
                             "mods skipped for a missing archive don't count, "
                             "mods skipped for empty inference do "
                             "(0 = all, default: all)")
    parser.add_argument("--separator", type=str, default=None, metavar="NAME",
                        help="only test mods under the given separator in "
                             "modlist.txt (e.g. CUSTOM)")
    args = parser.parse_args()

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
        for m in sorted(separator_mods)[:10]:
            log_debug(f"  separator mod: {m!r}")

    dll_path = find_dll()
    log(f"DLL: {dll_path}")
    lib = load_dll(dll_path)

    mod_folders = sorted(
        d for d in MODS_PATH.iterdir()
        if d.is_dir() and get_archive_path(d)
    )

    if separator_mods is not None:
        mod_folders = [d for d in mod_folders if d.name in separator_mods]

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

        label = f"[{i}/{total}] {mod_name}"
        log_debug(f"--- {label} ---")

        raw_archive = get_archive_path(mod_folder)
        archive = resolve_archive(raw_archive)
        if archive is None:
            reason = ("no meta.ini entry" if not raw_archive
                      else "archive not found")
            log(status_line(label, "SKIP", f"({reason})"))
            log_debug(f"  installationFile = {raw_archive!r}")
            skipped += 1
            continue

        if args.limit > 0 and tested >= args.limit:
            break

        log_debug(f"  Archive: {archive}")
        log_debug(f"  Archive size: {archive.stat().st_size / (1024*1024):.1f} MB")

        tested += 1

        tmp = tempfile.mkdtemp(prefix="salma_test_")
        try:
            t0 = time.perf_counter()

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
            log(status_line(
                f"[infer] {label}",
                "INFERRED",
                f"({len(json_str)} chars, {t_scan:.1f}s)",
            ))

            # keep the config outside the install tree so it is not an extra file.
            json_file = Path(tmp + "_config.json")
            json_file.write_text(json_str, encoding="utf-8")
            log_debug(f"  [config] Written to {json_file}")

            t_install_start = time.perf_counter()
            log_debug(f"  [install] Installing to {tmp}...")
            result = install_mod(archive, Path(tmp), json_file, dll=lib)
            t_install = time.perf_counter() - t_install_start
            result_text = normalize_install_result(result)
            log_debug(f"  [install] Done in {t_install:.2f}s: "
                      f"{result_text:.200}")

            json_file.unlink(missing_ok=True)

            t_cmp_start = time.perf_counter()
            log_debug(f"  [compare] Comparing trees "
                      f"(full={args.full})...")
            diff = compare_trees(mod_folder, Path(tmp), args.full,
                                 archive_path=archive)
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
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        logger.exception("Fatal error")
        sys.exit(1)
