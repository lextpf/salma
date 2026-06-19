"""
@brief run one FOMOD inference and install round trip.
@author Alex (https://github.com/lextpf)

the default comparison checks paths and sizes. `--full` also checks bytes. `--dll`
selects the engine; otherwise DLL discovery can select a deployed build.

`SALMA_MODS_PATH` and `SALMA_DEPLOY_PATH` must be set before import.

### :material-check-circle-outline: results and cleanup

exit status 0 means the trees match. status 1 means inference, installation, or comparison failed.
status 2 means a required path was unset during import. the temporary install and configuration are
removed after the run. the installed mod is not modified.
"""

import argparse
import shutil
import sys
import tempfile
from pathlib import Path

from scripts.common import find_dll, load_dll, compare_trees
from scripts.scan import scan
from scripts.install import install_mod


def main():
    parser = argparse.ArgumentParser(
        description="single-mod FOMOD round-trip test")
    parser.add_argument("archive", help="path to mod archive")
    parser.add_argument("mod_path", help="path to installed mod folder")
    parser.add_argument("--full", action="store_true",
                        help="compare file contents byte-for-byte")
    parser.add_argument("--dll", help="path to mo2-salma.dll")
    args = parser.parse_args()

    lib = load_dll(Path(args.dll) if args.dll else find_dll())
    archive = Path(args.archive)
    mod_path = Path(args.mod_path)

    json_str = scan(archive, mod_path, dll=lib)
    if not json_str:
        print("ERROR: inferFomodSelections returned empty", file=sys.stderr)
        sys.exit(1)

    with tempfile.TemporaryDirectory(prefix="salma_test_") as tmp:
        # keep the config outside the install tree so it is not an extra file.
        json_file = Path(tmp + "_config.json")
        json_file.write_text(json_str, encoding="utf-8")

        try:
            install_mod(archive, Path(tmp), json_file, dll=lib)
        finally:
            json_file.unlink(missing_ok=True)

        result = compare_trees(mod_path, Path(tmp), args.full,
                               archive_path=archive)

    print(f"Installed: {result.total_files}  "
          f"Missing: {len(result.missing)}  "
          f"Extra: {len(result.extra)}  "
          f"Size mismatch: {len(result.size_mismatch)}")

    if result.size_mismatch:
        for f in sorted(result.size_mismatch)[:10]:
            print(f"  SIZE: {f}")
    if result.missing:
        for f in sorted(result.missing)[:10]:
            print(f"  MISSING: {f}")
    if result.extra:
        for f in sorted(result.extra)[:10]:
            print(f"  EXTRA: {f}")
    if result.content_mismatch:
        for f in sorted(result.content_mismatch)[:10]:
            print(f"  CONTENT: {f}")

    if result.ok:
        print("PASS!")
    else:
        sys.exit(1)


if __name__ == "__main__":
    main()
