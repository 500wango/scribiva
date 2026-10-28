import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from scribiva import config, devices

PW_DUMP = [
    {'id': 0, 'type': 'PipeWire:Interface:Core'},
    {
        'id': 36,
        'type': 'PipeWire:Interface:Metadata',
        'props': {'metadata.name': 'default'},
        'metadata': [
            {'key': 'default.configured.audio.source',
             'value': {'name': 'alsa_input.usb'}},
            {'key': 'default.audio.source',
             'value': {'name': 'alsa_input.builtin'}},
        ],
    },
    {
        'id': 54,
        'type': 'PipeWire:Interface:Node',
        'info': {'props': {
            'media.class': 'Audio/Source',
            'node.name': 'alsa_input.builtin',
            'node.description': 'Built-in Microphone',
            'priority.session': '2009',
        }},
    },
    {
        'id': 63,
        'type': 'PipeWire:Interface:Node',
        'info': {'props': {
            'media.class': 'Audio/Source',
            'node.name': 'NoiseTorch Microphone',
            'node.description': 'NoiseTorch Microphone',
            'node.virtual': 'true',
        }},
    },
    {
        'id': 64,
        'type': 'PipeWire:Interface:Node',
        'info': {'props': {
            'media.class': 'Audio/Sink',
            'node.name': 'alsa_output.builtin',
        }},
    },
    {
        'id': 65,
        'type': 'PipeWire:Interface:Node',
        'info': {'props': {
            'media.class': 'Audio/Source',
            'node.name': 'NoiseTorch Microphone',
            'node.description': 'NoiseTorch duplicate',
            'node.virtual': 'true',
        }},
    },
    {
        'id': 66,
        'type': 'PipeWire:Interface:Node',
        'info': {'props': {
            'media.class': 'Audio/Source',
            'node.name': 'alsa_input.usb',
            'node.nick': 'USB Mic',
            'priority.session': 100,
        }},
    },
]


class ParseTests(unittest.TestCase):
    def test_sources_default_and_dedupe(self):
        result = devices.parse_pw_dump(PW_DUMP)
        nodes = [d['node'] for d in result['devices']]
        self.assertEqual(result['default'], 'alsa_input.builtin')
        self.assertEqual(nodes, ['alsa_input.builtin', 'alsa_input.usb', 'NoiseTorch Microphone'])
        self.assertTrue(result['devices'][0]['default'])
        self.assertFalse(result['devices'][1]['default'])
        # Duplicate virtual entries collapse; the physical/default node wins ordering.
        noise = next(d for d in result['devices'] if d['node'] == 'NoiseTorch Microphone')
        self.assertTrue(noise['virtual'])
        self.assertEqual(noise['label'], 'NoiseTorch Microphone')

    def test_sink_and_malformed_entries_are_ignored(self):
        result = devices.parse_pw_dump(PW_DUMP + ['nope', {'type': 'Node'}, {'info': None}])
        self.assertNotIn('alsa_output.builtin', [d['node'] for d in result['devices']])

    def test_invalid_top_level(self):
        with self.assertRaises(devices.DeviceError):
            devices.parse_pw_dump({'devices': []})

    def test_default_value_as_json_string(self):
        result = devices.parse_pw_dump([
            {'id': 1, 'type': 'PipeWire:Interface:Metadata',
             'props': {'metadata.name': 'default'},
             'metadata': [{'key': 'default.audio.source',
                           'value': '{"name": "alsa_input.json"}'}]},
            {'id': 2, 'type': 'PipeWire:Interface:Node',
             'info': {'props': {'media.class': 'Audio/Source',
                                'node.name': 'alsa_input.json'}}},
        ])
        self.assertEqual(result['default'], 'alsa_input.json')
        self.assertTrue(result['devices'][0]['default'])


class ListSourcesTests(unittest.TestCase):
    def test_falls_back_to_wpctl_when_pw_dump_missing(self):
        source = {'node': 'alsa_input.fallback', 'label': 'Fallback Mic',
                  'virtual': False, 'priority': 0, 'id': None, 'default': True}
        with patch.object(devices.shutil, 'which',
                          side_effect=lambda name: '/usr/bin/wpctl' if name == 'wpctl' else None), \
             patch.object(devices, '_wpctl_source', return_value=source):
            result = devices.list_sources()
        self.assertEqual(result['default'], 'alsa_input.fallback')
        self.assertEqual(result['devices'], [source])
        self.assertIn('pw-dump', result['error'])
        self.assertEqual(result['source'], 'wpctl')

    def test_pw_dump_failure_is_reported(self):
        class Failed:
            returncode = 1
            stdout = ''
            stderr = 'connection refused'
        with patch.object(devices.shutil, 'which', return_value='/usr/bin/tool'), \
             patch.object(devices, '_run', return_value=Failed()), \
             patch.object(devices, '_wpctl_source', return_value=None):
            result = devices.list_sources()
        self.assertIn('connection refused', result['error'])
        self.assertIsNone(result['default'])


class MicResolutionTests(unittest.TestCase):
    def test_explicit_setting_wins(self):
        conf = {'mic_node': 'alsa_input.explicit'}
        self.assertEqual(config.mic_node(conf, {'default': 'alsa_input.default'}), 'alsa_input.explicit')

    def test_legacy_file_then_system_default(self):
        with tempfile.TemporaryDirectory() as temp:
            legacy = Path(temp) / 'mic-node'
            with patch.object(config, 'MIC_PATH', legacy):
                legacy.write_text('alsa_input.legacy\n', encoding='utf-8')
                self.assertEqual(config.mic_node({'mic_node': ''}, {'default': 'alsa_input.default'}),
                                 'alsa_input.legacy')
                legacy.unlink()
                self.assertEqual(config.mic_node({'mic_node': ''}, {'default': 'alsa_input.default'}),
                                 'alsa_input.default')
                with self.assertRaises(RuntimeError):
                    config.mic_node({'mic_node': ''}, {'default': None})

    def test_clear_mic_node_removes_legacy_file(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            settings = root / 'settings.json'
            legacy = root / 'mic-node'
            legacy.write_text('alsa_input.old', encoding='utf-8')
            with patch.object(config, 'CONFIG_PATH', settings), \
                 patch.object(config, 'MIC_PATH', legacy), \
                 patch.object(config, 'secure_dirs'):
                config.save_config({'mic_node': 'alsa_input.old'})
                result = config.clear_mic_node()
            self.assertEqual(result['mic_node'], '')
            self.assertFalse(legacy.exists())
            self.assertEqual(json.loads(settings.read_text(encoding='utf-8'))['mic_node'], '')


if __name__ == '__main__':
    unittest.main()
