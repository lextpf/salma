"""Make doxide's markdown fit for MkDocs Material, between the two builds.

What it does to each page:
- Removes @author attributions, both on their own line and mid-line inside a
  summary-table cell, where doxide flattens a whole doc block onto one line
- Strips @brief and @details tags
- Fixes admonition indentation (1-space to 4-space)
- Adds Material icons to page titles and section headers
- Trims every summary table (Types, Functions, Variables, ...) to first-sentence
  briefs
- Flattens namespace definition lists into single-line bullets
- Injects class listings under groups on the home page
- Injects the shipping API version (MO2_SALMA_API_VERSION in src/capi.rs) into
  the home page subtitle
- Injects the cross-link to the rustdoc engine API on the home page

Walks the docs directory recursively for `*.md` and rewrites matching files in
place, so point it at generated output only. A file matches only when its first
200 characters carry 'generator: doxide' frontmatter; everything else, the
tracked docs/main.html theme override included, is left alone.

build.bat runs this between `doxide build` and `mkdocs build`.

Usage:
    python scripts/_clean_docs.py          # defaults to docs/
    python scripts/_clean_docs.py path/    # custom docs directory
"""

import re
import sys
from pathlib import Path


def is_doxide_generated(text: str) -> bool:
    return "generator: doxide" in text[:200]


def fix_admonition_indent(text: str) -> str:
    """Fix doxide's 1-space admonition indent to 4-space for MkDocs Material."""
    lines = text.split("\n")
    result = []
    in_admonition = False

    for line in lines:
        if re.match(r"^!!! \w+", line):
            in_admonition = True
            result.append(line)
            continue

        if in_admonition:
            # Body line with 1-space indent
            m = re.match(r"^ (\S.*)", line)
            if m:
                result.append("    " + m.group(1))
                continue
            # Continuation with deeper indent
            if line.startswith("  "):
                result.append("    " + line.lstrip())
                continue
            # Blank or unindented line ends the admonition
            in_admonition = False

        result.append(line)

    return "\n".join(result)


PAGE_TITLE_ICONS = {
    "Core":           ":material-cube-outline:",
    "FOMOD":          ":material-package-variant:",
    "Installation":   ":material-download:",
    "Server":         ":material-server:",
}


SECTION_ICONS = {
    "Types":               ":material-shape-outline:",
    "Functions":           ":material-function:",
    "Variables":           ":material-variable:",
    "Macros":              ":material-pound:",
    "Operators":           ":material-math-compass:",
    "Type Aliases":        ":material-link-variant:",
    "Type Details":        ":material-shape-outline:",
    "Type Alias Details":  ":material-link-variant:",
    "Function Details":    ":material-function:",
    "Variable Details":    ":material-variable:",
    "Macro Details":       ":material-pound:",
    "Operator Details":    ":material-math-compass:",
}


def add_page_title_icons(text: str) -> str:
    """Prepend Material icons to doxide-generated H1 page titles."""
    for title, icon in PAGE_TITLE_ICONS.items():
        text = re.sub(rf"^# {re.escape(title)}$", f"# {icon} {title}", text, count=1, flags=re.MULTILINE)
    return text


def add_section_icons(text: str) -> str:
    """Prepend Material icons to doxide-generated section headers."""
    for title, icon in SECTION_ICONS.items():
        text = text.replace(f"## {title}", f"## {icon} {title}")
    return text


# Section headers that introduce a doxide summary table: a two-column
# "| Name | Description |" listing whose rows link into the detail sections
# further down the page. Both the plain and the icon-prefixed spellings must
# match, because add_section_icons() has already run by the time the trimmer
# does. "... Details" headers are deliberately absent: those sections hold the
# full prose and must not be trimmed.
SUMMARY_TABLE_SECTIONS = (
    "Types",
    "Functions",
    "Variables",
    "Macros",
    "Operators",
    "Type Aliases",
)


def _is_summary_table_header(stripped: str) -> bool:
    """True when a line is a summary-table section header, with or without icon."""
    for name in SUMMARY_TABLE_SECTIONS:
        if stripped == f"## {name}":
            return True
        icon = SECTION_ICONS.get(name)
        if icon and stripped == f"## {icon} {name}":
            return True
    return False


def trim_summary_table_descriptions(text: str) -> str:
    """Keep only a brief first sentence in every doxide summary table row.

    Doxide emits an entity's whole doc block into the description column of the
    summary table that lists it, crushed onto one physical line. For a class
    with sectioned prose that fills a table cell with the entire class
    documentation, headings and all. Trimming each row to its first sentence
    leaves the detail under the matching Details section, where it belongs.

    Applies to every table in SUMMARY_TABLE_SECTIONS, not only Functions: the
    Types table holds the worst offenders, because class-level blocks are the
    longest in the codebase.
    """
    lines = text.split("\n")
    out = []
    in_summary_table = False

    for line in lines:
        stripped = line.strip()

        if _is_summary_table_header(stripped):
            in_summary_table = True
            out.append(line)
            continue

        # Any following section header ends the table context.
        if in_summary_table and stripped.startswith("## "):
            in_summary_table = _is_summary_table_header(stripped)
            out.append(line)
            continue

        if in_summary_table and stripped.startswith("| [") and stripped.endswith("|"):
            parts = [p.strip() for p in stripped.strip("|").split("|", 1)]
            if len(parts) == 2:
                name_col, desc_col = parts
                desc_col = re.sub(r"\s+", " ", desc_col).strip()
                # Keep only first sentence in summary table.
                m = re.match(r"^(.*?\.)\s+.*$", desc_col)
                brief = m.group(1) if m else desc_col
                out.append(f"| {name_col} | {brief} |")
                continue

        out.append(line)

    return "\n".join(out)


def flatten_namespace_lists(text: str) -> str:
    """Flatten doxide namespace definition lists into single-line bullets.

    Home page namespace entries are emitted as definition lists:
        :material-package: [Name](...)
        :    Description
    which renders description on the next line. Convert these to:
        - :material-package: [Name](...) - Description
    """
    lines = text.split("\n")
    out = []
    i = 0

    while i < len(lines):
        line = lines[i]
        term = line.strip()
        if term.startswith(":material-package:") or term.startswith(":material-format-section:"):
            desc = ""
            if i + 1 < len(lines):
                m = re.match(r"^:\s+(.*)$", lines[i + 1])
                if m:
                    desc = m.group(1).strip()
                    i += 1

            if desc:
                out.append(f"- {term} - {desc}")
            else:
                out.append(f"- {term}")

            i += 1
            if i < len(lines) and lines[i].strip() == "":
                i += 1
            continue

        out.append(line)
        i += 1

    return "\n".join(out)


def collect_members(index_path: Path, prefix: str) -> list[tuple[str, str, str]]:
    """Extract types and functions from a group or subgroup index.md.

    Returns (name, relative_path, description) tuples with `prefix` prepended,
    so every path is relative to the docs root rather than to the page it came
    from. An anchor-only link is rewritten onto that page's index.md.
    """
    if not index_path.exists():
        return []
    text = index_path.read_text(encoding="utf-8")
    results = []

    for m in re.finditer(
        r"^\| \[([^\]]+)\]\(([^)]+)\) \|(.+)\|",
        text,
        re.MULTILINE,
    ):
        name = m.group(1)
        rel_path = m.group(2).strip()
        desc = m.group(3).strip()
        # Strip leftover @brief tag
        desc = re.sub(r"^@brief\s+", "", desc)
        # Anchor links (#func) need the index.md path prepended
        if rel_path.startswith("#"):
            full_path = f"{prefix}index.md{rel_path}"
        else:
            full_path = f"{prefix}{rel_path}"
        results.append((name, full_path, desc))

    return results


def collect_group_members(docs_dir: Path, group_dir: str) -> list[tuple[str, str, str]]:
    """Collect the types of a group and of every subgroup beneath it.

    Reads the group's own index.md first, then each subgroup's index.md.
    """
    group_index = docs_dir / group_dir / "index.md"
    prefix = f"{group_dir}/"
    members = []

    # Direct members in the group (types + functions)
    members.extend(collect_members(group_index, prefix))

    # Find subgroup links: :material-format-section: [Name](SubDir/index.md)
    if group_index.exists():
        text = group_index.read_text(encoding="utf-8")
        for m in re.finditer(
            r":material-format-section: \[([^\]]+)\]\(([^)]+)/index\.md\)",
            text,
        ):
            sub_name = m.group(1)
            sub_dir = m.group(2)
            # Class-based subgroups have content in the nested stub; try there first.
            sub_index = docs_dir / group_dir / sub_dir / "index.md"
            sub_members = collect_members(sub_index, f"{group_dir}/{sub_dir}/")
            if sub_members:
                members.extend(sub_members)
                continue
            # Fallback for a namespace-based subgroup: doxide writes an empty
            # stub at the nested path (warning "namespace cannot have @ingroup,
            # ignoring") and puts the real content in a top-level directory
            # instead. Every subgroup in doxide.yml is a class, so this branch
            # does not fire on a clean build. It is unsafe on a docs/ that was
            # never wiped, because the top-level directory it reads may be a
            # stale leftover.
            top_index = docs_dir / sub_dir / "index.md"
            members.extend(collect_members(top_index, f"{sub_dir}/"))

    return members


def inject_group_members(text: str, docs_dir: Path) -> str:
    """Append a flat class and function listing after the home page group list.

    Collects the types and functions from every group index.md and its subgroup
    pages, then inserts them as one bullet list after the last group entry.
    Idempotent: lines injected by an earlier run are stripped first.
    """
    lines = text.split("\n")
    # Strip previously-injected member lines (top-level and indented)
    lines = [l for l in lines if not re.match(r"^-?\s*- :material-package:", l)]

    out = []
    all_members = []
    last_group_idx = -1

    for line in lines:
        out.append(line)

        m = re.match(
            r"^- :material-format-section: \[.*\]\(([^/]+)/index\.md\)",
            line,
        )
        if not m:
            continue

        last_group_idx = len(out) - 1
        group_dir = m.group(1)
        members = collect_group_members(docs_dir, group_dir)
        for name, path, desc in members:
            sentence = re.match(r"^(.*?\.)\s", desc)
            brief = sentence.group(1) if sentence else desc
            all_members.append(f"- :material-package: [{name}]({path}) - {brief}")

    if all_members and last_group_idx >= 0:
        insert_at = last_group_idx + 1
        out.insert(insert_at, "")
        for j, entry in enumerate(all_members):
            out.insert(insert_at + 1 + j, entry)

    return "\n".join(out)


def parse_version(repo_root: Path) -> str:
    """Read the shipping API version, the value the release artifacts carry.

    The source of truth is `MO2_SALMA_API_VERSION` in `src/capi.rs`: the DLL
    returns it from getApiVersion, and CMakeLists.txt parses the same constant
    into SALMA_API_VERSION for the CPack archive name. `project(salma VERSION
    ...)` in CMakeLists.txt is decorative and lags behind, so it serves only as
    a fallback that keeps a version on the site when capi.rs is unreadable.

    Returns "" when neither source yields a major.minor.patch number.
    """
    capi = repo_root / "src" / "capi.rs"
    if capi.exists():
        m = re.search(
            r'MO2_SALMA_API_VERSION\s*:\s*&str\s*=\s*"(\d+\.\d+\.\d+)"',
            capi.read_text(encoding="utf-8"),
        )
        if m:
            return m.group(1)

    cmakelists = repo_root / "CMakeLists.txt"
    if not cmakelists.exists():
        return ""
    content = cmakelists.read_text(encoding="utf-8")
    m = re.search(r"project\s*\([^)]*VERSION\s+(\d+\.\d+\.\d+)", content)
    return m.group(1) if m else ""


def inject_version(text: str, version: str) -> str:
    """Prefix the home page subtitle line with the version badge.

    Idempotent whatever the version: badges from an earlier run are stripped
    before the current one is written. Without that strip a changed version
    stacks a second badge on the same line, which is what a docs/ directory that
    was never regenerated produces. Does nothing when `version` is empty or the
    H1 plus subtitle pair is absent.
    """
    if not version:
        return text
    # Drop any badge an earlier run left on the subtitle line.
    text = re.sub(
        r"^(# salma\n\n)(?:\*\*v\d+\.\d+\.\d+\*\*\s*\|\s*)+",
        r"\1",
        text,
        count=1,
        flags=re.MULTILINE,
    )
    return re.sub(
        r"^(# salma)\n\n(.+)$",
        rf"\1\n\n**v{version}** | \2",
        text,
        count=1,
        flags=re.MULTILINE,
    )


# Relative to the generated site root. build.bat step 7 copies target/doc there
# after `mkdocs build`, because mkdocs clears site/ on every run. The link is
# injected here rather than declared in mkdocs.yml's nav for the same reason:
# at nav-validation time the directory does not exist yet.
RUST_DOCS_HREF = "rust/mo2_salma_rs/index.html"

RUST_DOCS_BLOCK = f"""!!! abstract ":material-language-rust: Rust engine API"

    All engine logic - archives, FOMOD parsing, the CSP solver, inference -
    lives in the Rust crate and is documented by rustdoc, not by doxide.

    [Browse the engine API]({RUST_DOCS_HREF})

    The pages below cover the C++ that remains: the Crow HTTP server and the
    salma-support static library.
"""


def inject_rust_link(text: str) -> str:
    """Point the home page at the rustdoc output. Idempotent.

    The block sits directly under the subtitle, not at the foot of the page: the
    Rust crate is the large majority of src/, so a link buried below the C++
    class listing would misrepresent where the code is.
    """
    if RUST_DOCS_HREF in text:
        return text
    # After the subtitle line (the one inject_version writes into), else after
    # the H1.  Both anchors are stable doxide output.
    subtitle = re.search(r"^# salma\n\n.+\n", text, flags=re.MULTILINE)
    if not subtitle:
        return text
    at = subtitle.end()
    return text[:at] + "\n" + RUST_DOCS_BLOCK + text[at:]


def clean(text: str) -> str:
    # Remove standalone @author lines
    text = re.sub(r"^\s*@author\b.*\n?", "", text, flags=re.MULTILINE)

    # Doxide crushes a whole doc block onto one line inside a summary table, so
    # the file/type @author lands mid-line where the rule above cannot see it
    # and renders as page text. Strip the house form "@author Name (url)"
    # wherever it appears. Only the parenthesised form is matched: a bare
    # "@author Name" has no end marker, and consuming to end of line would eat
    # the rest of the table row, including its closing pipe.
    text = re.sub(r"[ \t]*@author\b[^\n(]*\([^)\n]*\)", "", text)

    # Strip @brief and @details tags but keep the description text
    text = re.sub(r"@brief\s+", "", text)
    text = re.sub(r"@details\s*\n?", "", text)

    # Fix admonition indentation (doxide outputs 1-space, MkDocs needs 4)
    text = fix_admonition_indent(text)

    # Add icons to page titles
    text = add_page_title_icons(text)

    # Add icons to section headers
    text = add_section_icons(text)

    # Trim over-detailed summary table entries.
    text = trim_summary_table_descriptions(text)

    # Keep namespace descriptions inline on Home/namespace listings.
    text = flatten_namespace_lists(text)

    return text


def main():
    docs_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("docs")

    if not docs_dir.is_dir():
        print(f"error: {docs_dir} is not a directory", file=sys.stderr)
        sys.exit(1)

    repo_root = docs_dir.resolve().parent
    version = parse_version(repo_root)
    if version:
        print(f"  version: {version}")

    changed = 0
    for md in docs_dir.rglob("*.md"):
        original = md.read_text(encoding="utf-8")
        if not is_doxide_generated(original):
            continue

        cleaned = clean(original)

        is_home = md.name == "index.md" and md.parent == docs_dir

        # Home page: inject version and group member listings
        if is_home:
            cleaned = inject_group_members(cleaned, docs_dir)
            if version:
                cleaned = inject_version(cleaned, version)
            # After inject_version, so the subtitle it edits is already final.
            cleaned = inject_rust_link(cleaned)
        else:
            # Group index pages: swap the subgroup bullets to the package icon,
            # in the header area only, before the first ## section.
            parts = cleaned.split("\n## ", 1)
            parts[0] = re.sub(
                r"^- :material-format-section:",
                "- :material-package:",
                parts[0],
                flags=re.MULTILINE,
            )
            # Do not add a ../ rewrite to subgroup links here. doxide emits
            # subgroup content nested under its parent, so the link it writes
            # is already correct; prepending ../ turns Server/index.md's
            # "MultipartHandler/index.md" into "../MultipartHandler/index.md",
            # which resolves to nothing. Such a rewrite can look right on a
            # docs/ that was never wiped, because stale top-level pages still
            # sitting on disk satisfy it. Verified against a wiped docs/:
            # mkdocs reports no missing links for the group index pages.
            cleaned = "\n## ".join(parts)

        if cleaned != original:
            md.write_text(cleaned, encoding="utf-8")
            changed += 1
            print(f"  cleaned {md.relative_to(docs_dir)}")

    print(f"done: {changed} file(s) cleaned")


if __name__ == "__main__":
    main()
