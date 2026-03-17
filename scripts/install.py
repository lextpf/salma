"""Install a single mod using a JSON config.

Library function:
    install_mod(archive, output_dir, json_path, dll=None) -> str

CLI:
    python scripts/install.py <archive> <output_dir> <json_path> [--dll PATH]
"""

from pathlib import Path
import sys; sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import argparse

from scripts.common import find_dll, load_dll


def install_mod(archive: Path, output_dir: Path, json_path: Path,
                dll=None) -> str:
    """Call installWithConfig. Returns DLL result string."""
    if dll is None:
        dll = load_dll(find_dll())
    result = dll.installWithConfig(
        str(archive).encode("utf-8"),
        str(output_dir).encode("utf-8"),
        str(json_path).encode("utf-8"),
    )
    return result.decode("utf-8") if result else ""


def main():
    parser = argparse.ArgumentParser(description="Install a mod with config")
    parser.add_argument("archive", help="Path to mod archive")
    parser.add_argument("output_dir", help="Output directory for installed mod")
    parser.add_argument("json_path", help="Path to FOMOD selections JSON")
    parser.add_argument("--dll", help="Path to mo2-salma.dll")
    args = parser.parse_args()

    lib = load_dll(Path(args.dll) if args.dll else find_dll())
    result = install_mod(
        Path(args.archive), Path(args.output_dir),
        Path(args.json_path), dll=lib)

    if result:
        print(result)


if __name__ == "__main__":
    main()
