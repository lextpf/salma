#!/usr/bin/env python
"""Produce a deployable artifact directory for the Rust salma DLL.

Builds `mo2_salma_rs.dll` in release, then stages it into
`target/package/` under the name the MO2 plugin expects.

The rename is the whole point of this script. `scripts/mo2-salma.py::find_dll`
looks for `mo2-salma.dll`; the cargo artifact is `mo2_salma_rs.dll`. During the
parity phase the two names are kept distinct on purpose so a stray copy can
never be mistaken for the C++ build. Staging under the deploy name is therefore
an explicit, auditable step rather than something the build does silently.

Usage:
  python tools/package.py                  # build + stage
  python tools/package.py --no-build       # stage an existing build
  python tools/package.py --out DIR        # stage somewhere else
  python tools/package.py --keep-rust-name # stage as mo2_salma_rs.dll

The artifact directory holds the DLL and nothing else: the engine has no
runtime data files, and `logs/` is created next to the DLL on first use.
See CUTOVER.md for how to deploy it and how to roll back.
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
    # Remove first: overwriting a DLL another process still has mapped fails
    # with a sharing violation on Windows.
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
    print("[package] Next: see CUTOVER.md. Deploying is a deliberate step -")
    print("[package] copy this file over %SALMA_DEPLOY_PATH%\\salma\\mo2-salma.dll")
    print("[package] only after backing the C++ DLL up, and verify with")
    print("[package] getApiVersion plus a fresh logs/salma.log line.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
