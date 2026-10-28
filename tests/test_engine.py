import array
import math
import os
import tempfile
import unittest
import wave
from pathlib import Path
from unittest.mock import patch
from scribiva import engine

CONFIG = {
    'language': 'zh', 'mode': 'standard', 'asr_model': 'whisper-large-v3',
    'grammar_model': 'openai/gpt-oss-20b', 'auto_insert': True,
    'restore_text_clipboard': False, 'mic_node': 'alsa_input.fake',
    'max_record_seconds': 180, 'asr_prompt': '', 'proxy_url': '',
    'service_url': '', 'delete_audio_on_success': False,
}


def write_wav(path: Path, seconds: float = 1.5):
    with wave.open(str(path), 'wb') as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(16000)
        samples = array.array('h', (int(math.sin(i / 20) * 6000)
                                    for i in range(int(16000 * seconds))))
        wav.writeframes(samples.tobytes())


class EngineJobTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.audio_dir = self.root / 'audio'
        self.audio_dir.mkdir()
        self.patches = [
            patch.object(engine, 'STATE_PATH', self.root / 'session.json'),
            patch.object(engine, 'LOCK_PATH', self.root / 'session.lock'),
            patch.object(engine, 'LATEST_PATH', self.root / 'last-result.txt'),
            patch.object(engine, 'LAST_RAW_PATH', self.root / 'last-raw.txt'),
            patch.object(engine, 'LOG_PATH', self.root / 'scribiva.log'),
            patch.object(engine, 'AUDIO_DIR', self.audio_dir),
            patch.object(engine, 'ROOT_STATE', self.root),
            patch.object(engine, 'secure_dirs'),
            patch.object(engine, 'init_logging'),
            patch.object(engine, 'notify'),
            patch.object(engine.logger, 'info'),
            patch.object(engine.logger, 'warning'),
            patch.object(engine.logger, 'exception'),
        ]
        for item in self.patches:
            item.start()
            self.addCleanup(item.stop)
        self.addCleanup(self.temp.cleanup)

    def recording(self):
        path = self.audio_dir / 'record-test.wav'
        write_wav(path)
        return path

    def test_worker_success_stores_final_and_raw(self):
        path = self.recording()
        engine.set_state({'phase': 'processing', 'id': 'job1', 'file': str(path)})
        with patch.object(engine, 'read_config', return_value=CONFIG), \
             patch.object(engine.audio, 'quality',
                          return_value={'duration': 1.5, 'rms': 900.0}), \
             patch.object(engine.providers, 'transcribe', return_value='原文内容'), \
             patch.object(engine.providers, 'correct',
                          return_value=('修正后的内容', 'Grammar correction skipped: timeout')), \
             patch.object(engine.inject, 'dispatch',
                          return_value='dispatched_unverified') as dispatch:
            engine.worker('job1')
        self.assertEqual(engine.LATEST_PATH.read_text(encoding='utf-8'), '修正后的内容')
        self.assertEqual(engine.LAST_RAW_PATH.read_text(encoding='utf-8'), '原文内容')
        state = engine.state()
        self.assertEqual(state['phase'], 'idle')
        self.assertEqual(state['last_result'], 'dispatched_unverified')
        self.assertIn('timeout', state['last_warning'])
        dispatch.assert_called_once()
        self.assertTrue(path.exists(), 'audio should be retained when delete_audio_on_success is false')

    def test_worker_uses_service_proxy_without_local_key(self):
        path = self.recording()
        service_config = {**CONFIG, 'service_url': 'http://127.0.0.1:8787'}
        engine.set_state({'phase': 'processing', 'id': 'job10', 'file': str(path)})
        with patch.object(engine, 'read_config', return_value=service_config), \
             patch.object(engine.audio, 'quality',
                          return_value={'duration': 1.5, 'rms': 900.0}), \
             patch.object(engine.providers, 'transcribe') as transcribe, \
             patch.object(engine.providers, 'correct') as correct, \
             patch.object(engine.providers, 'service_dictation',
                          return_value={'job_id': 'job_server_1',
                                        'raw_text': 'service raw',
                                        'output_text': 'Service edited.',
                                        'warning': 'service warn',
                                        'billing': {'charged_audio_seconds': 2,
                                                    'remaining_seconds': 1798}}) as service, \
             patch.object(engine.inject, 'dispatch',
                          return_value='dispatched_unverified'):
            engine.worker('job10')
        transcribe.assert_not_called()
        correct.assert_not_called()
        service.assert_called_once_with(path, service_config, job_id='job10')
        self.assertEqual(engine.LATEST_PATH.read_text(encoding='utf-8'), 'Service edited.')
        self.assertEqual(engine.LAST_RAW_PATH.read_text(encoding='utf-8'), 'service raw')
        self.assertEqual(engine.state()['last_warning'], 'service warn')

    def test_cancel_during_asr_skips_grammar_and_result(self):
        path = self.recording()

        def cancelled(*_args, **_kwargs):
            engine.set_state({'phase': 'idle', 'last_job': 'job2'})
            return '原文'

        engine.set_state({'phase': 'processing', 'id': 'job2', 'file': str(path)})
        with patch.object(engine, 'read_config', return_value=CONFIG), \
             patch.object(engine.audio, 'quality', return_value={'duration': 1.5, 'rms': 900.0}), \
             patch.object(engine.providers, 'transcribe', side_effect=cancelled), \
             patch.object(engine.providers, 'correct') as correct, \
             patch.object(engine.inject, 'dispatch') as dispatch:
            engine.worker('job2')
        correct.assert_not_called()
        dispatch.assert_not_called()
        self.assertFalse(engine.LATEST_PATH.exists())
        self.assertFalse(engine.LAST_RAW_PATH.exists())

    def test_cancel_after_grammar_drops_result(self):
        path = self.recording()

        def corrected(*_args, **_kwargs):
            engine.set_state({'phase': 'idle', 'last_job': 'job3'})
            return '修正后', None

        engine.set_state({'phase': 'processing', 'id': 'job3', 'file': str(path)})
        with patch.object(engine, 'read_config', return_value=CONFIG), \
             patch.object(engine.audio, 'quality', return_value={'duration': 1.5, 'rms': 900.0}), \
             patch.object(engine.providers, 'transcribe', return_value='原文'), \
             patch.object(engine.providers, 'correct', side_effect=corrected), \
             patch.object(engine.inject, 'dispatch') as dispatch:
            engine.worker('job3')
        dispatch.assert_not_called()
        self.assertFalse(engine.LATEST_PATH.exists())
        self.assertFalse(engine.LAST_RAW_PATH.exists())

    def test_discard_deletes_retained_error_audio(self):
        path = self.recording()
        engine.set_state({'phase': 'error', 'id': 'job4', 'error': 'boom', 'file': str(path)})
        self.assertEqual(engine.discard(), 'discarded')
        self.assertFalse(path.exists())
        self.assertEqual(engine.state()['phase'], 'idle')

    def test_cancel_refuses_to_hide_retained_error_audio(self):
        path = self.recording()
        engine.set_state({'phase': 'error', 'id': 'job5', 'error': 'boom', 'file': str(path)})
        with self.assertRaisesRegex(RuntimeError, 'discard'):
            engine.cancel()
        self.assertTrue(path.exists())
        self.assertEqual(engine.state()['phase'], 'error')

    def test_clear_last_removes_text_and_warning(self):
        engine.LATEST_PATH.write_text('final', encoding='utf-8')
        engine.LAST_RAW_PATH.write_text('raw', encoding='utf-8')
        engine.set_state({'phase': 'idle', 'last_job': 'job6', 'last_warning': 'warn'})
        self.assertTrue(engine.clear_last())
        self.assertFalse(engine.LATEST_PATH.exists())
        self.assertFalse(engine.LAST_RAW_PATH.exists())
        self.assertNotIn('last_warning', engine.state())

    def test_last_result_reports_both_variants(self):
        engine.LATEST_PATH.write_text('final', encoding='utf-8')
        engine.LAST_RAW_PATH.write_text('raw', encoding='utf-8')
        engine.set_state({'phase': 'idle', 'last_job': 'job7', 'last_warning': 'warn'})
        data = engine.last_result()
        self.assertTrue(data['available'])
        self.assertEqual(data['text'], 'final')
        self.assertEqual(data['raw'], 'raw')
        self.assertEqual(data['warning'], 'warn')

    def test_dispatch_failure_keeps_result_and_warning(self):
        path = self.recording()
        engine.set_state({'phase': 'processing', 'id': 'job9', 'file': str(path)})
        with patch.object(engine, 'read_config', return_value=CONFIG), \
             patch.object(engine.audio, 'quality', return_value={'duration': 1.5, 'rms': 900.0}), \
             patch.object(engine.providers, 'transcribe', return_value='原文'), \
             patch.object(engine.providers, 'correct', return_value=('修正后', 'warn')), \
             patch.object(engine.inject, 'dispatch',
                          side_effect=RuntimeError('Paste dispatch failed/uncertain')):
            engine.worker('job9')
        state = engine.state()
        self.assertEqual(state['phase'], 'error')
        self.assertEqual(state['last_warning'], 'warn')
        self.assertEqual(engine.LATEST_PATH.read_text(encoding='utf-8'), '修正后')
        self.assertTrue(path.exists())

    def test_toggle_stop_failure_keeps_error_state(self):
        path = self.recording()
        engine.set_state({'phase': 'recording', 'id': 'job8', 'pid': 4242,
                          'file': str(path), 'started': 1.0})
        with patch.object(engine, '_belongs_here', return_value=True), \
             patch.object(engine.audio, 'stop', side_effect=RuntimeError('process vanished')):
            with self.assertRaisesRegex(RuntimeError, 'Stopping the recorder failed'):
                engine.toggle()
        state = engine.state()
        self.assertEqual(state['phase'], 'error')
        self.assertEqual(state['file'], str(path))
        self.assertIn('process vanished', state['error'])


class DoctorTests(unittest.TestCase):
    def test_service_mode_does_not_require_local_key(self):
        service_config = {**CONFIG, 'service_url': 'https://api.example.test'}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'audio').mkdir()
            with patch.object(engine, 'STATE_PATH', root / 'missing.json'), \
                 patch.object(engine, 'ROOT_STATE', root), \
                 patch.object(engine, 'AUDIO_DIR', root / 'audio'), \
                 patch.object(engine, 'KEY_PATH', root / 'no-key'), \
                 patch.object(engine, 'SERVICE_TOKEN_PATH', root / 'no-token'), \
                 patch.object(engine, 'MIC_PATH', root / 'no-mic'), \
                 patch.object(engine, 'secure_dirs'), \
                 patch.object(engine, 'read_config', return_value=service_config), \
                 patch.object(engine, 'mic_node', return_value='alsa_input.fake'), \
                 patch.object(engine, 'service_token',
                              return_value='scr_' + 'a' * 40), \
                 patch.object(engine, 'api_key', return_value=''), \
                 patch.object(engine.devices, 'list_sources',
                              return_value={'devices': [{'node': 'alsa_input.fake',
                                                         'label': 'Fake', 'virtual': False,
                                                         'priority': 100, 'id': 1,
                                                         'default': True}],
                                            'default': 'alsa_input.fake', 'error': None}), \
                 patch.object(engine.inject, 'eitype_command', return_value='/usr/bin/eitype'), \
                 patch.object(engine.shutil, 'which', return_value='/usr/bin/tool'), \
                 patch.dict(os.environ, {'XDG_SESSION_TYPE': 'wayland',
                                         'XDG_CURRENT_DESKTOP': 'ubuntu:GNOME',
                                         'XDG_RUNTIME_DIR': temp}):
                (root / 'pipewire-0').write_text('', encoding='utf-8')
                report = engine.doctor()
        self.assertTrue(report['healthy'], report['checks'])
        self.assertTrue(report['service_mode'])
        self.assertTrue(report['has_service_token'])
        checks = {check['id']: check for check in report['checks']}
        self.assertEqual(checks['service']['status'], 'ok')
        self.assertEqual(checks['api_key']['status'], 'ok')
        self.assertIn('stays on the server', checks['api_key']['detail'])

    def test_healthy_environment(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'audio').mkdir()
            with patch.object(engine, 'STATE_PATH', root / 'missing.json'), \
                 patch.object(engine, 'ROOT_STATE', root), \
                 patch.object(engine, 'AUDIO_DIR', root / 'audio'), \
                 patch.object(engine, 'KEY_PATH', root / 'no-key'), \
                 patch.object(engine, 'MIC_PATH', root / 'no-mic'), \
                 patch.object(engine, 'secure_dirs'), \
                 patch.object(engine, 'read_config', return_value=CONFIG), \
                 patch.object(engine, 'mic_node', return_value='alsa_input.fake'), \
                 patch.object(engine.devices, 'list_sources',
                              return_value={'devices': [{'node': 'alsa_input.fake',
                                                         'label': 'Fake', 'virtual': False,
                                                         'priority': 100, 'id': 1,
                                                         'default': True}],
                                            'default': 'alsa_input.fake', 'error': None}), \
                 patch.object(engine.inject, 'eitype_command', return_value='/usr/bin/eitype'), \
                 patch.object(engine, 'api_key', return_value='gsk_test'), \
                 patch.object(engine.shutil, 'which', return_value='/usr/bin/tool'), \
                 patch.dict(os.environ, {'XDG_SESSION_TYPE': 'wayland',
                                         'XDG_CURRENT_DESKTOP': 'ubuntu:GNOME',
                                         'XDG_RUNTIME_DIR': temp}):
                (root / 'pipewire-0').write_text('', encoding='utf-8')
                report = engine.doctor()
        self.assertTrue(report['healthy'])
        self.assertEqual(report['mic_node'], 'alsa_input.fake')
        self.assertEqual(report['default_source'], 'alsa_input.fake')
        self.assertEqual(report['key_mode'], None)
        ids = {check['id'] for check in report['checks']}
        self.assertTrue({'settings', 'session', 'desktop', 'capture', 'devices',
                         'microphone', 'clipboard', 'injection', 'api_key',
                         'state_directory'} <= ids)
        self.assertFalse(any(check['status'] == 'fail' for check in report['checks']))


if __name__ == '__main__':
    unittest.main()
