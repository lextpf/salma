"""Install a single mod using a JSON config.

Library function:
    install_mod(archive, output_dir, json_path, dll=None) -> str

CLI:
    python scripts/install.py <archive> <output_dir> <json_path> [--dll PATH]

Replays a FOMOD install into `output_dir` using the selections in `json_path`,
and prints whatever the DLL returns. The exit code is always 0, a failed install
included: failure travels in the returned text and in the separate
installSucceeded() flag, and this script inspects neither.

Precondition: SALMA_MODS_PATH and SALMA_DEPLOY_PATH must be set even when
--dll is given. Importing scripts.common reads them and exits 2 if either is
missing, before argparse runs.
"""

from pathlib import Path
import sys; sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import argparse

from scripts.common import call_owned_string, find_dll, load_dll


def install_mod(archive: Path, output_dir: Path, json_path: Path,
                dll=None) -> str:
    """Call installWithConfig. Returns the DLL's result string.

    That string is the install path on success and an error message on failure;
    only ``installSucceeded()`` tells the two apart, and this function does not
    check it.

    Goes through ``call_owned_string`` so the C-side ``_strdup`` buffer is
    freed. Calling the export directly with ``restype = c_char_p`` leaks the
    result pointer on every call.
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
