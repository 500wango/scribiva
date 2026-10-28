#!/usr/bin/env python3
"""Static checks for the landing page (landing/), stdlib only.

    python3 scripts/release/check_landing.py landing

For every .html file under the directory:

- well-formed: <!DOCTYPE html>, every non-void element is closed in order,
  no stray end tags, no duplicate id attributes, one <title>, <html lang>;
- every relative src/href (and srcset, CSS url()) resolves to a file inside the
  directory, and "#fragment" links point at an existing id;
- nothing is loaded from another origin: no absolute or protocol-relative URL
  in src/href/srcset of script, link, img, iframe, source, video, audio,
  embed, object, or in CSS url()/@import (the page must work from mainland
  China without third-party CDNs, FR-015). Plain <a> links may point anywhere.

Exits 1 with one line per problem, 0 when clean.
"""

from __future__ import annotations

import html.parser
import pathlib
import re
import sys
import urllib.parse

# Inside <svg>/<math> (foreign content) "<path/>" is a legal self-closing tag.
FOREIGN = {'svg', 'math'}
VOID = {'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link',
        'meta', 'source', 'track', 'wbr'}
# Elements that load a resource into the page: their URLs must stay same-origin.
LOADING = {'script', 'link', 'img', 'iframe', 'source', 'video', 'audio',
           'embed', 'object', 'track', 'input'}
URL_ATTRS = ('src', 'href', 'data', 'poster')
EXTERNAL = re.compile(r'^(?:[a-zA-Z][a-zA-Z0-9+.-]*:|//)')
CSS_URL = re.compile(r'url\(\s*([\'"]?)(.*?)\1\s*\)|@import\s+([\'"])(.*?)\3', re.IGNORECASE)


class Page(html.parser.HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.problems: list[str] = []
        self.stack: list[tuple[str, int]] = []
        self.ids: dict[str, int] = {}
        self.refs: list[tuple[int, str, str, bool]] = []  # line, tag, url, loads
        self.doctype = False
        self.titles = 0
        self.html_lang = None
        self._in_style = False
        self._css: list[tuple[int, str]] = []

    def _line(self) -> int:
        return self.getpos()[0]

    def handle_decl(self, decl: str) -> None:
        if decl.lower().split() == ['doctype', 'html']:
            self.doctype = True

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self._attrs(tag, attrs)
        if tag not in VOID:
            self.stack.append((tag, self._line()))
        if tag == 'style':
            self._in_style = True

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self._attrs(tag, attrs)
        foreign = tag in FOREIGN or any(t in FOREIGN for t, _ in self.stack)
        if tag not in VOID and not foreign:
            self.problems.append(f'{self._line()}: <{tag}/> is not a void element')

    def handle_endtag(self, tag: str) -> None:
        line = self._line()
        if tag in VOID:
            self.problems.append(f'{line}: end tag </{tag}> for a void element')
            return
        if tag == 'style':
            self._in_style = False
        if not self.stack:
            self.problems.append(f'{line}: stray </{tag}>')
            return
        open_tag, open_line = self.stack[-1]
        if open_tag == tag:
            self.stack.pop()
            return
        if any(t == tag for t, _ in self.stack):
            self.problems.append(f'{line}: </{tag}> closes <{open_tag}> from line {open_line}')
            while self.stack and self.stack[-1][0] != tag:
                self.stack.pop()
            self.stack.pop()
        else:
            self.problems.append(f'{line}: stray </{tag}>')

    def handle_data(self, data: str) -> None:
        if self._in_style:
            self._css.append((self._line(), data))

    def _attrs(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        line = self._line()
        values = dict(attrs)
        if tag == 'title':
            self.titles += 1
        if tag == 'html':
            self.html_lang = values.get('lang')
        ident = values.get('id')
        if ident:
            if ident in self.ids:
                self.problems.append(f'{line}: duplicate id="{ident}" (first on line {self.ids[ident]})')
            else:
                self.ids[ident] = line
        loads = tag in LOADING
        for name in URL_ATTRS:
            value = values.get(name)
            if value is not None:
                self.refs.append((line, tag, value.strip(), loads))
        for name in ('srcset', 'imagesrcset'):
            if values.get(name):
                for candidate in values[name].split(','):
                    url = candidate.strip().split(' ')[0]
                    if url:
                        self.refs.append((line, tag, url, True))
        if values.get('style'):
            self._css.append((line, values['style']))

    def css_refs(self) -> list[tuple[int, str]]:
        return css_urls(self._css)


def css_urls(chunks: list[tuple[int, str]]) -> list[tuple[int, str]]:
    found = []
    for line, text in chunks:
        for m in CSS_URL.finditer(text):
            url = (m.group(2) if m.group(2) is not None else m.group(4)).strip()
            found.append((line + text.count('\n', 0, m.start()), url))
    return found


def check_target(root: pathlib.Path, page: pathlib.Path, url: str,
                 ids_of) -> str | None:
    """Return a problem for a relative URL that does not resolve, else None."""
    parts = urllib.parse.urlsplit(url)
    path = urllib.parse.unquote(parts.path)
    if path.startswith('/'):
        return f'root-relative URL {url!r} breaks when the site is not at the domain root'
    target = (page.parent / path).resolve() if path else page.resolve()
    try:
        target.relative_to(root.resolve())
    except ValueError:
        return f'{url!r} points outside {root}'
    if target.is_dir():
        target = target / 'index.html'
    if not target.is_file():
        return f'{url!r} does not resolve ({target.relative_to(root.resolve())} missing)'
    if parts.fragment and target.suffix == '.html':
        if parts.fragment not in ids_of(target):
            return f'{url!r}: no id="{parts.fragment}" in {target.name}'
    return None


def check_site(root: pathlib.Path) -> list[str]:
    root = root.resolve()
    pages = sorted(root.rglob('*.html'))
    problems: list[str] = []
    if not pages:
        return [f'{root}: no .html files']
    parsed: dict[pathlib.Path, Page] = {}

    def parse(path: pathlib.Path) -> Page:
        if path not in parsed:
            p = Page()
            p.feed(path.read_text(encoding='utf-8'))
            p.close()
            parsed[path] = p
        return parsed[path]

    def ids_of(path: pathlib.Path) -> dict[str, int]:
        return parse(path.resolve()).ids

    def check_url(where: str, page: pathlib.Path, url: str, loads: bool) -> None:
        if not url or url.startswith('#') and len(url) == 1:
            return
        if EXTERNAL.match(url):
            scheme = url.split(':', 1)[0].lower()
            if loads or scheme not in ('http', 'https', 'mailto', 'tel'):
                if loads and scheme in ('data',):
                    return
                problems.append(f'{where}: external URL {url!r} not allowed here')
            return
        problem = check_target(root, page, url, ids_of)
        if problem:
            problems.append(f'{where}: {problem}')

    for path in pages:
        rel = path.relative_to(root)
        page = parse(path)
        for p in page.problems:
            problems.append(f'{rel}:{p}')
        for tag, line in page.stack:
            problems.append(f'{rel}:{line}: <{tag}> is never closed')
        if not page.doctype:
            problems.append(f'{rel}: missing <!DOCTYPE html>')
        if page.titles != 1:
            problems.append(f'{rel}: expected one <title>, found {page.titles}')
        if not page.html_lang:
            problems.append(f'{rel}: <html> has no lang attribute')
        for line, tag, url, loads in page.refs:
            check_url(f'{rel}:{line} <{tag}>', path, url, loads)
        for line, url in page.css_refs():
            check_url(f'{rel}:{line} css', path, url, True)

    for css in sorted(root.rglob('*.css')):
        rel = css.relative_to(root)
        for line, url in css_urls([(1, css.read_text(encoding='utf-8'))]):
            check_url(f'{rel}:{line}', css, url, True)
    return problems


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 1 or args[0].startswith('-'):
        print(__doc__.strip().split('\n\n')[1].strip(), file=sys.stderr)
        return 2
    root = pathlib.Path(args[0])
    if not root.is_dir():
        print(f'error: {root} is not a directory', file=sys.stderr)
        return 2
    problems = check_site(root)
    for p in problems:
        print(p)
    if problems:
        print(f'{len(problems)} problem(s) in {root}', file=sys.stderr)
        return 1
    count = len(list(root.rglob('*.html')))
    print(f'{root}: {count} page(s) OK')
    return 0


if __name__ == '__main__':
    sys.exit(main())
