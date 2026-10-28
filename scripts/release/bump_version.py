#!/usr/bin/env python3
"""Set the Scribiva release version in every place that carries it.

    python3 scripts/release/bump_version.py 1.0.1            # write
    python3 scripts/release/bump_version.py --dry-run 1.0.1  # show only
    python3 scripts/release/bump_version.py --check v1.0.1   # CI: all files match?

The version must be SemVer 2.0.0 (a leading "v" is accepted and dropped).
Each file is edited in place with a narrow pattern so formatting and the rest
of the file stay untouched. Nothing is written unless every pattern matches
exactly once. Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

# https://semver.org/#is-there-a-suggested-regular-expression-regex-to-check-a-semver-string
SEMVER = re.compile(
    r'^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)'
    r'(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)'
    r'(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?'
    r'(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$'
)

# (path relative to the repo root, pattern with a named group "v").
# Every pattern must match exactly once in its file.
TARGETS: list[tuple[str, str]] = [
    ('desktop/package.json', r'^  "version": "(?P<v>[^"\n]*)"'),
    ('desktop/package-lock.json', r'^  "version": "(?P<v>[^"\n]*)"'),
    ('desktop/package-lock.json',
     r'^    "": \{\n      "name": "[^"\n]*",\n      "version": "(?P<v>[^"\n]*)"'),
    ('desktop/src-tauri/Cargo.toml',
     r'\A\[package\]\n(?:(?!\[).*\n)*?version = "(?P<v>[^"\n]*)"'),
    # Keeps `cargo build --locked` working after the bump.
    ('desktop/src-tauri/Cargo.lock',
     r'^name = "scribiva-desktop"\nversion = "(?P<v>[^"\n]*)"'),
    ('desktop/src-tauri/tauri.conf.json', r'^  "version": "(?P<v>[^"\n]*)"'),
    ('src/scribiva/__init__.py', r"^__version__ = '(?P<v>[^'\n]*)'"),
    ('server/scribiva_server/__init__.py', r"^__version__ = '(?P<v>[^'\n]*)'"),
]

JSON_FILES = ('desktop/package.json', 'desktop/package-lock.json',
              'desktop/src-tauri/tauri.conf.json')


class VersionError(Exception):
    pass


def parse_version(text: str) -> str:
    """Return the SemVer string without a leading "v", or raise VersionError."""
    version = text.strip()
    if version[:1] in ('v', 'V'):
        version = version[1:]
    if not SEMVER.match(version):
        raise VersionError(f'not a SemVer 2.0.0 version: {text!r}')
    return version


def find(root: pathlib.Path) -> list[dict]:
    """Locate every version field. Raises VersionError if a pattern is off."""
    found = []
    for rel, pattern in TARGETS:
        path = root / rel
        if not path.is_file():
            raise VersionError(f'{rel}: file not found')
        text = path.read_text(encoding='utf-8')
        matches = list(re.finditer(pattern, text, re.MULTILINE))
        if len(matches) != 1:
            raise VersionError(
                f'{rel}: expected exactly one version field, found {len(matches)}')
        m = matches[0]
        found.append({
            'path': rel,
            'line': text.count('\n', 0, m.start('v')) + 1,
            'start': m.start('v'),
            'end': m.end('v'),
            'old': m.group('v'),
        })
    return found


def apply(root: pathlib.Path, found: list[dict], new: str) -> None:
    by_file: dict[str, list[dict]] = {}
    for item in found:
        by_file.setdefault(item['path'], []).append(item)
    for rel, items in by_file.items():
        path = root / rel
        text = path.read_text(encoding='utf-8')
        # Replace from the end so earlier offsets stay valid.
        for item in sorted(items, key=lambda i: i['start'], reverse=True):
            text = text[:item['start']] + new + text[item['end']:]
        if rel in JSON_FILES:
            json.loads(text)  # never write a broken JSON file
        path.write_text(text, encoding='utf-8')


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('version', help='new version, SemVer (e.g. 1.0.1 or v1.2.0-beta.1)')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--dry-run', action='store_true', help='show the changes, write nothing')
    mode.add_argument('--check', action='store_true',
                      help='exit 1 unless every file already carries this version')
    parser.add_argument('--root', type=pathlib.Path,
                        default=pathlib.Path(__file__).resolve().parents[2],
                        help='repository root (default: this checkout)')
    args = parser.parse_args(argv)

    try:
        new = parse_version(args.version)
        found = find(args.root)
    except VersionError as exc:
        print(f'error: {exc}', file=sys.stderr)
        return 2

    if args.check:
        wrong = [f for f in found if f['old'] != new]
        for f in wrong:
            print(f"{f['path']}:{f['line']}: {f['old']} != {new}", file=sys.stderr)
        if wrong:
            print(f'error: {len(wrong)} version field(s) do not match {new}; '
                  f'run scripts/release/bump_version.py {new}', file=sys.stderr)
            return 1
        print(f'All {len(found)} version fields are {new}.')
        return 0

    changed = 0
    for f in found:
        mark = '  ' if f['old'] == new else '~ '
        changed += f['old'] != new
        print(f"{mark}{f['path']}:{f['line']}: {f['old']} -> {new}")
    if args.dry_run:
        print(f'Dry run: {changed} of {len(found)} field(s) would change.')
        return 0
    apply(args.root, found, new)
    print(f'Updated {changed} of {len(found)} field(s) to {new}.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
