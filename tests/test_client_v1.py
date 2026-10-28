"""v1.0 Linux client: focus parsing, silence trimming, FLAC upload, and the
real curl client against a loopback server for device login and dictionary."""
from __future__ import annotations
import array
import email
import math
import os
import re
import shutil
import sys
import tempfile
import threading
import unittest
import wave
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))

from scribiva import audio, focus, providers  # noqa: E402
from server_support import ServerTestCase  # noqa: E402

CONFIG = {
    'language': 'en', 'mode': 'standard', 'asr_model': 'whisper-large-v3-turbo',
    'grammar_model': 'openai/gpt-oss-20b', 'auto_insert': True,
    'restore_text_clipboard': False, 'mic_node': '', 'max_record_seconds': 180,
    'asr_prompt': '', 'proxy_url': '', 'service_url': '', 'delete_audio_on_success': True,
    'output_script': 'hans', 'dictation_commands': True, 'insert_without_focus_check': False,
}


def write_wav(path: Path, parts: list[tuple[float, int]], rate: int = 16000) -> None:
    samples = array.array('h')
    for seconds, amplitude in parts:
        samples.extend(int(math.sin(i / 9) * amplitude) for i in range(int(rate * seconds)))
    with wave.open(str(path), 'wb') as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(rate)
        wav.writeframes(samples.tobytes())


class FocusTests(unittest.TestCase):
    def test_parse_gdbus_output(self):
        out = "('org.gnome.Ptyxis.desktop', 'org.gnome.Ptyxis', uint64 5000000001, uint32 4242)"
        parsed = focus.parse_gdbus_focus(out)
        self.assertEqual(parsed, {'source': 'gnome-extension', 'app_id': 'org.gnome.Ptyxis.desktop',
                                  'wm_class': 'org.gnome.Ptyxis', 'window_id': 5000000001,
                                  'pid': 4242})
        self.assertTrue(focus.is_terminal(parsed))
        self.assertIsNone(focus.parse_gdbus_focus("('', '', uint64 0, uint32 0)"))
        self.assertIsNone(focus.parse_gdbus_focus('garbage'))

    def test_same_target_and_terminal_detection(self):
        editor = {'source': 'x11', 'app_id': 'obsidian', 'wm_class': 'obsidian',
                  'window_id': 1, 'pid': 1}
        self.assertTrue(focus.same_target(editor, dict(editor)))
        self.assertFalse(focus.same_target(editor, {**editor, 'window_id': 2}))
        self.assertFalse(focus.same_target(editor, None))
        self.assertFalse(focus.is_terminal(editor))
        self.assertTrue(focus.is_terminal({**editor, 'wm_class': 'kitty'}))

    def test_snapshot_without_tools_is_unknown(self):
        with patch.dict(os.environ, {'XDG_SESSION_TYPE': 'wayland'}), \
             patch.object(focus.shutil, 'which', return_value=None):
            self.assertIsNone(focus.snapshot())


class AudioTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'a.wav'

    def test_trim_keeps_speech_with_padding(self):
        write_wav(self.path, [(2.0, 30), (1.0, 8000), (2.0, 30)])
        result = audio.trim_silence(self.path)
        self.assertTrue(result['trimmed'])
        self.assertGreater(result['duration'], 1.0)
        self.assertLess(result['duration'], 2.0)

    def test_trim_leaves_all_silence_untouched(self):
        write_wav(self.path, [(1.5, 20)])
        self.assertFalse(audio.trim_silence(self.path)['trimmed'])
        with wave.open(str(self.path)) as wav:
            self.assertEqual(wav.getnframes(), 24000)

    @unittest.skipUnless(shutil.which('ffmpeg'), 'ffmpeg not installed')
    def test_flac_encoding(self):
        write_wav(self.path, [(1.5, 6000)])
        flac, mime = audio.encode_flac(self.path)
        self.assertEqual(mime, 'audio/flac')
        self.assertEqual(flac.read_bytes()[:4], b'fLaC')

    def test_flac_missing_ffmpeg(self):
        write_wav(self.path, [(1.5, 6000)])
        with patch('shutil.which', return_value=None):
            self.assertIsNone(audio.encode_flac(self.path))


class ClientServerV1Tests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.outbox = tempfile.TemporaryDirectory()
        self.addCleanup(self.outbox.cleanup)
        self.start_server(mail_outbox=Path(self.outbox.name))
        self.config = {**CONFIG, 'service_url': f'http://127.0.0.1:{self.port}'}

    def _approve_by_email(self, user_code: str):
        """Act as the user in the browser: request link, open it, confirm."""
        self.request('POST', '/v1/auth/magic-link', token=False,
                     body={'email': 'linux@example.com', 'user_code': user_code})
        files = sorted(Path(self.outbox.name).glob('*.eml'))
        body = email.message_from_bytes(files[-1].read_bytes()).get_payload(decode=True)
        token = re.search(rb'token=([A-Za-z0-9_-]+)', body).group(1).decode()
        self.request('POST', '/auth/verify', token=False,
                     body=urlencode({'token': token}).encode(),
                     headers={'Content-Type': 'application/x-www-form-urlencoded'})

    def test_device_login_through_curl_client(self):
        start = providers.device_login_start(self.config)
        approved = threading.Event()

        def fake_sleep(_seconds):
            if not approved.is_set():
                self._approve_by_email(start['user_code'])
                approved.set()

        token = providers.device_login_poll(self.config, start['device_code'],
                                            interval=1, sleep=fake_sleep)
        self.assertTrue(token.startswith('scr_'))
        account = providers.service_me(self.config, token=token)
        self.assertEqual(account['user']['email'], 'linux@example.com')

    def test_login_url_must_match_service_host(self):
        with patch.object(providers, '_public_post', return_value=(
                '200', {'device_code': 'x' * 43, 'user_code': 'BCDF-GHJK',
                        'verification_uri_complete': 'http://evil.example/login'}, '')):
            with self.assertRaises(providers.ProviderError) as ctx:
                providers.device_login_start(self.config)
        self.assertEqual(ctx.exception.code, 'LOGIN_URL_INVALID')

    def test_dictionary_and_flac_dictation(self):
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
            data = providers.service_dictionary(self.config, ['Scribiva', 'Kubernetes'])
            self.assertEqual(data['terms'], ['Scribiva', 'Kubernetes'])
            self.assertEqual(providers.service_dictionary(self.config)['terms'],
                             ['Scribiva', 'Kubernetes'])
            path = Path(self.outbox.name) / 'clip.wav'
            write_wav(path, [(1.5, 6000)])
            upload, mime = path, 'audio/wav'
            encoded = audio.encode_flac(path)
            if encoded:
                upload, mime = encoded
            result = providers.service_dictation(upload, self.config,
                                                 job_id='client-v1-flac-job', mime=mime)
        self.assertEqual(result['raw_text'], 'hello world')
        self.assertFalse(result['suspected_hallucination'])
        call = self.provider.transcribe_calls[-1]
        self.assertEqual(call['prompt'], 'Scribiva, Kubernetes')
        self.assertEqual(call['content_type'], mime)


if __name__ == '__main__':
    unittest.main()
