#!/usr/bin/env python
"""Produce a deployable artifact directory for the salma engine DLL.

Builds `mo2_salma_rs.dll` in release, then stages it into `target/package/`
under the name the MO2 plugin expects.

The rename is the point of this script. `scripts/mo2-salma.py::find_dll` looks
for `mo2-salma.dll` while the cargo artifact is `mo2_salma_rs.dll`. Keeping the
two names apart means a file called `mo2-salma.dll` has provably been through
this step, and a raw cargo artifact sitting somewhere else can never be mistaken
for a deployable one. Do not let the build rename silently; the auditable step
is the guarantee.

Usage:
  python scripts/package.py                  # build + stage
  python scripts/package.py --no-build       # stage an existing build
  python scripts/package.py --out DIR        # stage somewhere else
  python scripts/package.py --keep-rust-name # stage as mo2_salma_rs.dll

The artifact directory holds the DLL and nothing else: the engine has no runtime
data files, and it creates `logs/` next to the DLL on first use. CUTOVER.md
covers deploying it and rolling back.

The printed SHA-256 is the only practical proof that a later deploy carries these
exact bytes, because getApiVersion returns the same string for every build.

Exits non-zero when `cargo build` fails, and when the built DLL is absent under
`--no-build`. Apart from that build it writes only into the artifact directory,
and it never deploys: copying the staged file to MO2 stays a manual step.
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
    print("[package] copy this file over %SALMA_DEPLOY_PATH%\\salma\\mo2-salma.dll,")
    print("[package] then verify by comparing the deployed file's SHA-256")
    print("[package] against the digest above. getApiVersion cannot confirm a")
    print("[package] deploy: every build of the engine reports the same string.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
