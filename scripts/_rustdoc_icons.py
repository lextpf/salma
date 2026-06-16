"""
@brief render Material section icons in generated Rustdoc HTML.
@author Alex (https://github.com/lextpf)

### :material-shield-lock: output scope

only HTML with the Rustdoc generator marker is eligible. text replacements preserve
attributes, heading IDs, links, and code samples. each changed page is replaced atomically.

### :material-memory: icon assets

SVG assets come from the installed MkDocs Material package. load them only when an
eligible text node contains a shortcode. an unknown icon fails the command.
"""

import argparse
import importlib.util
import re
import tempfile
from html.parser import HTMLParser
from pathlib import Path


SHORTCODE = re.compile(r":material-([A-Za-z0-9_-]+):")
EXCLUDED_TAGS = {"pre", "code", "script", "style", "textarea", "title"}


def material_icon_directory() -> Path:
    spec = importlib.util.find_spec("material")
    if spec is None or not spec.submodule_search_locations:
        raise RuntimeError("install mkdocs-material to render Rustdoc section icons")
    return Path(next(iter(spec.submodule_search_locations))) / "templates/.icons/material"


class MaterialIcons:
    """
    @class MaterialIcons
    @brief cache SVG assets after validating each shortcode name.
    @author Alex (https://github.com/lextpf)
    """

    def __init__(self, directory: Path | None = None):
        self.directory = directory
        self.cache: dict[str, str] = {}

    def svg(self, name: str) -> str:
        if name in self.cache:
            return self.cache[name]
        if not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", name):
            raise ValueError(f"invalid Material icon name: {name}")
        if self.directory is None:
            self.directory = material_icon_directory()
        path = self.directory / f"{name}.svg"
        if not path.is_file():
            raise ValueError(f"unknown Material icon: :material-{name}:")
        svg = path.read_text(encoding="utf-8").strip()
        if not svg.startswith("<svg "):
            raise ValueError(f"invalid Material SVG asset: {path}")
        svg = svg.replace(
            "<svg ",
            '<svg class="material-icon" aria-hidden="true" focusable="false" '
            'width="1em" height="1em" fill="currentColor" '
            'style="vertical-align: -0.125em;" ',
            1,
        )
        self.cache[name] = svg
        return svg


class _IconTextParser(HTMLParser):
    # record source offsets so serialization cannot alter unrelated HTML.
    def __init__(self, text: str):
        super().__init__(convert_charrefs=False)
        self.line_starts = [0] + [match.end() for match in re.finditer("\n", text)]
        self.excluded: list[str] = []
        self.generated = False
        self.matches: list[tuple[int, int, str]] = []

    def handle_starttag(self, tag, attrs):
        if tag == "meta":
            values = dict(attrs)
            if values.get("name") == "generator" and values.get("content") == "rustdoc":
                self.generated = True
        if tag in EXCLUDED_TAGS:
            self.excluded.append(tag)

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        self.handle_endtag(tag)

    def handle_endtag(self, tag):
        if self.excluded and self.excluded[-1] == tag:
            self.excluded.pop()

    def handle_data(self, data):
        if self.excluded:
            return
        line, column = self.getpos()
        offset = self.line_starts[line - 1] + column
        for match in SHORTCODE.finditer(data):
            self.matches.append((offset + match.start(), offset + match.end(), match.group(1)))


def render_icons(text: str, icons: MaterialIcons | None = None) -> str:
    """
    @fn render_icons(text, icons=None) -> str
    @brief replace eligible shortcodes without reserializing the HTML document.
    @author Alex (https://github.com/lextpf)

    attributes and code retain literal shortcodes. pages without the Rustdoc generator
    marker are returned unchanged, even when they contain an unknown icon.

    @param text generated HTML, including its original line endings.
    @param icons shared asset cache; null creates a lazy cache for this call.
    @return HTML with SVG text replacements, or the original text when none apply.
    """
    if not SHORTCODE.search(text):
        return text
    parser = _IconTextParser(text)
    parser.feed(text)
    parser.close()
    if not parser.generated or not parser.matches:
        return text
    if icons is None:
        icons = MaterialIcons()
    chunks = []
    cursor = 0
    for start, end, name in parser.matches:
        chunks.extend((text[cursor:start], icons.svg(name)))
        cursor = end
    chunks.append(text[cursor:])
    return "".join(chunks)


def process_directory(directory: Path, icons: MaterialIcons | None = None) -> int:
    """
    @fn process_directory(directory, icons=None) -> int
    @brief update generated HTML below the selected Rustdoc directory.
    @author Alex (https://github.com/lextpf)

    symlinks are skipped. unchanged pages keep their modification time. file replacement
    preserves the original page when writing the replacement fails.

    @param directory existing generated documentation directory.
    @param icons shared asset cache; null uses installed Material assets.
    @return number of changed HTML files.
    """
    directory = directory.resolve()
    if not directory.is_dir():
        raise FileNotFoundError(f"Rustdoc directory does not exist: {directory}")
    if icons is None:
        icons = MaterialIcons()
    changed = 0
    for path in sorted(directory.rglob("*.html")):
        if path.is_symlink() or not path.resolve().is_relative_to(directory):
            continue
        original = path.read_bytes()
        try:
            rendered = render_icons(original.decode("utf-8"), icons).encode("utf-8")
        except (ValueError, RuntimeError) as error:
            raise ValueError(f"{path}: {error}") from error
        if rendered == original:
            continue
        replacement = None
        try:
            with tempfile.NamedTemporaryFile(dir=path.parent, suffix=".tmp", delete=False) as file:
                replacement = Path(file.name)
                file.write(rendered)
            replacement.replace(path)
        finally:
            if replacement is not None and replacement.exists():
                replacement.unlink()
        changed += 1
    return changed


def main() -> int:
    parser = argparse.ArgumentParser(description="render Material icons in generated Rustdoc HTML")
    parser.add_argument("directory", nargs="?", type=Path, default=Path("target/doc"))
    args = parser.parse_args()
    try:
        changed = process_directory(args.directory)
    except (OSError, ValueError, RuntimeError) as error:
        parser.exit(1, f"Rustdoc icon rendering failed: {error}\n")
    print(f"Rendered Material icons in {changed} Rustdoc HTML files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
