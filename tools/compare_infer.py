#!/usr/bin/env python3
"""DLL-vs-committed-C++ inference parity gate for the Rust mo2_salma_rs.dll.

Loads the Rust cdylib via ctypes (the same argtypes/restype the MO2 plugin uses)
and runs `inferFomodSelections(archive_path, mod_dir)` against the real corpus,
comparing each result to the committed C++ output for that mod.

Two modes:

  full (default) - iterate tests/golden/full/manifest.json's fixtures. Each
    full/<fixture_file>.json carries `archive_path` and `output_json` (the C++
    DLL's committed output). The installed mod dir is `<mods_path>/<mod_name>`.

  --curated      - REMOVED. The committed case corpus was built from real
                   mods and no longer ships; see _iter_curated_cases.
    Each case.json carries `source_archive_path` and `mod_name`; the committed
    C++ output is the sibling expected.json (empty for expected_status=="empty").
    The installed mod dir is `<mods_path>/<mod_name>` (mods_path from the
    manifest, or --mods-path).

Comparison per case (both sides parsed as JSON unless both are empty):

  EXACT         - byte-identical (or both empty).
  METRICS_EQUAL - equal after zeroing diagnostics.timings_ms.{list,scan,solve,
                  total} on both sides AND set-comparing every reasons[].detail
                  .files array (the documented UNIQUE_FILE_EVIDENCE unordered-set
                  divergence). Everything else must match.
  DIVERGE       - a real difference remains.
  SKIP          - the archive file or the mod dir is missing (counted, not fatal).

Exit code is nonzero if any case DIVERGEs (or on a usage/load error). Timings and
the UNIQUE_FILE_EVIDENCE files set are the ONLY sanctioned divergences; see
PARITY-NOTES.md "Task 12".

Usage:
    python compare_infer.py <path-to-mo2_salma_rs.dll> [--curated]
                            [--mods-path DIR] [--limit N] [--verbose]
"""
import ctypes
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.join(HERE, "..", "tests", "golden")
FULL_DIR = os.path.join(GOLDEN, "full")
CASES_DIR = os.path.join(GOLDEN, "cases")
MANIFEST = os.path.join(FULL_DIR, "manifest.json")


def _configure_dll(lib):
    """Mirror of scripts/mo2-salma.py::_configure_dll for the two symbols used."""
    lib.inferFomodSelections.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.inferFomodSelections.restype = ctypes.c_void_p  # owned heap pointer
    lib.freeResult.argtypes = [ctypes.c_void_p]
    lib.freeResult.restype = None


def _infer(lib, archive_path, mod_dir):
    """Call inferFomodSelections, copy the bytes, and free the heap pointer."""
    addr = lib.inferFomodSelections(
        archive_path.encode("utf-8"), mod_dir.encode("utf-8")
    )
    if not addr:
        return ""
    try:
        return ctypes.string_at(addr).decode("utf-8")
    finally:
        lib.freeResult(addr)


def _zero_timings(doc):
    """Zero diagnostics.timings_ms.{list,scan,solve,total} in place."""
    timings = doc.get("diagnostics", {}).get("timings_ms")
    if isinstance(timings, dict):
        for key in ("list", "scan", "solve", "total"):
            if key in timings:
                timings[key] = 0
    return doc


def _canonicalize_evidence_files(doc):
    """Sort every reasons[].detail.files array so the documented unordered-set
    ordering (UNIQUE_FILE_EVIDENCE) is compared as a set, not a sequence.

    Walks steps[].groups[].{plugins,deselected}[].reasons[].detail.files. When
    detail.count exceeds 4 the C++ picks an unreproducible 4-of-N subset, so a
    length mismatch there is tolerated (only the sorted intersection semantics
    matter); we collapse such arrays to a sentinel so only `count` is compared.
    """
    for step in doc.get("steps", []):
        for group in step.get("groups", []):
            for key in ("plugins", "deselected"):
                for plugin in group.get(key, []):
                    for reason in plugin.get("reasons", []):
                        detail = reason.get("detail")
                        if not isinstance(detail, dict):
                            continue
                        files = detail.get("files")
                        if not isinstance(files, list):
                            continue
                        count = detail.get("count", len(files))
                        if isinstance(count, int) and count > 4:
                            # 4-of-N subset is nondeterministic on both sides:
                            # compare only the count, blank the examples.
                            detail["files"] = ["<subset>"]
                        else:
                            detail["files"] = sorted(files)
    return doc


def _classify(rust_out, cpp_out):
    """Return "EXACT" / "METRICS_EQUAL" / "DIVERGE" and an optional diff snippet."""
    if rust_out == cpp_out:
        return "EXACT", None
    both_empty = rust_out == "" and cpp_out == ""
    if both_empty:
        return "EXACT", None
    # One empty, the other not -> a real divergence.
    if rust_out == "" or cpp_out == "":
        snippet = f"one side empty: rust_len={len(rust_out)} cpp_len={len(cpp_out)}"
        return "DIVERGE", snippet
    try:
        rust_doc = json.loads(rust_out)
        cpp_doc = json.loads(cpp_out)
    except json.JSONDecodeError as exc:
        return "DIVERGE", f"json decode error: {exc}"

    rust_n = _canonicalize_evidence_files(_zero_timings(rust_doc))
    cpp_n = _canonicalize_evidence_files(_zero_timings(cpp_doc))
    if rust_n == cpp_n:
        return "METRICS_EQUAL", None

    return "DIVERGE", _first_json_diff(rust_n, cpp_n)


def _first_json_diff(a, b, path="$"):
    """A short human-readable description of the first structural difference."""
    if type(a) is not type(b):
        return f"{path}: type {type(a).__name__} vs {type(b).__name__}"
    if isinstance(a, dict):
        for key in sorted(set(a) | set(b)):
            if key not in a:
                return f"{path}.{key}: missing on rust"
            if key not in b:
                return f"{path}.{key}: missing on cpp"
            sub = _first_json_diff(a[key], b[key], f"{path}.{key}")
            if sub:
                return sub
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f"{path}: array len {len(a)} vs {len(b)}"
        for i, (x, y) in enumerate(zip(a, b)):
            sub = _first_json_diff(x, y, f"{path}[{i}]")
            if sub:
                return sub
        return None
    if a != b:
        return f"{path}: {a!r} vs {b!r}"
    return None


def _load_manifest():
    with open(MANIFEST, encoding="utf-8") as handle:
        return json.load(handle)


def _iter_full_cases(mods_path):
    """Yield (name, archive_path, mod_dir, cpp_output) for each manifest fixture."""
    manifest = _load_manifest()
    for entry in manifest.get("fixtures", []):
        fixture_file = os.path.join(FULL_DIR, entry["fixture_file"])
        if not os.path.isfile(fixture_file):
            continue
        with open(fixture_file, encoding="utf-8") as handle:
            data = json.load(handle)
        mod_name = data.get("mod_name", entry.get("mod_name", ""))
        yield (
            mod_name,
            data.get("archive_path", ""),
            os.path.join(mods_path, mod_name),
            data.get("output_json", ""),
        )


def _iter_curated_cases(mods_path):
    """Yield (name, archive_path, mod_dir, cpp_output) for each curated case.

    The curated cases were committed C++ oracle output built from real mods.
    They were removed from the repo because the case names, FOMOD documents and
    local archive paths together described the mod setup. Only the gitignored
    full corpus under tests/golden/full/ remains, and only on the machine that
    generated it.
    """
    if not os.path.isdir(CASES_DIR):
        sys.exit(
            "--curated is unavailable: the committed case corpus was removed.\n"
            "It was built from real mods and is not in the repo any more.\n"
            "Use the full corpus instead (drop --curated), which reads the\n"
            "gitignored tests/golden/full/ that gen_golden.py produces locally."
        )
    for name in sorted(os.listdir(CASES_DIR)):
        case_json = os.path.join(CASES_DIR, name, "case.json")
        if not os.path.isfile(case_json):
            continue
        with open(case_json, encoding="utf-8") as handle:
            case = json.load(handle)
        mod_name = case.get("mod_name", "")
        # Committed C++ output is the sibling expected.json (empty when the
        # expected status is "empty").
        expected_path = os.path.join(CASES_DIR, name, "expected.json")
        cpp_output = ""
        if os.path.isfile(expected_path):
            with open(expected_path, encoding="utf-8") as handle:
                cpp_output = handle.read()
        yield (
            case.get("case_name", name),
            case.get("source_archive_path", ""),
            os.path.join(mods_path, mod_name),
            cpp_output,
        )


def main(argv):
    args = argv[1:]
    if not args:
        print(__doc__, file=sys.stderr)
        return 2

    dll_path = args[0]
    curated = "--curated" in args
    verbose = "--verbose" in args
    mods_path = None
    limit = None
    for i, arg in enumerate(args):
        if arg == "--mods-path" and i + 1 < len(args):
            mods_path = args[i + 1]
        if arg == "--limit" and i + 1 < len(args):
            limit = int(args[i + 1])

    if mods_path is None:
        try:
            mods_path = _load_manifest().get("mods_path", "")
        except OSError as exc:
            print(f"cannot read manifest for mods_path: {exc}", file=sys.stderr)
            return 2

    lib = ctypes.CDLL(dll_path)
    _configure_dll(lib)

    cases = _iter_curated_cases(mods_path) if curated else _iter_full_cases(mods_path)

    counts = {"EXACT": 0, "METRICS_EQUAL": 0, "DIVERGE": 0, "SKIP": 0}
    diverges = []
    processed = 0

    for name, archive_path, mod_dir, cpp_output in cases:
        if limit is not None and processed >= limit:
            break
        # Skip when the archive or the installed mod dir is absent on this host.
        if not archive_path or not os.path.isfile(archive_path) or not os.path.isdir(mod_dir):
            counts["SKIP"] += 1
            if verbose:
                print(f"[SKIP] {name} (archive or mod dir missing)")
            continue
        processed += 1

        rust_out = _infer(lib, archive_path, mod_dir)
        verdict, snippet = _classify(rust_out, cpp_output)
        counts[verdict] += 1
        if verdict == "DIVERGE":
            diverges.append((name, snippet))
        if verbose:
            print(f"[{verdict}] {name}")

    total = sum(counts.values())
    mode = "curated" if curated else "full"
    print()
    print(f"=== compare_infer parity ({mode}, {total} cases) ===")
    print(f"  EXACT         {counts['EXACT']}")
    print(f"  METRICS_EQUAL {counts['METRICS_EQUAL']}")
    print(f"  DIVERGE       {counts['DIVERGE']}")
    print(f"  SKIP          {counts['SKIP']}")
    if diverges:
        print("\nFirst DIVERGE cases:")
        for name, snippet in diverges[:5]:
            print(f"  - {name}: {snippet}")

    return 1 if counts["DIVERGE"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
