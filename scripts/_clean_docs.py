"""
@brief normalize generated doxide pages for MkDocs Material.
@author Alex (https://github.com/lextpf)

the command edits only `.md` files with doxide frontmatter in the first 200
characters. it rewrites files in place without backup; use generated output
only.

it removes documentation tags, corrects Material syntax, shortens summary
rows, and injects version, member, and rustdoc links on the home page.
"""

import re
import sys
from pathlib import Path


def is_doxide_generated(text: str) -> bool:
    return "generator: doxide" in text[:200]


def fix_admonition_indent(text: str) -> str:
    lines = text.split("\n")
    result = []
    in_admonition = False

    for line in lines:
        if re.match(r"^!!! \w+", line):
            in_admonition = True
            result.append(line)
            continue

        if in_admonition:
            m = re.match(r"^ (\S.*)", line)
            if m:
                result.append("    " + m.group(1))
                continue
            if line.startswith("  "):
                result.append("    " + line.lstrip())
                continue
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


def _add_heading_icons(text: str, level: int, icons: dict[str, str]) -> str:
    lines = []
    fence = ""
    for line in text.splitlines(keepends=True):
        marker = re.match(r"^ {0,3}(`{3,}|~{3,})(.*)$", line)
        if fence:
            if (marker and marker[1][0] == fence[0]
                    and len(marker[1]) >= len(fence) and not marker[2].strip()):
                fence = ""
        elif marker:
            fence = marker[1]
        else:
            # generated headings start at column zero; indented examples stay literal.
            heading = re.fullmatch(
                rf"(#{{{level}}} )(.+?)((?:[ \t]+\{{[^}}\r\n]*\}})?[ \t]*)(\r?\n)?",
                line,
            )
            if heading and (icon := icons.get(heading[2])):
                line = f"{heading[1]}{icon} {heading[2]}{heading[3]}{heading[4] or ''}"
        lines.append(line)
    return "".join(lines)


def add_page_title_icons(text: str) -> str:
    return _add_heading_icons(text, 1, PAGE_TITLE_ICONS)


def add_section_icons(text: str) -> str:
    """
    @fn add_section_icons(text: str) -> str
    @brief decorate complete generated section headings once.
    @author Alex (https://github.com/lextpf)

    authored Material headings keep their chosen icons. fenced and indented
    code examples keep literal heading text.
    """
    return _add_heading_icons(text, 2, SECTION_ICONS)


# detail sections retain full prose; summary tables keep one sentence.
SUMMARY_TABLE_SECTIONS = (
    "Types",
    "Functions",
    "Variables",
    "Macros",
    "Operators",
    "Type Aliases",
)


def _is_summary_table_header(stripped: str) -> bool:
    for name in SUMMARY_TABLE_SECTIONS:
        if stripped == f"## {name}":
            return True
        icon = SECTION_ICONS.get(name)
        if icon and stripped == f"## {icon} {name}":
            return True
    return False


def trim_summary_table_descriptions(text: str) -> str:
    """
    @fn trim_summary_table_descriptions(text: str) -> str
    @brief keep one sentence in generated summary-table descriptions.
    @author Alex (https://github.com/lextpf)

    doxide flattens each declaration block into one physical row. detail
    sections retain the full prose.
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

        if in_summary_table and stripped.startswith("## "):
            in_summary_table = _is_summary_table_header(stripped)
            out.append(line)
            continue

        if in_summary_table and stripped.startswith("| [") and stripped.endswith("|"):
            parts = [p.strip() for p in stripped.strip("|").split("|", 1)]
            if len(parts) == 2:
                name_col, desc_col = parts
                desc_col = re.sub(r"\s+", " ", desc_col).strip()
                m = re.match(r"^(.*?\.)\s+.*$", desc_col)
                brief = m.group(1) if m else desc_col
                out.append(f"| {name_col} | {brief} |")
                continue

        out.append(line)

    return "\n".join(out)


def flatten_namespace_lists(text: str) -> str:
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


def rewrite_sibling_entity_links(text: str, page_path: Path) -> str:
    """
    @fn rewrite_sibling_entity_links(text: str, page_path: Path) -> str
    @brief repair doxide links when an entity is emitted as a directory.
    @author Alex (https://github.com/lextpf)

    only relative sibling `X.md` targets are eligible. the rewrite requires
    `X.md` to be absent and `X/index.md` to exist. query and fragment suffixes
    are preserved.
    """
    def replace(match: re.Match) -> str:
        target = match.group("target")
        target_path = Path(target)
        if target_path.parent != Path("."):
            return match.group(0)

        sibling_file = page_path.parent / target_path
        sibling_index = page_path.parent / target_path.stem / "index.md"
        if sibling_file.exists() or not sibling_index.is_file():
            return match.group(0)

        suffix = match.group("suffix") or ""
        return f"{match.group('open')}{target[:-3]}/index.md{suffix})"

    return re.sub(
        r"(?P<open>(?<!\!)\[[^\]\n]+\]\()"
        r"(?P<target>[^()\s?#]+\.md)"
        r"(?P<suffix>[?#][^)\s]*)?\)",
        replace,
        text,
        flags=re.IGNORECASE,
    )


def collect_members(index_path: Path, prefix: str) -> list[tuple[str, str, str]]:
    """
    @fn collect_members(index_path: Path, prefix: str) -> list[tuple[str, str, str]]
    @brief make member links relative to the documentation root.
    @author Alex (https://github.com/lextpf)

    the result contains name, path, and description tuples. anchor-only links
    target the source page.
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
        desc = re.sub(r"^@brief\s+", "", desc)
        if rel_path.startswith("#"):
            full_path = f"{prefix}index.md{rel_path}"
        else:
            full_path = f"{prefix}{rel_path}"
        results.append((name, full_path, desc))

    return results


def collect_group_members(docs_dir: Path, group_dir: str) -> list[tuple[str, str, str]]:
    """
    @fn collect_group_members(docs_dir: Path, group_dir: str) -> list[tuple[str, str, str]]
    @brief collect members from one group and its subgroups.
    @author Alex (https://github.com/lextpf)

    nested subgroup content takes precedence. namespace groups can expose
    content at the documentation root.
    """
    group_index = docs_dir / group_dir / "index.md"
    prefix = f"{group_dir}/"
    members = []

    members.extend(collect_members(group_index, prefix))

    if group_index.exists():
        text = group_index.read_text(encoding="utf-8")
        for m in re.finditer(
            r":material-format-section: \[([^\]]+)\]\(([^)]+)/index\.md\)",
            text,
        ):
            sub_name = m.group(1)
            sub_dir = m.group(2)
            sub_index = docs_dir / group_dir / sub_dir / "index.md"
            sub_members = collect_members(sub_index, f"{group_dir}/{sub_dir}/")
            if sub_members:
                members.extend(sub_members)
                continue
            top_index = docs_dir / sub_dir / "index.md"
            members.extend(collect_members(top_index, f"{sub_dir}/"))

    return members


def inject_group_members(text: str, docs_dir: Path) -> str:
    """
    @fn inject_group_members(text: str, docs_dir: Path) -> str
    @brief list group members on the home page.
    @author Alex (https://github.com/lextpf)

    the function removes injected member rows before it writes the current
    list.
    """
    lines = text.split("\n")
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
    """
    @fn parse_version(repo_root: Path) -> str
    @brief read the API version used by release artifacts.
    @author Alex (https://github.com/lextpf)

    `MO2_SALMA_API_VERSION` is authoritative. the CMake project version is the
    fallback. the result is empty when neither source contains three numeric
    components.
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
    """
    @fn inject_version(text: str, version: str) -> str
    @brief put one current version badge in the home-page subtitle.
    @author Alex (https://github.com/lextpf)

    the function removes existing version badges first. it leaves the page
    unchanged when `version` is empty or the subtitle is absent.
    """
    if not version:
        return text
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


# mkdocs clears the site before rustdoc is copied, so inject this post-build link.
RUST_DOCS_HREF = "rust/mo2_salma_rs/index.html"

RUST_DOCS_BLOCK = f"""!!! abstract ":material-language-rust: Rust engine API"

    rustdoc covers archives, FOMOD parsing, constraint solving, and inference.

    [browse the engine API]({RUST_DOCS_HREF})
"""


def inject_rust_link(text: str) -> str:
    if RUST_DOCS_HREF in text:
        return text
    subtitle = re.search(r"^# salma\n\n.+\n", text, flags=re.MULTILINE)
    if not subtitle:
        return text
    at = subtitle.end()
    return text[:at] + "\n" + RUST_DOCS_BLOCK + text[at:]


def clean(text: str) -> str:
    text = re.sub(r"^\s*@author\b.*\n?", "", text, flags=re.MULTILINE)

    # match only the parenthesized house form; a bare author has no safe end marker.
    text = re.sub(r"[ \t]*@author\b[^\n(]*\([^)\n]*\)", "", text)

    text = re.sub(r"@brief\s+", "", text)
    text = re.sub(r"@details\s*\n?", "", text)

    text = fix_admonition_indent(text)
    text = add_page_title_icons(text)
    text = add_section_icons(text)
    text = trim_summary_table_descriptions(text)
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
    # home aggregation reads cleaned group pages, so process the root index last.
    pages = sorted(docs_dir.rglob("*.md"), key=lambda path: path == docs_dir / "index.md")
    for md in pages:
        original = md.read_text(encoding="utf-8")
        if not is_doxide_generated(original):
            continue

        cleaned = clean(original)
        cleaned = rewrite_sibling_entity_links(cleaned, md)

        is_home = md.name == "index.md" and md.parent == docs_dir

        if is_home:
            cleaned = inject_group_members(cleaned, docs_dir)
            if version:
                cleaned = inject_version(cleaned, version)
            cleaned = inject_rust_link(cleaned)
        else:
            # change subgroup icons only before the first section.
            parts = cleaned.split("\n## ", 1)
            parts[0] = re.sub(
                r"^- :material-format-section:",
                "- :material-package:",
                parts[0],
                flags=re.MULTILINE,
            )
            # keep nested links unchanged; a ../ prefix breaks their targets.
            cleaned = "\n## ".join(parts)

        cleaned = cleaned.rstrip() + "\n"
        if cleaned != original:
            md.write_text(cleaned, encoding="utf-8")
            changed += 1
            print(f"  cleaned {md.relative_to(docs_dir)}")

    print(f"done: {changed} file(s) cleaned")


if __name__ == "__main__":
    main()
