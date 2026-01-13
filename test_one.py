"""Single-mod round-trip test: scan → install → compare."""

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
        description="Single-mod FOMOD round-trip test")
    parser.add_argument("archive", help="Path to mod archive")
    parser.add_argument("mod_path", help="Path to installed mod folder")
    parser.add_argument("--full", action="store_true",
                        help="Compare file contents byte-for-byte")
    parser.add_argument("--dll", help="Path to mo2-salma.dll")
    args = parser.parse_args()

    lib = load_dll(Path(args.dll) if args.dll else find_dll())
    archive = Path(args.archive)
    mod_path = Path(args.mod_path)

    # Step 1: infer FOMOD selections
    json_str = scan(archive, mod_path, dll=lib)
    if not json_str:
        print("ERROR: inferFomodSelections returned empty", file=sys.stderr)
        sys.exit(1)

    # Step 2: install into temp dir
    with tempfile.TemporaryDirectory(prefix="salma_test_") as tmp:
        # Write JSON config outside install dir to avoid "extra" file
        json_file = Path(tmp + "_config.json")
        json_file.write_text(json_str, encoding="utf-8")

        try:
            install_mod(archive, Path(tmp), json_file, dll=lib)
        finally:
            json_file.unlink(missing_ok=True)

        # Step 3: compare
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
