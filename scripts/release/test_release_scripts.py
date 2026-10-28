"""Unit tests for scripts/release/*.py (stdlib unittest).

    python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import base64
import contextlib
import io
import json
import os
import pathlib
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import bump_version  # noqa: E402
import check_landing  # noqa: E402
import make_latest_json  # noqa: E402

REPO = HERE.parents[1]


def quiet(func, *args, **kwargs):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = func(*args, **kwargs)
    return code, out.getvalue(), err.getvalue()


# --- bump_version -----------------------------------------------------------

FIXTURE_FILES = {
    'desktop/package.json': '{\n  "name": "scribiva-desktop",\n  "private": true,\n'
                            '  "version": "1.0.0",\n  "dependencies": {\n'
                            '    "react": "19.3.1"\n  }\n}\n',
    'desktop/package-lock.json': '{\n  "name": "scribiva-desktop",\n  "version": "1.0.0",\n'
                                 '  "lockfileVersion": 3,\n  "packages": {\n    "": {\n'
                                 '      "name": "scribiva-desktop",\n      "version": "1.0.0",\n'
                                 '      "dependencies": {}\n    },\n'
                                 '    "node_modules/x": {\n      "version": "1.0.0"\n    }\n'
                                 '  }\n}\n',
    'desktop/src-tauri/Cargo.toml': '[package]\nname = "scribiva-desktop"\nversion = "1.0.0"\n'
                                    'edition = "2021"\n\n[dependencies]\n'
                                    'serde = { version = "=1.0.0" }\n',
    'desktop/src-tauri/Cargo.lock': '[[package]]\nname = "other"\nversion = "1.0.0"\n\n'
                                    '[[package]]\nname = "scribiva-desktop"\nversion = "1.0.0"\n'
                                    'dependencies = [\n "other",\n]\n',
    'desktop/src-tauri/tauri.conf.json': '{\n  "productName": "Scribiva",\n  "version": "1.0.0",\n'
                                         '  "plugins": {\n    "x": {\n      "version": "1.0.0"\n'
                                         '    }\n  }\n}\n',
    'src/scribiva/__init__.py': '"""Scribiva."""\n__version__ = \'1.0.0\'\n',
    'server/scribiva_server/__init__.py': '"""API."""\n__version__ = \'1.0.0\'\n',
}


class BumpVersionTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        for rel, text in FIXTURE_FILES.items():
            path = self.root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding='utf-8')

    def tearDown(self):
        self.tmp.cleanup()

    def read(self, rel):
        return (self.root / rel).read_text(encoding='utf-8')

    def test_semver_validation(self):
        for good in ('1.0.1', 'v1.0.1', '0.0.0', '2.10.3-beta.1', '1.0.0-rc.1+build.5'):
            self.assertTrue(bump_version.parse_version(good))
        self.assertEqual(bump_version.parse_version('v1.2.3'), '1.2.3')
        for bad in ('1.0', '01.0.0', '1.0.0.0', '1.0.0-', 'latest', '', '1.0.0 ', 'v'):
            if bad == '1.0.0 ':
                continue  # surrounding whitespace is stripped on purpose
            with self.assertRaises(bump_version.VersionError, msg=bad):
                bump_version.parse_version(bad)

    def test_rejects_non_semver_without_writing(self):
        before = {rel: self.read(rel) for rel in FIXTURE_FILES}
        code, _, err = quiet(bump_version.main, ['1.0', '--root', str(self.root)])
        self.assertEqual(code, 2)
        self.assertIn('SemVer', err)
        self.assertEqual(before, {rel: self.read(rel) for rel in FIXTURE_FILES})

    def test_dry_run_lists_every_file_and_writes_nothing(self):
        before = {rel: self.read(rel) for rel in FIXTURE_FILES}
        code, out, _ = quiet(bump_version.main, ['--dry-run', '1.0.1', '--root', str(self.root)])
        self.assertEqual(code, 0)
        for rel in FIXTURE_FILES:
            self.assertIn(rel, out)
        self.assertIn('8 of 8', out)
        self.assertEqual(before, {rel: self.read(rel) for rel in FIXTURE_FILES})

    def test_bump_changes_only_the_project_version(self):
        code, _, _ = quiet(bump_version.main, ['v1.0.1', '--root', str(self.root)])
        self.assertEqual(code, 0)
        pkg = json.loads(self.read('desktop/package.json'))
        self.assertEqual(pkg['version'], '1.0.1')
        self.assertEqual(pkg['dependencies']['react'], '19.3.1')
        lock = json.loads(self.read('desktop/package-lock.json'))
        self.assertEqual(lock['version'], '1.0.1')
        self.assertEqual(lock['packages']['']['version'], '1.0.1')
        self.assertEqual(lock['packages']['node_modules/x']['version'], '1.0.0')
        conf = json.loads(self.read('desktop/src-tauri/tauri.conf.json'))
        self.assertEqual(conf['version'], '1.0.1')
        self.assertEqual(conf['plugins']['x']['version'], '1.0.0')
        cargo = self.read('desktop/src-tauri/Cargo.toml')
        self.assertIn('version = "1.0.1"\nedition', cargo)
        self.assertIn('version = "=1.0.0"', cargo)
        cargo_lock = self.read('desktop/src-tauri/Cargo.lock')
        self.assertIn('name = "other"\nversion = "1.0.0"', cargo_lock)
        self.assertIn('name = "scribiva-desktop"\nversion = "1.0.1"', cargo_lock)
        self.assertIn("__version__ = '1.0.1'", self.read('src/scribiva/__init__.py'))
        self.assertIn("__version__ = '1.0.1'", self.read('server/scribiva_server/__init__.py'))
        code, out, _ = quiet(bump_version.main, ['--check', 'v1.0.1', '--root', str(self.root)])
        self.assertEqual(code, 0, out)

    def test_check_reports_mismatch(self):
        code, _, err = quiet(bump_version.main, ['--check', '2.0.0', '--root', str(self.root)])
        self.assertEqual(code, 1)
        self.assertIn('desktop/package.json', err)

    def test_missing_or_ambiguous_field_writes_nothing(self):
        conf = self.root / 'desktop/src-tauri/tauri.conf.json'
        conf.write_text('{\n  "productName": "Scribiva"\n}\n', encoding='utf-8')
        before = self.read('desktop/package.json')
        code, _, err = quiet(bump_version.main, ['1.0.1', '--root', str(self.root)])
        self.assertEqual(code, 2)
        self.assertIn('tauri.conf.json', err)
        self.assertEqual(before, self.read('desktop/package.json'))

    def test_real_repository_layout_matches(self):
        # Every pattern finds exactly one field in this checkout.
        found = bump_version.find(REPO)
        self.assertEqual(len(found), len(bump_version.TARGETS))


# --- make_latest_json -------------------------------------------------------

KEY_ID = bytes(range(1, 9))


def minisign_blob(lines):
    return base64.b64encode('\n'.join(lines).encode() + b'\n').decode()


def fake_pubkey(key_id=KEY_ID):
    raw = b'Ed' + key_id + b'\x00' * 32
    return minisign_blob(['untrusted comment: minisign public key: 0102030405060708',
                          base64.b64encode(raw).decode()])


def fake_signature(file_name, version='1.0.1', key_id=KEY_ID):
    raw = b'ED' + key_id + b'\x11' * 64
    trusted = f'trusted comment: timestamp:1793212385\tfile:{file_name}'
    if version is not None:
        trusted += f'\tversion:{version}'
    return minisign_blob(['untrusted comment: signature from tauri secret key',
                          base64.b64encode(raw).decode(), trusted,
                          base64.b64encode(b'\x22' * 64).decode()])


class LatestJsonTest(unittest.TestCase):
    BASE = 'https://github.com/500wango/scribiva/releases/download/v1.0.1'

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = pathlib.Path(self.tmp.name)
        self.exe = self.dir / 'Scribiva_1.0.1_x64-setup.exe'
        self.exe.write_bytes(b'MZ installer')

    def tearDown(self):
        self.tmp.cleanup()

    def sign(self, path, **kwargs):
        sig = path.with_name(path.name + '.sig')
        sig.write_text(fake_signature(path.name, **kwargs), encoding='ascii')
        return sig

    def build(self, artifacts=None, pubkey=None, version='v1.0.1'):
        return make_latest_json.build_manifest(
            version, 'Bug fixes', '2026-10-29T00:00:00Z', self.BASE,
            artifacts or [('windows-x86_64', self.exe)], pubkey)

    def test_manifest_shape(self):
        sig = self.sign(self.exe)
        manifest = self.build(pubkey=fake_pubkey())
        self.assertEqual(manifest, {
            'version': '1.0.1',
            'notes': 'Bug fixes',
            'pub_date': '2026-10-29T00:00:00Z',
            'platforms': {'windows-x86_64': {
                'signature': sig.read_text().strip(),
                'url': self.BASE + '/Scribiva_1.0.1_x64-setup.exe',
            }},
        })

    def test_url_is_percent_encoded(self):
        exe = self.dir / 'Scribiva 1.0.1 setup.exe'
        exe.write_bytes(b'x')
        self.sign(exe)
        manifest = self.build([('windows-x86_64', exe)])
        self.assertTrue(manifest['platforms']['windows-x86_64']['url'].endswith('/Scribiva%201.0.1%20setup.exe'))

    def test_signature_from_older_cli_without_version_is_accepted(self):
        self.sign(self.exe, version=None)
        self.assertIn('windows-x86_64', self.build()['platforms'])

    def test_refuses_deb_and_rpm(self):
        deb = self.dir / 'Scribiva_1.0.1_amd64.deb'
        deb.write_bytes(b'!<arch>')
        self.sign(deb)
        with self.assertRaisesRegex(make_latest_json.ManifestError, 'APT'):
            self.build([('linux-x86_64', deb)])

    def test_refuses_missing_signature(self):
        with self.assertRaisesRegex(make_latest_json.ManifestError, 'signature not found'):
            self.build()

    def test_refuses_signature_for_other_file(self):
        sig = self.exe.with_name(self.exe.name + '.sig')
        sig.write_text(fake_signature('Scribiva_1.0.0_x64-setup.exe'), encoding='ascii')
        with self.assertRaisesRegex(make_latest_json.ManifestError, 'signs'):
            self.build()

    def test_refuses_signature_for_other_version(self):
        self.sign(self.exe, version='1.0.0')
        with self.assertRaisesRegex(make_latest_json.ManifestError, 'version 1.0.0'):
            self.build()

    def test_refuses_signature_from_other_key(self):
        self.sign(self.exe, key_id=b'\x09' * 8)
        with self.assertRaisesRegex(make_latest_json.ManifestError, 'different key'):
            self.build(pubkey=fake_pubkey())

    def test_refuses_garbage_signature(self):
        self.exe.with_name(self.exe.name + '.sig').write_text('not base64 !!', encoding='ascii')
        with self.assertRaises(make_latest_json.ManifestError):
            self.build()

    def test_refuses_bad_version_platform_and_url(self):
        self.sign(self.exe)
        with self.assertRaises(make_latest_json.ManifestError):
            self.build(version='1.0')
        with self.assertRaisesRegex(make_latest_json.ManifestError, 'platform'):
            self.build([('win64', self.exe)])
        with self.assertRaisesRegex(make_latest_json.ManifestError, 'https'):
            make_latest_json.build_manifest('1.0.1', '', '2026-10-29T00:00:00Z',
                                            'http://example.invalid', [('windows-x86_64', self.exe)])

    def test_pubkey_from_tauri_conf(self):
        conf = self.dir / 'tauri.conf.json'
        conf.write_text(json.dumps({'plugins': {'updater': {'pubkey': fake_pubkey()}}}))
        self.assertEqual(make_latest_json.parse_pubkey(make_latest_json.load_pubkey(conf)), KEY_ID)
        conf.write_text(json.dumps({'version': '1.0.1'}))
        with self.assertRaisesRegex(make_latest_json.ManifestError, 'pubkey'):
            make_latest_json.load_pubkey(conf)

    def test_cli_writes_file(self):
        self.sign(self.exe)
        notes = self.dir / 'notes.md'
        notes.write_text('First line\n\n- unsigned build\n', encoding='utf-8')
        out = self.dir / 'latest.json'
        code, _, err = quiet(make_latest_json.main, [
            '--version', 'v1.0.1', '--notes-file', str(notes), '--base-url', self.BASE,
            '--pub-date', '2026-10-29T08:00:00+08:00',
            '--artifact', f'windows-x86_64={self.exe}', '--output', str(out)])
        self.assertEqual(code, 0, err)
        data = json.loads(out.read_text(encoding='utf-8'))
        self.assertEqual(data['notes'], 'First line\n\n- unsigned build')
        self.assertEqual(data['pub_date'], '2026-10-29T08:00:00+08:00')
        self.assertEqual(list(data['platforms']), ['windows-x86_64'])

    def test_cli_rejects_naive_pub_date(self):
        self.sign(self.exe)
        code, _, err = quiet(make_latest_json.main, [
            '--version', '1.0.1', '--base-url', self.BASE, '--pub-date', '2026-10-29T08:00:00',
            '--artifact', f'windows-x86_64={self.exe}'])
        self.assertEqual(code, 2)
        self.assertIn('time zone', err)

    def test_default_pub_date_is_rfc3339_utc(self):
        stamp = make_latest_json.rfc3339_now()
        self.assertRegex(stamp, r'^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$')


# --- check_landing ----------------------------------------------------------

GOOD_PAGE = '''<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Test</title>
<link rel="stylesheet" href="assets/style.css">
<script src="assets/main.js" defer></script>
</head>
<body>
<a href="#main">skip</a>
<main id="main">
<p>Text <a href="sub/">sub</a> <a href="sub/#top">top</a> <a href="https://example.com/">ok</a>
<a href="mailto:support@example.com">mail</a></p>
<img src="assets/logo.svg" alt="">
<svg viewBox="0 0 1 1"><path d="M0 0"/></svg>
</main>
</body>
</html>
'''


class LandingTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        (self.root / 'assets').mkdir()
        (self.root / 'sub').mkdir()
        (self.root / 'assets/style.css').write_text('body{background:url("logo.svg")}\n')
        (self.root / 'assets/main.js').write_text('')
        (self.root / 'assets/logo.svg').write_text('<svg/>')
        (self.root / 'index.html').write_text(GOOD_PAGE)
        (self.root / 'sub/index.html').write_text(
            GOOD_PAGE.replace('assets/', '../assets/')
            .replace('href="sub/#top"', 'href="../#main"').replace('href="sub/"', 'href="../"')
            .replace('<main id="main">', '<main id="main"><h1 id="top">x</h1>'))

    def tearDown(self):
        self.tmp.cleanup()

    def problems(self):
        return check_landing.check_site(self.root)

    def edit(self, old, new, rel='index.html'):
        path = self.root / rel
        text = path.read_text()
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1))

    def test_clean_site(self):
        self.assertEqual(self.problems(), [])

    def test_external_script_style_image_iframe(self):
        for old, new in [
            ('src="assets/main.js"', 'src="https://cdn.example.com/x.js"'),
            ('href="assets/style.css"', 'href="//fonts.example.com/css"'),
            ('src="assets/logo.svg"', 'src="http://img.example.com/a.png"'),
            ('</main>', '<iframe src="https://www.example.com/embed"></iframe></main>'),
        ]:
            with self.subTest(new=new):
                self.setUp()
                self.edit(old, new)
                problems = self.problems()
                self.assertTrue(any('external URL' in p for p in problems), problems)

    def test_external_css_url(self):
        (self.root / 'assets/style.css').write_text("@import 'https://fonts.example.com/a.css';\n")
        self.assertTrue(any('external URL' in p for p in self.problems()))

    def test_broken_relative_link_and_fragment(self):
        self.edit('href="sub/"', 'href="missing.html"')
        self.edit('href="sub/#top"', 'href="sub/#nope"')
        self.edit('href="#main"', 'href="#gone"')
        problems = '\n'.join(self.problems())
        self.assertIn('missing.html', problems)
        self.assertIn('id="nope"', problems)
        self.assertIn('id="gone"', problems)

    def test_root_relative_and_escaping_links(self):
        self.edit('href="sub/"', 'href="/privacy.html"')
        self.edit('href="sub/#top"', 'href="../../etc/passwd"')
        problems = '\n'.join(self.problems())
        self.assertIn('root-relative', problems)
        self.assertIn('outside', problems)

    def test_malformed_html(self):
        self.edit('<p>Text', '<div><p>Text')
        self.edit('</main>', '</span></main>')
        self.edit('<title>Test</title>', '<title>Test</title><title>Again</title>')
        self.edit('<main id="main">', '<main id="main"><div id="main"></div><br/><div/>')
        problems = '\n'.join(self.problems())
        self.assertIn('</main> closes <div>', problems)
        self.assertIn('stray </span>', problems)
        self.assertIn('one <title>', problems)
        self.assertIn('duplicate id="main"', problems)
        self.assertIn('<div/> is not a void element', problems)

    def test_unclosed_at_end_of_file(self):
        self.edit('</body>\n</html>\n', '')
        problems = '\n'.join(self.problems())
        self.assertIn('<body> is never closed', problems)
        self.assertIn('<html> is never closed', problems)

    def test_missing_doctype_and_lang(self):
        self.edit('<!DOCTYPE html>\n', '')
        self.edit('<html lang="en">', '<html>')
        problems = '\n'.join(self.problems())
        self.assertIn('DOCTYPE', problems)
        self.assertIn('lang', problems)

    def test_cli_exit_codes(self):
        self.assertEqual(quiet(check_landing.main, [str(self.root)])[0], 0)
        self.edit('src="assets/main.js"', 'src="https://cdn.example.com/x.js"')
        self.assertEqual(quiet(check_landing.main, [str(self.root)])[0], 1)
        self.assertEqual(quiet(check_landing.main, [])[0], 2)

    @unittest.skipUnless((REPO / 'landing/index.html').is_file(), 'no landing/ in this checkout')
    def test_repository_landing_page(self):
        self.assertEqual(check_landing.check_site(REPO / 'landing'), [])


if __name__ == '__main__':
    os.chdir(REPO)
    unittest.main()
