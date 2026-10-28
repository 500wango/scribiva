"""Shared helpers for offline server tests."""
from __future__ import annotations
import array
import dataclasses
import http.client
import io
import json
import math
import threading
import wave
from pathlib import Path

from scribiva_server import auth, db
from scribiva_server.config import Settings
from scribiva_server.http import create_server
from scribiva_server.provider import ProviderError
from scribiva_server.service import DictationService


def wav_bytes(seconds: float = 1.5, rate: int = 16000, amplitude: int = 6000) -> bytes:
    buffer = io.BytesIO()
    with wave.open(buffer, 'wb') as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(rate)
        samples = array.array('h', (int(math.sin(i / 20) * amplitude)
                                    for i in range(int(rate * seconds))))
        wav.writeframes(samples.tobytes())
    return buffer.getvalue()


def multipart(audio: bytes, fields: dict[str, str], filename: str = 'audio.wav') -> tuple[bytes, str]:
    boundary = '----scribivatest' + 'a' * 12
    chunks = []
    for name, value in fields.items():
        chunks.append(
            f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n'
            f'{value}\r\n'.encode('utf-8'))
    chunks.append(
        f'--{boundary}\r\nContent-Disposition: form-data; name="audio"; '
        f'filename="{filename}"\r\nContent-Type: audio/wav\r\n\r\n'.encode('utf-8'))
    chunks.append(audio)
    chunks.append(f'\r\n--{boundary}--\r\n'.encode('utf-8'))
    return b''.join(chunks), f'multipart/form-data; boundary={boundary}'


class FakeProvider:
    available = True

    def __init__(self):
        self.transcribe_calls = []
        self.correct_calls = []
        self.transcribe_error: Exception | None = None
        self.correct_error: Exception | None = None
        self.transcribe_result = 'hello world'
        self.correct_result = ('hello world, edited.', None)
        self.block: threading.Event | None = None
        self.started = threading.Event()

    def transcribe(self, data, *, filename, content_type, language, timeout=None):
        self.transcribe_calls.append({'bytes': len(data), 'filename': filename,
                                      'content_type': content_type,
                                      'language': language})
        self.started.set()
        if self.block is not None:
            if not self.block.wait(timeout=5):
                raise ProviderError('PROVIDER_TIMEOUT', 'blocked test provider',
                                    status=504, retryable=True)
        if self.transcribe_error:
            raise self.transcribe_error
        return self.transcribe_result

    def correct(self, text, *, mode, language):
        self.correct_calls.append({'text': text, 'mode': mode, 'language': language})
        if self.correct_error:
            raise self.correct_error
        return self.correct_result


def make_settings(root: Path, **overrides) -> Settings:
    base = Settings(
        db_path=root / 'test.sqlite3',
        host='127.0.0.1',
        port=0,
        groq_api_key='gsk_test_server_key',
        free_seconds=1800,
        pro_seconds=36000,
        result_ttl_seconds=900,
        rate_limit_per_minute=100,
        stripe_webhook_secret='whsec_test',
        public_base_url='https://example.test',
        stripe_secret_key='sk_test_fake',
        stripe_price_monthly='price_monthly',
        stripe_price_annual='price_annual',
    )
    if overrides:
        base = dataclasses.replace(base, **overrides)
    return base


class ServerTestCase:
    """Mixin: starts one HTTP server per test with an isolated SQLite database."""

    def start_server(self, **overrides):
        import tempfile
        from pathlib import Path as _Path
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = _Path(self.temp.name)
        self.settings = make_settings(root, **overrides)
        self.provider = FakeProvider()
        self.service = DictationService(self.settings, self.provider)
        self.server = create_server(self.settings, self.service)
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       kwargs={'poll_interval': 0.05}, daemon=True)
        self.thread.start()
        self.addCleanup(self._stop)
        self.port = self.server.server_port
        conn = db.connect(self.settings.db_path)
        try:
            self.user = db.create_user(conn, 'tester@example.test')
            self.token = auth.issue_token(conn, self.user['id'], 30)
        finally:
            conn.close()
        return self

    def _stop(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)

    def request(self, method, path, *, body=None, headers=None, token=True):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=15)
        sent = dict(headers or {})
        if token and self.token:
            sent.setdefault('Authorization', 'Bearer ' + self.token)
        if body is not None and not isinstance(body, bytes):
            body = json.dumps(body).encode('utf-8')
            sent.setdefault('Content-Type', 'application/json')
        try:
            connection.request(method, path, body=body, headers=sent)
            response = connection.getresponse()
            raw = response.read()
            parsed = None
            if raw:
                try:
                    parsed = json.loads(raw)
                except ValueError:
                    parsed = raw.decode('utf-8', errors='replace')
            return response.status, parsed, dict(response.headers)
        finally:
            connection.close()

    def dictation(self, *, audio=None, fields=None, key='job-abcdefgh',
                  headers=None):
        payload, content_type = multipart(
            audio if audio is not None else wav_bytes(),
            fields if fields is not None else {
                'language': 'en', 'grammar_mode': 'standard',
                'client_job_id': key, 'client_version': 'test'},
        )
        sent = {'Content-Type': content_type, 'Idempotency-Key': key}
        if headers:
            sent.update(headers)
        return self.request('POST', '/v1/dictations', body=payload, headers=sent)
