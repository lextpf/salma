"""
@brief infer FOMOD selections for one installed mod.
@author Alex (https://github.com/lextpf)

an empty result represents every inference failure and exits 1. importing
`scripts.common` requires `SALMA_MODS_PATH` and `SALMA_DEPLOY_PATH`, including
when `--dll` is set.
"""

from pathlib import Path
import sys; sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import argparse
import json

from scripts.common import call_owned_string, find_dll, load_dll


def scan(archive: Path, mod_path: Path, dll=None) -> str:
    """
    @fn scan(archive: Path, mod_path: Path, dll=None) -> str
    @brief preserve the DLL result lifetime while running inference.
    @author Alex (https://github.com/lextpf)

    `call_owned_string` frees the DLL allocation after copying it.
    @return the choices JSON, or an empty string for any failure.
    """
    if dll is None:
        dll = load_dll(find_dll())
    return call_owned_string(
        dll,
        dll.inferFomodSelections,
        str(archive).encode("utf-8"),
        str(mod_path).encode("utf-8"),
    )


def main():
    parser = argparse.ArgumentParser(
        description="Infer FOMOD selections for a mod")
    parser.add_argument("archive", help="Path to mod archive")
    parser.add_argument("mod_path", help="Path to installed mod folder")
    parser.add_argument("--output", "-o", help="Write JSON to file")
    parser.add_argument("--dll", help="Path to mo2-salma.dll")
    args = parser.parse_args()

    lib = load_dll(Path(args.dll) if args.dll else find_dll())
    result = scan(Path(args.archive), Path(args.mod_path), dll=lib)

    if not result:
        print("ERROR: inferFomodSelections returned empty", file=sys.stderr)
        sys.exit(1)

    formatted = json.dumps(json.loads(result), indent=2)
    if args.output:
        Path(args.output).write_text(formatted, encoding="utf-8")
        print(f"Written to {args.output}")
    else:
        print(formatted)


if __name__ == "__main__":
    main()
