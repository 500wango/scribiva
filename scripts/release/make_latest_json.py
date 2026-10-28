#!/usr/bin/env python3
"""Assemble the Tauri v2 static updater manifest (latest.json).

    python3 scripts/release/make_latest_json.py \
        --version v1.0.1 \
        --notes-file notes.md \
        --base-url https://github.com/500wango/scribiva/releases/download/v1.0.1 \
        --artifact windows-x86_64=dist/Scribiva_1.0.1_x64-setup.exe \
        [--pubkey-file desktop/src-tauri/tauri.conf.json] \
        --output dist/latest.json

For every --artifact PLATFORM=FILE the updater signature FILE.sig (written by
`tauri build` with bundle.createUpdaterArtifacts) is read and checked before it
goes into the manifest:

- it is a base64 minisign signature as produced by `tauri signer`;
- its trusted comment names this file and, when present, this version
  (Tauri >= 2.12 records the version; a manifest that pairs one release's
  version with another release's file is refused);
- with --pubkey-file, it was made by the key whose public half the app embeds
  (key id comparison; the Ed25519 check itself happens in the app).

Linux packages (.deb/.rpm) are installed and updated through the website or
APT, never by the in-app updater, so they are refused here. Stdlib only.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import datetime
import json
import pathlib
import re
import sys
import urllib.parse

SEMVER = re.compile(
    r'^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)'
    r'(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)'
    r'(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?'
    r'(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$'
)
# Tauri v2 target keys: OS-ARCH, optionally with an installer suffix
# (e.g. windows-x86_64-nsis) for apps that ship several formats per platform.
PLATFORM = re.compile(r'^(windows|darwin|linux)-(x86_64|aarch64|i686|armv7)(-[a-z0-9]+)?$')
NOT_AUTO_UPDATED = ('.deb', '.rpm')


class ManifestError(Exception):
    pass


def parse_version(text: str) -> str:
    version = text.strip()
    if version[:1] in ('v', 'V'):
        version = version[1:]
    if not SEMVER.match(version):
        raise ManifestError(f'not a SemVer 2.0.0 version: {text!r}')
    return version


def _b64(text: str, what: str) -> bytes:
    try:
        return base64.b64decode(text.strip(), validate=True)
    except (binascii.Error, ValueError) as exc:
        raise ManifestError(f'{what}: not valid base64') from exc


def _minisign_lines(blob: str, what: str) -> list[str]:
    """Tauri stores minisign files base64-encoded once more; undo that."""
    try:
        text = _b64(blob, what).decode('utf-8')
    except UnicodeDecodeError as exc:
        raise ManifestError(f'{what}: not a minisign text file') from exc
    return text.splitlines()


def parse_signature(signature: str, what: str = 'signature') -> dict:
    """Return key id and trusted-comment fields of a Tauri updater signature."""
    lines = _minisign_lines(signature, what)
    if (len(lines) < 4 or not lines[0].startswith('untrusted comment:')
            or not lines[2].startswith('trusted comment:')):
        raise ManifestError(f'{what}: not a minisign signature')
    raw = _b64(lines[1], what)
    # 2 bytes algorithm ("Ed" or prehashed "ED"), 8 bytes key id, 64 bytes signature.
    if len(raw) != 74 or raw[:2] not in (b'Ed', b'ED'):
        raise ManifestError(f'{what}: unexpected signature length or algorithm')
    fields = {}
    for part in lines[2][len('trusted comment:'):].strip().split('\t'):
        key, sep, value = part.partition(':')
        if sep:
            fields[key] = value
    return {'key_id': raw[2:10], 'trusted': fields}


def parse_pubkey(pubkey: str, what: str = 'public key') -> bytes:
    """Return the 8-byte key id of a Tauri updater public key."""
    lines = _minisign_lines(pubkey, what)
    if len(lines) < 2 or not lines[0].startswith('untrusted comment:'):
        raise ManifestError(f'{what}: not a minisign public key')
    raw = _b64(lines[1], what)
    if len(raw) != 42 or raw[:2] != b'Ed':
        raise ManifestError(f'{what}: unexpected public key length or algorithm')
    return raw[2:10]


def load_pubkey(path: pathlib.Path) -> str:
    """Read the key from a .pub file or from tauri.conf.json plugins.updater.pubkey."""
    text = path.read_text(encoding='utf-8')
    if path.suffix == '.json':
        try:
            key = json.loads(text)['plugins']['updater']['pubkey']
        except (KeyError, TypeError, json.JSONDecodeError) as exc:
            raise ManifestError(f'{path}: no plugins.updater.pubkey') from exc
        if not isinstance(key, str) or not key.strip():
            raise ManifestError(f'{path}: plugins.updater.pubkey is empty')
        return key
    return text


def platform_entry(platform: str, artifact: pathlib.Path, base_url: str,
                   version: str, key_id: bytes | None) -> dict:
    if not PLATFORM.match(platform):
        raise ManifestError(f'unknown updater platform {platform!r} (expected e.g. windows-x86_64)')
    if artifact.name.endswith(NOT_AUTO_UPDATED):
        raise ManifestError(
            f'{artifact.name}: Linux packages are updated through the website/APT, '
            'not the in-app updater')
    if not artifact.is_file():
        raise ManifestError(f'{artifact}: file not found')
    sig_path = artifact.with_name(artifact.name + '.sig')
    if not sig_path.is_file():
        raise ManifestError(f'{sig_path}: updater signature not found '
                            '(build with bundle.createUpdaterArtifacts and TAURI_SIGNING_PRIVATE_KEY)')
    signature = sig_path.read_text(encoding='ascii').strip()
    info = parse_signature(signature, sig_path.name)
    signed_file = info['trusted'].get('file')
    if signed_file != artifact.name:
        raise ManifestError(f'{sig_path.name}: signs {signed_file!r}, not {artifact.name!r}')
    signed_version = info['trusted'].get('version')
    if signed_version is not None and signed_version != version:
        raise ManifestError(f'{sig_path.name}: signed for version {signed_version}, not {version}')
    if key_id is not None and info['key_id'] != key_id:
        raise ManifestError(f'{sig_path.name}: signed with a different key than the app\'s pubkey')
    url = base_url.rstrip('/') + '/' + urllib.parse.quote(artifact.name)
    if not url.startswith('https://'):
        raise ManifestError(f'download URL must be https: {url}')
    return {'signature': signature, 'url': url}


def build_manifest(version: str, notes: str, pub_date: str, base_url: str,
                   artifacts: list[tuple[str, pathlib.Path]],
                   pubkey: str | None = None) -> dict:
    version = parse_version(version)
    if not artifacts:
        raise ManifestError('at least one --artifact is required')
    key_id = parse_pubkey(pubkey) if pubkey is not None else None
    platforms = {}
    for platform, path in artifacts:
        if platform in platforms:
            raise ManifestError(f'platform {platform} given twice')
        platforms[platform] = platform_entry(platform, path, base_url, version, key_id)
    return {'version': version, 'notes': notes, 'pub_date': pub_date, 'platforms': platforms}


def rfc3339_now() -> str:
    now = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)
    return now.isoformat().replace('+00:00', 'Z')


def check_rfc3339(value: str) -> str:
    try:
        parsed = datetime.datetime.fromisoformat(value.replace('Z', '+00:00'))
    except ValueError as exc:
        raise ManifestError(f'pub_date is not RFC 3339: {value!r}') from exc
    if parsed.tzinfo is None:
        raise ManifestError(f'pub_date needs a time zone: {value!r}')
    return value


def _artifact(text: str) -> tuple[str, pathlib.Path]:
    platform, sep, path = text.partition('=')
    if not sep or not platform or not path:
        raise argparse.ArgumentTypeError('expected PLATFORM=FILE')
    return platform, pathlib.Path(path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--version', required=True)
    notes = parser.add_mutually_exclusive_group()
    notes.add_argument('--notes', default='')
    notes.add_argument('--notes-file', type=pathlib.Path)
    parser.add_argument('--pub-date', help='RFC 3339 time (default: now, UTC)')
    parser.add_argument('--base-url', required=True, help='https URL the artifacts are downloaded from')
    parser.add_argument('--artifact', action='append', type=_artifact, default=[],
                        metavar='PLATFORM=FILE')
    parser.add_argument('--pubkey-file', type=pathlib.Path,
                        help='updater public key (.pub) or tauri.conf.json to check the key id against')
    parser.add_argument('--output', '-o', type=pathlib.Path, help='write here instead of stdout')
    args = parser.parse_args(argv)

    try:
        text = args.notes_file.read_text(encoding='utf-8') if args.notes_file else args.notes
        pub_date = check_rfc3339(args.pub_date) if args.pub_date else rfc3339_now()
        pubkey = load_pubkey(args.pubkey_file) if args.pubkey_file else None
        manifest = build_manifest(args.version, text.strip(), pub_date, args.base_url,
                                  args.artifact, pubkey)
    except (ManifestError, OSError) as exc:
        print(f'error: {exc}', file=sys.stderr)
        return 2

    out = json.dumps(manifest, indent=2, ensure_ascii=False) + '\n'
    if args.output:
        args.output.write_text(out, encoding='utf-8')
        print(f'Wrote {args.output} ({", ".join(manifest["platforms"])})', file=sys.stderr)
    else:
        sys.stdout.write(out)
    return 0


if __name__ == '__main__':
    sys.exit(main())
