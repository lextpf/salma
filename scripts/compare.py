"""Compare two directory trees.

CLI:
    python scripts/compare.py <expected_dir> <actual_dir> [--no-full]
"""

from pathlib import Path
import sys; sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import argparse

from scripts.common import compare_trees


def main():
    parser = argparse.ArgumentParser(
        description="Compare two directory trees")
    parser.add_argument("expected_dir", help="Expected (original) directory")
    parser.add_argument("actual_dir", help="Actual (test) directory")
    parser.add_argument(
        "--full",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Compare file contents byte-for-byte (default: enabled)",
    )
    args = parser.parse_args()

    result = compare_trees(
        Path(args.expected_dir), Path(args.actual_dir), args.full)

    print(f"Total files: {result.total_files}")

    if result.ok:
        print("PASS -- directories match")
        sys.exit(0)

    if result.missing:
        print(f"\nMissing ({len(result.missing)}):")
        for f in result.missing[:20]:
            print(f"  {f}")
        if len(result.missing) > 20:
            print(f"  ... and {len(result.missing) - 20} more")

    if result.extra:
        print(f"\nExtra ({len(result.extra)}):")
        for f in result.extra[:20]:
            print(f"  {f}")
        if len(result.extra) > 20:
            print(f"  ... and {len(result.extra) - 20} more")

    if result.size_mismatch:
        print(f"\nSize mismatch ({len(result.size_mismatch)}):")
        for f in result.size_mismatch[:20]:
            print(f"  {f}")
        if len(result.size_mismatch) > 20:
            print(f"  ... and {len(result.size_mismatch) - 20} more")

    if result.content_mismatch:
        print(f"\nContent mismatch ({len(result.content_mismatch)}):")
        for f in result.content_mismatch[:20]:
            print(f"  {f}")
        if len(result.content_mismatch) > 20:
            print(f"  ... and {len(result.content_mismatch) - 20} more")

    sys.exit(1)


if __name__ == "__main__":
    main()
