#!/usr/bin/env python
"""
@brief build and stage the engine DLL under the MO2 plugin name.
@author Alex (https://github.com/lextpf)

the command writes only to the artifact directory and never deploys. the
`mo2-salma.dll` name confirms that the raw Cargo artifact passed through this
step. use the printed SHA-256 to identify exact build bytes because
`getApiVersion` is identical across builds.

the selected destination file is removed without backup before copy. Windows
cannot overwrite a DLL that another process has mapped.
"""

import argparse
import hashlib
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CARGO_TOML = REPO / "Cargo.toml"
BUILT_DLL = REPO / "target" / "release" / "mo2_salma_rs.dll"
DEFAULT_OUT = REPO / "target" / "package"
DEPLOY_NAME = "mo2-salma.dll"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def build() -> None:
    print("[package] cargo build --release")
    result = subprocess.run(
        ["cargo", "build", "--release", "--manifest-path", str(CARGO_TOML)],
        cwd=str(REPO),
    )
    if result.returncode != 0:
        raise SystemExit(f"[package] cargo build failed ({result.returncode})")


def main() -> int:
    ap = argparse.ArgumentParser(description="Stage the Rust salma DLL for deployment")
    ap.add_argument("--no-build", action="store_true",
                    help="Stage the existing target/release build without rebuilding")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT,
                    help=f"Artifact directory (default: {DEFAULT_OUT})")
    ap.add_argument("--keep-rust-name", action="store_true",
                    help="Stage as mo2_salma_rs.dll instead of the deploy name")
    args = ap.parse_args()

    if not args.no_build:
        build()

    if not BUILT_DLL.is_file():
        raise SystemExit(
            f"[package] {BUILT_DLL} not found. Run without --no-build, or "
            f"`cargo build --release` first."
        )

    name = BUILT_DLL.name if args.keep_rust_name else DEPLOY_NAME
    out_dir: Path = args.out
    out_dir.mkdir(parents=True, exist_ok=True)
    dest = out_dir / name
    # remove first because Windows cannot overwrite a mapped DLL.
    dest.unlink(missing_ok=True)
    shutil.copy2(BUILT_DLL, dest)

    digest = sha256(dest)
    size = dest.stat().st_size
    print(f"[package] source : {BUILT_DLL}")
    print(f"[package] staged : {dest}")
    print(f"[package] size   : {size:,} bytes")
    print(f"[package] sha256 : {digest}")
    if not args.keep_rust_name:
        print(f"[package] NOTE   : renamed {BUILT_DLL.name} -> {name} for deployment.")
    print()
    print("[package] Next: deploy deliberately by copying this file over")
    print("[package] copy this file over %SALMA_DEPLOY_PATH%\\salma\\mo2-salma.dll,")
    print("[package] then verify the deployed file against the SHA-256 above.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
