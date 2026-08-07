"""Infer FOMOD selections for a single mod.

Library function:
    scan(archive, mod_path, dll=None) -> str

CLI:
    python scripts/scan.py <archive> <mod_path> [--output FILE] [--dll PATH]

Prints the JSON re-indented to 2 spaces, or writes it to --output. Exits 1 when
inference returns an empty string, which is the engine's failure contract for
every cause: no FOMOD, unreadable archive, parse error.

Precondition: SALMA_MODS_PATH and SALMA_DEPLOY_PATH must be set even when
--dll is given. Importing scripts.common reads them and exits 2 if either is
missing, before argparse runs.
"""

from pathlib import Path
import sys; sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import argparse
import json

from scripts.common import call_owned_string, find_dll, load_dll


def scan(archive: Path, mod_path: Path, dll=None) -> str:
    """Infer FOMOD selections. Returns the JSON string, or "" on any failure.

    Goes through ``call_owned_string`` so the C-side ``_strdup`` buffer is
    freed. Calling the export directly with ``restype = c_char_p`` leaks the
    result pointer on every call.
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
