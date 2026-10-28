#!/usr/bin/env python3
"""Validate the GNOME Shell extension and pack it the way `gnome-extensions pack` does.

    python3 scripts/release/check_extension.py extensions/gnome-shell/scribiva@scribiva.app \
        [--zip dist/scribiva@scribiva.app.shell-extension.zip]

`gnome-extensions` is not available on CI runners, so this checks what GNOME
Shell itself requires before it loads an extension:

- metadata.json is a JSON object with uuid, name, description and a non-empty
  shell-version list of major versions ("45", "46", ...);
- the uuid is well-formed and equals the directory name;
- the files the loader and the .deb staging (desktop/packaging/linux/stage.sh)
  expect are present: extension.js, impl.js, metadata.json, stylesheet.css.

With --zip the same file set as `build.sh` (`gnome-extensions pack
--extra-source=impl.js`) is written to a zip, read back and compared.
JavaScript syntax is checked separately with `node --check` in CI. Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys
import zipfile

UUID = re.compile(r'^[A-Za-z0-9._-]+@[A-Za-z0-9._-]+$')
SHELL_VERSION = re.compile(r'^[1-9]\d*(?:\.\d+)?$')
REQUIRED_FILES = ('extension.js', 'impl.js', 'metadata.json', 'stylesheet.css')
# What `gnome-extensions pack` puts in the zip: the default sources plus build.sh's
# --extra-source=impl.js.
PACKED_FILES = ('extension.js', 'impl.js', 'metadata.json', 'stylesheet.css')


def check_metadata(directory: pathlib.Path) -> tuple[dict, list[str]]:
    problems: list[str] = []
    path = directory / 'metadata.json'
    try:
        meta = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        return {}, [f'metadata.json: {exc}']
    if not isinstance(meta, dict):
        return {}, ['metadata.json: not a JSON object']
    for key in ('uuid', 'name', 'description'):
        value = meta.get(key)
        if not isinstance(value, str) or not value.strip():
            problems.append(f'metadata.json: "{key}" must be a non-empty string')
    uuid = meta.get('uuid')
    if isinstance(uuid, str):
        if not UUID.match(uuid):
            problems.append(f'metadata.json: uuid {uuid!r} is not of the form name@domain')
        if uuid != directory.name:
            problems.append(f'metadata.json: uuid {uuid!r} does not match directory {directory.name!r}')
    versions = meta.get('shell-version')
    if not isinstance(versions, list) or not versions:
        problems.append('metadata.json: "shell-version" must be a non-empty list')
    else:
        for v in versions:
            if not isinstance(v, str) or not SHELL_VERSION.match(v):
                problems.append(f'metadata.json: shell-version entry {v!r} is not a version string like "46"')
        if len(set(map(str, versions))) != len(versions):
            problems.append('metadata.json: duplicate shell-version entries')
    for key in ('version-name', 'url', 'gettext-domain', 'settings-schema'):
        if key in meta and not isinstance(meta[key], str):
            problems.append(f'metadata.json: "{key}" must be a string')
    if 'version' in meta and not (isinstance(meta['version'], int) and meta['version'] > 0):
        problems.append('metadata.json: "version" must be a positive integer')
    return meta, problems


def check_files(directory: pathlib.Path) -> list[str]:
    return [f'{name}: missing' for name in REQUIRED_FILES if not (directory / name).is_file()]


def pack(directory: pathlib.Path, out: pathlib.Path) -> list[str]:
    """Write the extension zip and verify it; return problems."""
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(out, 'w', compression=zipfile.ZIP_DEFLATED) as zf:
        for name in PACKED_FILES:
            zf.write(directory / name, arcname=name)
    return verify_zip(directory, out)


def verify_zip(directory: pathlib.Path, path: pathlib.Path) -> list[str]:
    problems = []
    try:
        with zipfile.ZipFile(path) as zf:
            names = sorted(zf.namelist())
            bad = zf.testzip()
            if bad:
                problems.append(f'{path.name}: corrupt member {bad}')
            for name in names:
                if name.startswith('/') or '..' in pathlib.PurePosixPath(name).parts:
                    problems.append(f'{path.name}: unsafe member path {name!r}')
            if names != sorted(PACKED_FILES):
                problems.append(f'{path.name}: contains {names}, expected {sorted(PACKED_FILES)}')
            for name in set(names) & set(PACKED_FILES):
                if zf.read(name) != (directory / name).read_bytes():
                    problems.append(f'{path.name}: {name} differs from the source file')
            if 'metadata.json' in names:
                json.loads(zf.read('metadata.json'))
    except (OSError, zipfile.BadZipFile, json.JSONDecodeError) as exc:
        problems.append(f'{path}: {exc}')
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('directory', type=pathlib.Path, help='extension directory (named after its uuid)')
    parser.add_argument('--zip', type=pathlib.Path, help='write and verify the extension zip here')
    args = parser.parse_args(argv)

    directory = args.directory.resolve()
    if not directory.is_dir():
        print(f'error: {args.directory} is not a directory', file=sys.stderr)
        return 2
    meta, problems = check_metadata(directory)
    problems += check_files(directory)
    if args.zip and not problems:
        problems += pack(directory, args.zip)
    for p in problems:
        print(p)
    if problems:
        return 1
    versions = ', '.join(meta['shell-version'])
    print(f'{meta["uuid"]}: metadata OK (GNOME {versions})'
          + (f'; packed {args.zip}' if args.zip else ''))
    return 0


if __name__ == '__main__':
    sys.exit(main())
