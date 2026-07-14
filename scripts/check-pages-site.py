#!/usr/bin/env python3
"""Validate the static Buzz support site without external dependencies."""

from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlparse

SITE = Path(__file__).resolve().parents[1] / "site"


class LinkParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.links: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = dict(attrs)
        if tag in {"a", "link"}:
            attribute = "href"
        elif tag in {"img", "script", "source"}:
            attribute = "src"
        else:
            attribute = None
        value = attributes.get(attribute) if attribute else None
        if value:
            self.links.append(value)


def resolve_local_link(page: Path, link: str) -> Path | None:
    parsed = urlparse(link)
    if parsed.scheme or parsed.netloc or link.startswith("#"):
        return None
    target = (page.parent / parsed.path).resolve()
    if not target.is_relative_to(SITE.resolve()):
        raise AssertionError(f"{page.relative_to(SITE)}: link escapes site: {link}")
    if parsed.path.endswith("/") or target.is_dir():
        target /= "index.html"
    return target


def main() -> None:
    pages = sorted(SITE.rglob("*.html"))
    required = {SITE / "index.html", SITE / "privacy/index.html", SITE / "support/index.html"}
    missing = required.difference(pages)
    if missing:
        raise AssertionError(f"missing required pages: {sorted(str(path) for path in missing)}")

    for page in pages:
        text = page.read_text(encoding="utf-8")
        parser = LinkParser()
        parser.feed(text)
        if '<html lang="en">' not in text or '<meta name="viewport"' not in text:
            raise AssertionError(f"{page.relative_to(SITE)}: missing required document metadata")
        for link in parser.links:
            target = resolve_local_link(page, link)
            if target is not None and not target.is_file():
                raise AssertionError(
                    f"{page.relative_to(SITE)}: broken local link {link} -> {target.relative_to(SITE)}"
                )

    print(f"Validated {len(pages)} HTML pages and their local links.")


if __name__ == "__main__":
    main()
