"""Desktop integration (scribiva setup) against an in-memory fake gsettings."""
from __future__ import annotations
import ast
import subprocess
import unittest
from unittest.mock import patch

from scribiva import setup


class FakeGsettings:
    def __init__(self, store: dict):
        self.store = store

    def __call__(self, cmd, **kwargs):
        name = cmd[0]
        if name == 'gnome-extensions':
            return subprocess.CompletedProcess(cmd, 1, '', 'not found')
        action, *rest = cmd[1:]
        if action == 'get':
            key = (rest[0], rest[1])
            value = self.store.get(key)
            if value is None:
                value = "@as []" if rest[1].endswith(('keybindings', 'extensions')) else "''"
            return subprocess.CompletedProcess(cmd, 0, value + '\n', '')
        if action == 'set':
            value = rest[2]
            stored = value if value.startswith(('[', '@as')) else repr(value)
            self.store[(rest[0], rest[1])] = stored
            return subprocess.CompletedProcess(cmd, 0, '', '')
        if action == 'reset':
            self.store.pop((rest[0], rest[1]), None)
            return subprocess.CompletedProcess(cmd, 0, '', '')
        raise AssertionError(cmd)


def item(path):
    return setup.SCHEMA + '.custom-keybinding:' + path


class SetupTests(unittest.TestCase):
    def run_with(self, store, fn, *args):
        fake = FakeGsettings(store)
        with patch.object(setup.subprocess, 'run', side_effect=fake), \
             patch.object(setup.shutil, 'which', return_value='/usr/bin/x'), \
             patch.object(setup, 'toggle_command', return_value='/usr/bin/scribiva-cli toggle'), \
             patch('scribiva.focus.extension_available', return_value=False):
            return fn(*args)

    def test_bind_preserves_other_shortcuts_and_removes_legacy(self):
        other = setup.BASE + 'custom0/'
        legacy = setup.LEGACY_PATHS[0]
        store = {
            (setup.SCHEMA, 'custom-keybindings'): repr([other, legacy]),
            (item(other), 'binding'): "'<Super>t'",
            (item(other), 'name'): "'Terminal'",
            (item(legacy), 'binding'): "'<Primary><Alt>v'",
        }
        result = self.run_with(store, setup.bind_hotkey)
        self.assertEqual(result['binding'], '<Primary><Alt>v')
        paths = ast.literal_eval(store[(setup.SCHEMA, 'custom-keybindings')])
        self.assertEqual(paths, [other, setup.OUR_PATH])
        self.assertEqual(store[(item(setup.OUR_PATH), 'command')],
                         repr('/usr/bin/scribiva-cli toggle'))
        self.assertNotIn((item(legacy), 'binding'), store)
        self.assertEqual(store[(item(other), 'binding')], "'<Super>t'")

    def test_bind_refuses_binding_owned_by_another_shortcut(self):
        other = setup.BASE + 'custom0/'
        store = {
            (setup.SCHEMA, 'custom-keybindings'): repr([other]),
            (item(other), 'binding'): "'<Primary><Alt>v'",
            (item(other), 'name'): "'Something else'",
        }
        with self.assertRaises(RuntimeError) as ctx:
            self.run_with(store, setup.bind_hotkey)
        self.assertIn('Something else', str(ctx.exception))
        self.assertEqual(ast.literal_eval(store[(setup.SCHEMA, 'custom-keybindings')]), [other])

    def test_repair_replaces_broken_gate_shortcut_keeping_binding(self):
        gate = setup.BASE + 'scribiva-gate1/'
        store = {
            (setup.SCHEMA, 'custom-keybindings'): repr([gate]),
            (item(gate), 'name'): "'Scribiva Test'",
            (item(gate), 'command'): "'/home/u/.local/bin/scribiva-insert-test'",
            (item(gate), 'binding'): "'<Primary><Alt>v'",
        }
        with patch.object(setup, '_command_target_exists', return_value=False):
            result = self.run_with(store, setup.repair_hotkey)
        self.assertEqual(result['binding'], '<Primary><Alt>v')
        self.assertEqual(result['replaced'], [gate])
        self.assertEqual(ast.literal_eval(store[(setup.SCHEMA, 'custom-keybindings')]),
                         [setup.OUR_PATH])
        self.assertNotIn((item(gate), 'command'), store)

    def test_repair_does_nothing_without_a_scribiva_shortcut(self):
        other = setup.BASE + 'custom0/'
        store = {(setup.SCHEMA, 'custom-keybindings'): repr([other]),
                 (item(other), 'command'): "'gnome-terminal'"}
        self.assertIsNone(self.run_with(store, setup.repair_hotkey))
        self.assertEqual(ast.literal_eval(store[(setup.SCHEMA, 'custom-keybindings')]), [other])

    def test_enable_extension_persists_choice_and_asks_for_relogin(self):
        store = {('org.gnome.shell', 'enabled-extensions'): "['ubuntu-dock@ubuntu.com']",
                 ('org.gnome.shell', 'disabled-extensions'): "['scribiva@scribiva.app']"}
        with patch.object(setup.os.path, 'isfile', return_value=True):
            result = self.run_with(store, setup.enable_extension)
        self.assertTrue(result['enabled'])
        self.assertTrue(result['needs_relogin'])
        self.assertIn('scribiva@scribiva.app',
                      ast.literal_eval(store[('org.gnome.shell', 'enabled-extensions')]))
        self.assertEqual(ast.literal_eval(store[('org.gnome.shell', 'disabled-extensions')]), [])

    def test_enable_requires_installed_extension(self):
        with patch.object(setup.os.path, 'isfile', return_value=False):
            with self.assertRaises(RuntimeError):
                self.run_with({}, setup.enable_extension)


if __name__ == '__main__':
    unittest.main()
