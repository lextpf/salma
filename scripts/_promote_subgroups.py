"""Orphaned tool: promotes nested subgroup docs to top-level directories.

Keep it out of the doc build. No pipeline invokes it: between `doxide build` and
`mkdocs build`, build.bat calls `scripts/_clean_docs.py` and nothing else.

Why it is dead. doxide emits subgroup content nested under its parent group and
writes correct links to it, so there is nothing left for this script to make
reachable. Running it copies pages to a second location that nothing links to,
and because docs/ is never cleaned automatically those copies survive and
satisfy nav entries a fresh clone cannot. That is the failure mode to avoid.

What it does, if you run it anyway. It reads doxide.yml for the group hierarchy,
then copies `ParentGroup/SubGroup/` to `SubGroup/` for every pair, skipping any
pair whose top-level directory already exists. When the copied directory holds a
class page named after the subgroup (`Logger/Logger.md`), the class content
overwrites the stub index.md so a reader lands on the full page rather than an
intermediate one. It writes inside the docs directory and deletes the copied
class page; it never touches the source tree.

Standard library only, no external dependencies.

Usage:
    python scripts/_promote_subgroups.py          # defaults to docs/
    python scripts/_promote_subgroups.py path/    # custom docs directory
"""

import re
import shutil
import sys
from pathlib import Path


def parse_group_hierarchy(config_path: Path) -> list[tuple[str, str]]:
    """Parse doxide.yml and return (parent, child) name pairs.

    An indent-aware line parser rather than PyYAML, which keeps the script free
    of external dependencies.
    """
    text = config_path.read_text(encoding="utf-8")
    pairs = []
    parent_name = ""
    in_child_groups = False

    for line in text.split("\n"):
        stripped = line.rstrip()
        indent = len(line) - len(line.lstrip())

        # Top-level group: "  - name: Core" (indent 2-4)
        m = re.match(r"^  - name:\s+(.+)", stripped)
        if m:
            parent_name = m.group(1).strip()
            in_child_groups = False
            continue

        # Child groups key: "    groups:" (indent 4-6)
        if re.match(r"^\s{4,6}groups:\s*$", stripped):
            in_child_groups = True
            continue

        # Child group entry: "      - name: Logger" (indent 6+)
        if in_child_groups and indent >= 6:
            m = re.match(r"^\s+- name:\s+(.+)", stripped)
            if m:
                pairs.append((parent_name, m.group(1).strip()))
                continue

        # Any non-indented or top-level key resets child context
        if indent < 4 and stripped and not stripped.startswith("#"):
            in_child_groups = False

    return pairs


def promote_class_to_index(top_level: Path, child_name: str) -> None:
    """Replace the stub index.md with the class page content, when one exists.

    Doxide writes a stub index.md (a title and a Types table) plus a separate
    ClassName.md carrying the full documentation. This moves the class content
    into index.md and deletes the now duplicate file. Does nothing when there is
    no class page.
    """
    class_page = top_level / f"{child_name}.md"
    index_page = top_level / "index.md"

    if not class_page.exists():
        return

    content = class_page.read_text(encoding="utf-8")
    index_page.write_text(content, encoding="utf-8")
    class_page.unlink()


def main():
    docs_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("docs")
    config_path = Path("doxide.yml")

    if not config_path.exists():
        print("error: doxide.yml not found", file=sys.stderr)
        sys.exit(1)
    if not docs_dir.is_dir():
        print(f"error: {docs_dir} is not a directory", file=sys.stderr)
        sys.exit(1)

    pairs = parse_group_hierarchy(config_path)
    promoted = 0

    for parent_name, child_name in pairs:
        nested = docs_dir / parent_name / child_name
        top_level = docs_dir / child_name

        if nested.is_dir() and not top_level.exists():
            shutil.copytree(nested, top_level)
            promote_class_to_index(top_level, child_name)
            promoted += 1
            print(f"  {parent_name}/{child_name}/ -> {child_name}/")

    print(f"done: {promoted} subgroup(s) promoted")


if __name__ == "__main__":
    main()
