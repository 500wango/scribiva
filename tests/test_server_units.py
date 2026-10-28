import hashlib
import json
import os
import tempfile
import unittest
from datetime import timedelta
from pathlib import Path

from scribiva_server import audio, auth, db, textguard
from scribiva_server.config import ConfigError, Settings
from scribiva_server.multipart import MultipartError, parse_multipart
from scribiva_server.provider import GroqProvider, ProviderError
from scribiva_server.stripe import apply_event
from server_support import multipart, wav_bytes


def raw_multipart(parts: list[tuple[str, str, bytes]], boundary='----testboundary'):
    chunks = []
    for name, filename, payload in parts:
        disposition = f'form-data; name="{name}"'
        if filename:
            disposition += f'; filename="{filename}"'
        chunks.append(f'--{boundary}\r\nContent-Disposition: {disposition}\r\n\r\n'.encode())
        chunks.append(payload)
        chunks.append(b'\r\n')
    chunks.append(f'--{boundary}--\r\n'.encode())
    return b''.join(chunks), f'multipart/form-data; boundary={boundary}'


class TextGuardTests(unittest.TestCase):
    def test_protect_and_restore_preserves_literals(self):
        text = ('Use https://example.com/a?b=1 and `rm -rf /` in ~/code/x.py, '
                'mail a@b.co, at 12:30 with 42 items. # Heading\n- item')
        protected, parts = textguard.protect(text)
        self.assertEqual(len(parts), 4)
        for token, value in parts.items():
            self.assertEqual(protected.count(token), 1)
            self.assertNotIn(value, protected)
        revised = protected.replace('Use ', 'Please use ')
        restored = textguard.restore(text, revised, parts)
        self.assertEqual(restored, 'Please use https://example.com/a?b=1 and `rm -rf /` '
                                   'in ~/code/x.py, mail a@b.co, at 12:30 with 42 items. '
                                   '# Heading\n- item')

    def test_restore_rejects_token_number_marker_and_expansion_changes(self):
        text = 'Read https://example.com now, 42 items.\n# Heading'
        protected, parts = textguard.protect(text)
        token = next(iter(parts))
        with self.assertRaises(textguard.GuardError):
            textguard.restore(text, protected.replace(token, 'gone'), parts)
        with self.assertRaises(textguard.GuardError):
            textguard.restore(text, protected.replace('42', '43'), parts)
        with self.assertRaises(textguard.GuardError):
            textguard.restore(text, protected.replace('# Heading', '## Heading'), parts)
        with self.assertRaises(textguard.GuardError):
            textguard.restore(text, 'x' * (len(text) * 4 + 1000), parts)

    def test_ensure_text(self):
        self.assertEqual(textguard.ensure_text('  hello  '), 'hello')
        for value in ('', '   ', None, 42, 'x' * 100_001):
            with self.assertRaises(textguard.GuardError):
                textguard.ensure_text(value)  # type: ignore[arg-type]


class AudioTests(unittest.TestCase):
    def test_valid_wav_is_measured_server_side(self):
        probed = audio.probe(wav_bytes(1.5), max_bytes=1_000_000, max_seconds=10)
        self.assertEqual(probed['duration_ms'], 1500)
        self.assertEqual(probed['billable_seconds'], 2)
        self.assertEqual(probed['codec'], 'wav')
        one = audio.probe(wav_bytes(1.0), max_bytes=1_000_000, max_seconds=10)
        self.assertEqual(one['billable_seconds'], 1)

    def test_rejects_oversize_long_short_and_unknown_audio(self):
        with self.assertRaises(audio.AudioError) as small:
            audio.probe(wav_bytes(1.5), max_bytes=100, max_seconds=10)
        self.assertEqual(small.exception.status, 413)
        self.assertEqual(small.exception.code, 'AUDIO_TOO_LARGE')
        with self.assertRaises(audio.AudioError) as long:
            audio.probe(wav_bytes(1.5), max_bytes=1_000_000, max_seconds=1)
        self.assertEqual(long.exception.status, 422)
        with self.assertRaises(audio.AudioError) as short:
            audio.probe(wav_bytes(0.1), max_bytes=1_000_000, max_seconds=10)
        self.assertEqual(short.exception.code, 'AUDIO_DURATION_INVALID')
        with self.assertRaises(audio.AudioError) as unknown:
            audio.probe(b'OggS not supported', max_bytes=1_000_000, max_seconds=10)
        self.assertEqual(unknown.exception.code, 'INVALID_AUDIO')
        with self.assertRaises(audio.AudioError):
            audio.probe(b'', max_bytes=1_000_000, max_seconds=10)
        with self.assertRaises(audio.AudioError):
            audio.probe(b'RIFF\x00\x00\x00\x00WAVE', max_bytes=1_000_000,
                        max_seconds=10)


class MultipartTests(unittest.TestCase):
    def test_parse_fields_and_file(self):
        body, content_type = multipart(b'\x00\x01', {'language': 'en'})
        fields, files = parse_multipart(body, content_type)
        self.assertEqual(fields, {'language': 'en'})
        self.assertEqual(files['audio']['data'], b'\x00\x01')
        self.assertEqual(files['audio']['filename'], 'audio.wav')

    def test_rejects_bad_bodies(self):
        with self.assertRaises(MultipartError):
            parse_multipart(b'', 'application/json')
        body, content_type = raw_multipart([
            ('language', '', b'en'), ('language', '', b'zh')])
        with self.assertRaises(MultipartError):
            parse_multipart(body, content_type)
        body, content_type = raw_multipart([
            ('audio', 'a.wav', b'1'), ('audio', 'b.wav', b'2')])
        with self.assertRaises(MultipartError):
            parse_multipart(body, content_type)
        body, content_type = raw_multipart([
            ('language', '', b'x' * 5000)])
        with self.assertRaises(MultipartError):
            parse_multipart(body, content_type)
        body, content_type = raw_multipart([
            (f'f{i}', '', b'x') for i in range(21)])
        with self.assertRaises(MultipartError):
            parse_multipart(body, content_type)


class AuthAndDbTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'test.sqlite3'
        db.init_db(self.path)
        self.conn = db.connect(self.path)
        self.addCleanup(self.conn.close)
        self.user = db.create_user(self.conn, 'tester@example.test')

    def test_database_files_are_private(self):
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        for suffix in ('-wal', '-shm'):
            candidate = Path(str(self.path) + suffix)
            if candidate.exists():
                self.assertEqual(candidate.stat().st_mode & 0o777, 0o600, suffix)

    def test_tokens_are_hashed_and_expire_or_revoke(self):
        token = auth.issue_token(self.conn, self.user['id'], 1)
        row = self.conn.execute('SELECT token_hash FROM access_tokens').fetchone()
        self.assertEqual(row['token_hash'], hashlib.sha256(token.encode()).hexdigest())
        self.assertNotIn(token, row['token_hash'])
        self.assertIsNotNone(auth.authenticate(self.conn, 'Bearer ' + token))
        self.assertIsNone(auth.authenticate(self.conn, 'Basic ' + token))
        self.assertIsNone(auth.authenticate(self.conn, 'Bearer malformed'))
        self.assertTrue(auth.revoke_token(self.conn, token))
        self.assertIsNone(auth.authenticate(self.conn, 'Bearer ' + token))
        self.assertFalse(auth.revoke_token(self.conn, token))

        expired = auth.issue_token(self.conn, self.user['id'], 1)
        self.conn.execute('UPDATE access_tokens SET expires_at=? WHERE token_hash=?',
                          (db.iso(db.utcnow() - timedelta(days=1)),
                           auth.hash_token(expired)))
        self.assertIsNone(auth.authenticate(self.conn, 'Bearer ' + expired))

    def test_token_matches_is_constant_time_helper(self):
        self.assertTrue(auth.token_matches('Bearer scr_' + 'a' * 30, 'scr_' + 'a' * 30))
        self.assertFalse(auth.token_matches('Bearer scr_' + 'a' * 30, 'scr_' + 'b' * 30))

    def test_job_lifecycle_idempotency_and_quota(self):
        job, created = db.create_or_get_job(
            self.conn, user_id=self.user['id'], client_job_id='client-job-1',
            billable_seconds=2, language='en', grammar_mode='standard',
            client_version='test', timeout_seconds=60, limit_seconds=100)
        self.assertTrue(created)
        same, created = db.create_or_get_job(
            self.conn, user_id=self.user['id'], client_job_id='client-job-1',
            billable_seconds=2, language='en', grammar_mode='standard',
            client_version='test', timeout_seconds=60, limit_seconds=100)
        self.assertFalse(created)
        self.assertEqual(same['id'], job['id'])
        with self.assertRaises(db.DbConflict) as busy:
            db.create_or_get_job(
                self.conn, user_id=self.user['id'], client_job_id='client-job-2',
                billable_seconds=2, language='en', grammar_mode='standard',
                client_version='test', timeout_seconds=60, limit_seconds=100)
        self.assertEqual(busy.exception.code, 'BUSY')

        self.assertTrue(db.complete_job(
            self.conn, job_id=job['id'], user_id=self.user['id'], duration_ms=1500,
            raw_text='hello', output_text='Hello.', detected_language='en',
            warning=None, processing={'asr_ms': 10}, result_ttl_seconds=900))
        self.assertFalse(db.complete_job(
            self.conn, job_id=job['id'], user_id=self.user['id'], duration_ms=1500,
            raw_text='hello', output_text='Hello.', detected_language='en',
            warning=None, processing={}, result_ttl_seconds=900))
        self.assertEqual(db.usage(self.conn, self.user['id'])['used_seconds'], 2)

        with self.assertRaises(db.DbConflict) as quota:
            db.create_or_get_job(
                self.conn, user_id=self.user['id'], client_job_id='client-job-3',
                billable_seconds=99, language='en', grammar_mode='standard',
                client_version='test', timeout_seconds=60, limit_seconds=100)
        self.assertEqual(quota.exception.code, 'QUOTA_EXCEEDED')
        self.assertEqual(db.usage(self.conn, self.user['id'])['used_seconds'], 2)

    def test_stale_processing_job_is_failed_before_new_reservation(self):
        job, _ = db.create_or_get_job(
            self.conn, user_id=self.user['id'], client_job_id='client-stale',
            billable_seconds=2, language='en', grammar_mode='standard',
            client_version='test', timeout_seconds=60, limit_seconds=100)
        self.conn.execute('UPDATE dictation_jobs SET created_at=? WHERE id=?',
                          (db.iso(db.utcnow() - timedelta(minutes=5)), job['id']))
        fresh, created = db.create_or_get_job(
            self.conn, user_id=self.user['id'], client_job_id='client-fresh',
            billable_seconds=2, language='en', grammar_mode='standard',
            client_version='test', timeout_seconds=60, limit_seconds=100)
        self.assertTrue(created)
        stale = db.get_job(self.conn, self.user['id'], job['id'])
        self.assertEqual(stale['state'], 'failed')
        self.assertEqual(stale['error_code'], 'JOB_TIMEOUT')
        self.assertEqual(db.usage(self.conn, self.user['id'])['reserved_seconds'], 2)
        self.assertNotEqual(fresh['id'], job['id'])

    def test_cancel_and_expiry_clear_text(self):
        job, _ = db.create_or_get_job(
            self.conn, user_id=self.user['id'], client_job_id='client-cancel',
            billable_seconds=2, language='en', grammar_mode='standard',
            client_version='test', timeout_seconds=60, limit_seconds=100)
        cancelled = db.cancel_job(self.conn, self.user['id'], job['id'])
        self.assertEqual(cancelled['state'], 'cancelled')
        self.assertEqual(db.cancel_job(self.conn, self.user['id'], job['id'])['state'],
                         'cancelled')
        self.assertIsNone(db.cancel_job(self.conn, self.user['id'], 'job_missing'))
        self.assertFalse(db.complete_job(
            self.conn, job_id=job['id'], user_id=self.user['id'], duration_ms=1000,
            raw_text='x', output_text='x', detected_language=None, warning=None,
            processing={}, result_ttl_seconds=900))

        done, _ = db.create_or_get_job(
            self.conn, user_id=self.user['id'], client_job_id='client-expire',
            billable_seconds=1, language='en', grammar_mode='standard',
            client_version='test', timeout_seconds=60, limit_seconds=100)
        db.complete_job(self.conn, job_id=done['id'], user_id=self.user['id'],
                        duration_ms=1000, raw_text='secret', output_text='Secret.',
                        detected_language='en', warning=None, processing={},
                        result_ttl_seconds=0)
        stored = db.get_job(self.conn, self.user['id'], done['id'])
        self.assertTrue(db.job_is_expired(stored))
        db.clear_expired_result(self.conn, stored)
        cleared = db.get_job(self.conn, self.user['id'], done['id'])
        self.assertIsNone(cleared['raw_text'])
        self.assertIsNone(cleared['output_text'])

    def test_period_start_is_utc_month(self):
        from datetime import datetime, timezone
        value = datetime(2026, 10, 31, 23, 59, tzinfo=timezone.utc)
        self.assertEqual(db.period_start(value), '2026-10-01')


class ConfigTests(unittest.TestCase):
    def test_from_env_and_billing_flags(self):
        env = {
            'SCRIBIVA_DB': '/tmp/scribiva-test.sqlite3',
            'SCRIBIVA_PORT': '9999',
            'SCRIBIVA_FREE_SECONDS': '10',
            'SCRIBIVA_PRO_SECONDS': '20',
            'STRIPE_SECRET_KEY': 'sk_test',
            'SCRIBIVA_PUBLIC_BASE_URL': 'https://example.test/',
            'STRIPE_WEBHOOK_SECRET': 'whsec',
        }
        old = os.environ.copy()
        try:
            os.environ.clear()
            os.environ.update(env)
            settings = Settings.from_env()
        finally:
            os.environ.clear()
            os.environ.update(old)
        self.assertEqual(settings.port, 9999)
        self.assertEqual(settings.free_seconds, 10)
        self.assertTrue(settings.billing_configured)
        self.assertTrue(settings.webhook_configured)
        self.assertEqual(settings.public_base_url, 'https://example.test')
        self.assertEqual(settings.max_request_bytes, settings.max_audio_bytes + 1_048_576)

    def test_invalid_environment_values(self):
        for key, value in (
                ('SCRIBIVA_PORT', '0'), ('SCRIBIVA_PORT', 'abc'),
                ('SCRIBIVA_FREE_SECONDS', '-1'), ('SCRIBIVA_RATE_LIMIT', '0'),
                ('SCRIBIVA_ALLOW_REMOTE', 'maybe'), ('SCRIBIVA_DB', '/tmp/no-suffix')):
            old = os.environ.copy()
            try:
                os.environ.clear()
                os.environ[key] = value
                with self.assertRaises(ConfigError):
                    Settings.from_env()
            finally:
                os.environ.clear()
                os.environ.update(old)


class ProviderTests(unittest.TestCase):
    class Stub(GroqProvider):
        def __init__(self, reply='', error=None):
            super().__init__(Settings())
            self.reply = reply
            self.error = error
            self.calls = []

        def _json(self, path, payload, timeout):
            self.calls.append(payload)
            if self.error:
                raise self.error
            return {'choices': [{'message': {'content': self.reply}}]}

    def test_correct_preserves_protected_literals(self):
        text = 'use https://example.com and 42 items'
        protected, parts = textguard.protect(text)
        token = next(iter(parts))
        provider = self.Stub(f'Please use {token} and 42 items.')
        result, warning = provider.correct(text, mode='standard', language='en')
        self.assertIsNone(warning)
        self.assertEqual(result, 'Please use https://example.com and 42 items.')
        self.assertIn(token, provider.calls[0]['messages'][1]['content'])

    def test_correct_fails_closed_on_guard_or_provider_failure(self):
        text = 'use https://example.com and 42 items'
        protected, parts = textguard.protect(text)
        token = next(iter(parts))
        cases = [
            ('Please use it and 42 items.', 'Protected token'),
            (f'Please use {token} and 43 items.', 'Numeric content'),
            (f'{token} {token} {token}', 'Protected token'),
            ('x' * 1000, 'Expansion'),
        ]
        for reply, expected in cases:
            provider = self.Stub(reply)
            result, warning = provider.correct(text, mode='standard', language='en')
            self.assertEqual(result, text)
            self.assertIn(expected, warning)
        failing = self.Stub(error=ProviderError('PROVIDER_ERROR', 'down'))
        result, warning = failing.correct(text, mode='polished', language='en')
        self.assertEqual(result, text)
        self.assertIn('down', warning)

    def test_correct_raw_mode_never_calls_provider(self):
        provider = self.Stub('unused')
        self.assertEqual(provider.correct('hello', mode='raw', language='en'),
                         ('hello', None))
        self.assertEqual(provider.calls, [])

    def test_transcribe_requires_key_and_valid_text(self):
        provider = GroqProvider(Settings())
        with self.assertRaises(ProviderError) as missing:
            provider.transcribe(b'x', filename='a.wav', content_type='audio/wav',
                                language='en')
        self.assertEqual(missing.exception.code, 'PROVIDER_NOT_CONFIGURED')

        class FakeOpen(GroqProvider):
            def __init__(self, body):
                super().__init__(Settings(groq_api_key='gsk_test'))
                self.body = body

            def _open(self, request, timeout):
                return self.body

        self.assertEqual(FakeOpen(b'{"text":" hi "}').transcribe(
            b'x', filename='a.wav', content_type='audio/wav', language='auto'), 'hi')
        with self.assertRaises(ProviderError) as empty:
            FakeOpen(b'{"text":"  "}').transcribe(
                b'x', filename='a.wav', content_type='audio/wav', language='auto')
        self.assertEqual(empty.exception.code, 'EMPTY_TRANSCRIPT')
        with self.assertRaises(ProviderError) as invalid:
            FakeOpen(b'not json').transcribe(
                b'x', filename='a.wav', content_type='audio/wav', language='auto')
        self.assertEqual(invalid.exception.code, 'PROVIDER_ERROR')
        too_long = json.dumps({'text': 'x' * 100_001}).encode()
        with self.assertRaises(ProviderError) as oversized:
            FakeOpen(too_long).transcribe(
                b'x', filename='a.wav', content_type='audio/wav', language='auto')
        self.assertEqual(oversized.exception.code, 'TRANSCRIPT_INVALID')

    def test_provider_rejects_unsafe_key(self):
        provider = GroqProvider(Settings(groq_api_key='gsk_bad\nkey'))
        with self.assertRaises(ProviderError) as bad:
            provider.transcribe(b'x', filename='a.wav', content_type='audio/wav',
                                language='en')
        self.assertEqual(bad.exception.code, 'PROVIDER_NOT_CONFIGURED')


class StripeEventTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'test.sqlite3'
        db.init_db(self.path)
        self.conn = db.connect(self.path)
        self.addCleanup(self.conn.close)
        self.user = db.create_user(self.conn, 'tester@example.test')

    def _subscription_event(self, event_type, status='active'):
        return {
            'type': event_type,
            'data': {'object': {
                'id': 'sub_1', 'customer': 'cus_1', 'status': status,
                'current_period_end': 1_800_000_000,
                'metadata': {'user_id': self.user['id']},
            }},
        }

    def test_subscription_events_map_to_entitlement(self):
        result = apply_event(self.conn, self._subscription_event(
            'customer.subscription.created'))
        self.assertTrue(result['applied'])
        sub = db.get_subscription(self.conn, self.user['id'])
        self.assertEqual(db.effective_plan(sub), 'pro')
        self.assertEqual(sub['current_period_end'], '2027-01-15T08:00:00+00:00')
        result = apply_event(self.conn, self._subscription_event(
            'customer.subscription.deleted', status='active'))
        self.assertTrue(result['applied'])
        self.assertEqual(db.effective_plan(db.get_subscription(self.conn,
                                                               self.user['id'])),
                         'free')

    def test_checkout_is_incomplete_until_subscription_event(self):
        event = {
            'type': 'checkout.session.completed',
            'data': {'object': {
                'customer': 'cus_1', 'subscription': 'sub_1',
                'client_reference_id': self.user['id'],
                'metadata': {'user_id': self.user['id']},
            }},
        }
        self.assertTrue(apply_event(self.conn, event)['applied'])
        sub = db.get_subscription(self.conn, self.user['id'])
        self.assertEqual(sub['status'], 'incomplete')
        self.assertEqual(db.effective_plan(sub), 'free')

    def test_invoice_paid_is_not_an_entitlement_source(self):
        event = {
            'type': 'invoice.paid',
            'data': {'object': {'customer': 'cus_1',
                                'metadata': {'user_id': self.user['id']}}},
        }
        result = apply_event(self.conn, event)
        self.assertFalse(result['applied'])
        self.assertEqual(db.get_subscription(self.conn, self.user['id'])['status'],
                         'inactive')

    def test_unknown_and_malformed_events_are_ignored(self):
        self.assertFalse(apply_event(self.conn, {'type': 'x', 'data': {}})['applied'])
        event = self._subscription_event('customer.subscription.created')
        event['data']['object']['metadata'] = {}
        event['data']['object']['client_reference_id'] = 'usr_missing'
        self.assertFalse(apply_event(self.conn, event)['applied'])


if __name__ == '__main__':
    unittest.main()
