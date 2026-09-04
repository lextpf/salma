"""
@brief install one archive from recorded FOMOD selections.
@author Alex (https://github.com/lextpf)

the command exits 0 even when the DLL reports failure. it does not call
`installSucceeded()`. importing `scripts.common` requires `SALMA_MODS_PATH` and
`SALMA_DEPLOY_PATH`, including when `--dll` is set.
"""

from pathlib import Path
import sys; sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import argparse

from scripts.common import call_owned_string, find_dll, load_dll


def install_mod(archive: Path, output_dir: Path, json_path: Path,
                dll=None) -> str:
    """
    @fn install_mod(archive: Path, output_dir: Path, json_path: Path, dll=None) -> str
    @brief return the ambiguous install path or error text from the DLL.
    @author Alex (https://github.com/lextpf)

    install_mod does not call `installSucceeded()`. `call_owned_string` frees
    the DLL allocation after copying it.
    """
    if dll is None:
        dll = load_dll(find_dll())
    return call_owned_string(
        dll,
        dll.installWithConfig,
        str(archive).encode("utf-8"),
        str(output_dir).encode("utf-8"),
        str(json_path).encode("utf-8"),
    )


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
