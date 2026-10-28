import hashlib
import hmac
import json
import threading
import time
import unittest

from scribiva_server import db
from scribiva_server.provider import ProviderError
from server_support import ServerTestCase, multipart, wav_bytes


class PublicAndAuthTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()

    def test_public_config_and_health_do_not_leak_secrets(self):
        status, config, _ = self.request('GET', '/v1/config/public', token=False)
        self.assertEqual(status, 200)
        self.assertEqual(config['api_version'], 'v1')
        self.assertTrue(config['features']['dictation'])
        self.assertNotIn('gsk_test_server_key', json.dumps(config))
        status, health, _ = self.request('GET', '/healthz', token=False)
        self.assertEqual(status, 200)
        self.assertTrue(health['provider_configured'])
        self.assertNotIn('gsk_test_server_key', json.dumps(health))

    def test_auth_required_and_token_validation(self):
        status, body, _ = self.request('GET', '/v1/me', token=False)
        self.assertEqual(status, 401)
        self.assertEqual(body['error']['code'], 'AUTH_REQUIRED')
        status, _, _ = self.request('GET', '/v1/me', token=False,
                                    headers={'Authorization': 'Bearer scr_invalid_token_aaaaaaaa'})
        self.assertEqual(status, 401)
        status, me, _ = self.request('GET', '/v1/me')
        self.assertEqual(status, 200)
        self.assertEqual(me['plan'], 'free')
        self.assertEqual(me['usage']['used_seconds'], 0)
        self.assertEqual(me['limits']['monthly_free_seconds'], 1800)

    def test_usage_endpoint(self):
        status, usage, _ = self.request('GET', '/v1/me/usage')
        self.assertEqual(status, 200)
        self.assertEqual(usage['plan'], 'free')
        self.assertEqual(usage['remaining_seconds'], 1800)

    def test_method_not_allowed(self):
        status, body, _ = self.request('PUT', '/v1/me')
        self.assertEqual(status, 405)
        self.assertEqual(body['error']['code'], 'METHOD_NOT_ALLOWED')


class DictationTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()

    def test_successful_dictation_records_usage(self):
        status, body, _ = self.dictation()
        self.assertEqual(status, 200, body)
        self.assertEqual(body['raw_text'], 'hello world')
        self.assertEqual(body['output_text'], 'hello world, edited.')
        self.assertEqual(body['billing']['charged_audio_seconds'], 2)
        self.assertEqual(body['billing']['remaining_free_seconds'], 1798)
        self.assertEqual(body['grammar_mode'], 'standard')
        self.assertEqual(self.provider.transcribe_calls[0]['language'], 'en')
        self.assertEqual(self.provider.correct_calls[0]['mode'], 'standard')
        status, usage, _ = self.request('GET', '/v1/me/usage')
        self.assertEqual(usage['used_seconds'], 2)
        status, fetched, _ = self.request('GET', f"/v1/dictations/{body['job_id']}")
        self.assertEqual(status, 200)
        self.assertEqual(fetched['raw_text'], 'hello world')
        status, recent, _ = self.request('GET', '/v1/me/dictations')
        self.assertEqual(status, 200)
        self.assertEqual(len(recent['jobs']), 1)
        self.assertNotIn('raw_text', recent['jobs'][0])
        self.assertNotIn('output_text', recent['jobs'][0])

    def test_raw_mode_skips_grammar(self):
        status, body, _ = self.dictation(fields={
            'language': 'en', 'grammar_mode': 'raw',
            'client_job_id': 'job-raw-mode', 'client_version': 'test'}, key='job-raw-mode')
        self.assertEqual(status, 200, body)
        self.assertEqual(body['output_text'], 'hello world')
        self.assertEqual(self.provider.correct_calls, [])

    def test_idempotent_replay_does_not_charge_twice(self):
        status, first, _ = self.dictation()
        self.assertEqual(status, 200)
        status, second, _ = self.dictation()
        self.assertEqual(status, 200)
        self.assertEqual(first['job_id'], second['job_id'])
        self.assertEqual(len(self.provider.transcribe_calls), 1)
        status, usage, _ = self.request('GET', '/v1/me/usage')
        self.assertEqual(usage['used_seconds'], 2)

    def test_missing_or_mismatched_idempotency_key(self):
        payload, content_type = multipart(wav_bytes(), {'language': 'en'})
        status, body, _ = self.request(
            'POST', '/v1/dictations', body=payload,
            headers={'Content-Type': content_type})
        self.assertEqual(status, 400)
        self.assertEqual(body['error']['code'], 'IDEMPOTENCY_KEY_REQUIRED')
        payload, content_type = multipart(wav_bytes(), {
            'language': 'en', 'client_job_id': 'other-job'})
        status, body, _ = self.request(
            'POST', '/v1/dictations', body=payload,
            headers={'Content-Type': content_type, 'Idempotency-Key': 'job-abcdefgh'})
        self.assertEqual(status, 400)
        self.assertEqual(body['error']['code'], 'IDEMPOTENCY_KEY_MISMATCH')

    def test_invalid_language_and_mode(self):
        status, body, _ = self.dictation(fields={
            'language': 'xx', 'grammar_mode': 'standard',
            'client_job_id': 'job-bad-lang'}, key='job-bad-lang')
        self.assertEqual(status, 400)
        self.assertEqual(body['error']['code'], 'UNSUPPORTED_LANGUAGE')
        status, body, _ = self.dictation(fields={
            'language': 'en', 'grammar_mode': 'translate',
            'client_job_id': 'job-bad-mode'}, key='job-bad-mode')
        self.assertEqual(status, 400)
        self.assertEqual(body['error']['code'], 'INVALID_GRAMMAR_MODE')

    def test_invalid_audio(self):
        status, body, _ = self.dictation(audio=b'not really audio')
        self.assertEqual(status, 400)
        self.assertEqual(body['error']['code'], 'INVALID_AUDIO')
        self.assertEqual(self.provider.transcribe_calls, [])

    def test_request_audio_cap_returns_413(self):
        status, body, _ = self.dictation(audio=wav_bytes(1.5), key='job-too-big')
        self.assertEqual(status, 200)
        # Rebuild this test with a cap smaller than the payload.
        self.server.server_close()
        self.thread.join(timeout=2)
        self.start_server(max_audio_bytes=2000)
        status, body, _ = self.dictation(audio=wav_bytes(1.5), key='job-too-big-2')
        self.assertEqual(status, 413)
        self.assertEqual(body['error']['code'], 'AUDIO_TOO_LARGE')

    def test_provider_failure_is_not_charged_and_replay_conflicts(self):
        self.provider.transcribe_error = ProviderError(
            'PROVIDER_TIMEOUT', 'provider down', status=504, retryable=True)
        status, body, _ = self.dictation()
        self.assertEqual(status, 504)
        self.assertEqual(body['error']['code'], 'PROVIDER_TIMEOUT')
        status, usage, _ = self.request('GET', '/v1/me/usage')
        self.assertEqual(usage['used_seconds'], 0)
        status, body, _ = self.dictation()
        self.assertEqual(status, 409)
        self.assertEqual(body['error']['code'], 'IDEMPOTENCY_CONFLICT')

    def test_quota_exceeded_does_not_call_provider(self):
        self.server.server_close()
        self.thread.join(timeout=2)
        self.start_server(free_seconds=1)
        status, body, _ = self.dictation(audio=wav_bytes(1.5))
        self.assertEqual(status, 402)
        self.assertEqual(body['error']['code'], 'QUOTA_EXCEEDED')
        self.assertEqual(self.provider.transcribe_calls, [])
        status, usage, _ = self.request('GET', '/v1/me/usage')
        self.assertEqual(usage['used_seconds'], 0)
        self.assertEqual(usage['remaining_seconds'], 1)

    def test_second_concurrent_job_is_rejected(self):
        self.provider.block = threading.Event()
        result = []

        def first():
            result.append(self.dictation(key='job-concurrent-1'))

        worker = threading.Thread(target=first)
        worker.start()
        self.assertTrue(self.provider.started.wait(timeout=3))
        status, body, _ = self.dictation(key='job-concurrent-2')
        self.assertEqual(status, 409)
        self.assertEqual(body['error']['code'], 'BUSY')
        self.provider.block.set()
        worker.join(timeout=5)
        self.assertEqual(result[0][0], 200, result)

    def test_cancel_during_provider_call_does_not_charge(self):
        self.provider.block = threading.Event()
        result = []

        def first():
            result.append(self.dictation(key='job-cancel-me'))

        worker = threading.Thread(target=first)
        worker.start()
        self.assertTrue(self.provider.started.wait(timeout=3))
        conn = db.connect(self.settings.db_path)
        try:
            job_id = conn.execute(
                "SELECT id FROM dictation_jobs WHERE user_id=? AND state='processing'",
                (self.user['id'],)).fetchone()['id']
        finally:
            conn.close()
        status, body, _ = self.request('POST', f'/v1/dictations/{job_id}/cancel')
        self.assertEqual(status, 200, body)
        self.assertEqual(body['status'], 'cancelled')
        self.provider.block.set()
        worker.join(timeout=5)
        self.assertEqual(result[0][0], 409, result)
        self.assertEqual(result[0][1]['error']['code'], 'JOB_CANCELLED')
        status, usage, _ = self.request('GET', '/v1/me/usage')
        self.assertEqual(usage['used_seconds'], 0)

    def test_result_ttl_expiry(self):
        self.server.server_close()
        self.thread.join(timeout=2)
        self.start_server(result_ttl_seconds=0)
        status, body, _ = self.dictation()
        self.assertEqual(status, 200)
        status, body, _ = self.request('GET', f"/v1/dictations/{body['job_id']}")
        self.assertEqual(status, 410)
        self.assertEqual(body['error']['code'], 'RESULT_EXPIRED')

    def test_get_and_cancel_unknown_job(self):
        status, body, _ = self.request('GET', '/v1/dictations/job_missing')
        self.assertEqual(status, 404)
        status, body, _ = self.request('POST', '/v1/dictations/job_missing/cancel')
        self.assertEqual(status, 404)

    def test_rate_limit(self):
        self.server.server_close()
        self.thread.join(timeout=2)
        self.start_server(rate_limit_per_minute=1)
        status, _, _ = self.dictation(key='job-rate-1')
        self.assertEqual(status, 200)
        status, body, headers = self.dictation(key='job-rate-2')
        self.assertEqual(status, 429)
        self.assertEqual(body['error']['code'], 'RATE_LIMITED')
        self.assertIn('Retry-After', headers)


class PrivacyTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()

    def test_delete_requires_confirmation(self):
        status, body, _ = self.request('DELETE', '/v1/me/data', body={})
        self.assertEqual(status, 400)
        self.assertEqual(body['error']['code'], 'CONFIRMATION_REQUIRED')

    def test_delete_removes_jobs_and_tokens(self):
        status, job, _ = self.dictation()
        self.assertEqual(status, 200)
        status, body, _ = self.request('DELETE', '/v1/me/data',
                                       body={'confirm': 'DELETE'})
        self.assertEqual(status, 202, body)
        self.assertEqual(body['status'], 'deletion_requested')
        self.assertGreaterEqual(body['deleted_jobs'], 1)
        status, _, _ = self.request('GET', '/v1/me')
        self.assertEqual(status, 401)
        conn = db.connect(self.settings.db_path)
        try:
            self.assertEqual(conn.execute(
                'SELECT COUNT(*) AS n FROM dictation_jobs WHERE user_id=?',
                (self.user['id'],)).fetchone()['n'], 0)
            self.assertEqual(conn.execute(
                'SELECT COUNT(*) AS n FROM access_tokens WHERE user_id=?',
                (self.user['id'],)).fetchone()['n'], 0)
            # Billing metadata is retained without transcript text.
            self.assertGreater(conn.execute(
                'SELECT COUNT(*) AS n FROM usage_ledger WHERE user_id=?',
                (self.user['id'],)).fetchone()['n'], 0)
        finally:
            conn.close()


class StripeTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()

    def sign(self, payload: bytes, secret='whsec_test', timestamp=None):
        stamp = int(time.time()) if timestamp is None else timestamp
        digest = hmac.new(secret.encode(), f'{stamp}.'.encode() + payload,
                          hashlib.sha256).hexdigest()
        return f't={stamp},v1={digest}'

    def event(self, user_id, event_id='evt_test_1'):
        return {
            'id': event_id,
            'type': 'customer.subscription.updated',
            'data': {'object': {
                'id': 'sub_test',
                'customer': 'cus_test',
                'status': 'active',
                'current_period_end': int(time.time()) + 86400,
                'metadata': {'user_id': user_id},
            }},
        }

    def test_signed_webhook_grants_pro_and_is_deduplicated(self):
        payload = json.dumps(self.event(self.user['id'])).encode()
        headers = {'Content-Type': 'application/json',
                   'Stripe-Signature': self.sign(payload)}
        status, body, _ = self.request('POST', '/v1/webhooks/stripe',
                                       body=payload, headers=headers, token=False)
        self.assertEqual(status, 200, body)
        self.assertTrue(body['applied'])
        status, me, _ = self.request('GET', '/v1/me')
        self.assertEqual(me['plan'], 'pro')
        self.assertEqual(me['limits']['monthly_pro_seconds'], 36000)
        status, body, _ = self.request('POST', '/v1/webhooks/stripe',
                                       body=payload, headers=headers, token=False)
        self.assertEqual(status, 200)
        self.assertTrue(body['duplicate'])

    def test_invalid_signature_and_unknown_event(self):
        payload = json.dumps(self.event(self.user['id'], 'evt_bad')).encode()
        headers = {'Content-Type': 'application/json',
                   'Stripe-Signature': 't=1,v1=deadbeef'}
        status, body, _ = self.request('POST', '/v1/webhooks/stripe',
                                       body=payload, headers=headers, token=False)
        self.assertEqual(status, 400)
        self.assertEqual(body['error']['code'], 'INVALID_SIGNATURE')
        status, body, _ = self.request('POST', '/v1/webhooks/stripe',
                                       body=b'{}', headers=headers, token=False)
        self.assertEqual(status, 400)

    def test_webhook_not_configured(self):
        self.server.server_close()
        self.thread.join(timeout=2)
        self.start_server(stripe_webhook_secret='')
        payload = json.dumps(self.event(self.user['id'])).encode()
        status, body, _ = self.request('POST', '/v1/webhooks/stripe', body=payload,
                                       headers={'Content-Type': 'application/json'},
                                       token=False)
        self.assertEqual(status, 503)
        self.assertEqual(body['error']['code'], 'BILLING_NOT_CONFIGURED')

    def test_checkout_not_configured(self):
        self.server.server_close()
        self.thread.join(timeout=2)
        self.start_server(stripe_secret_key='', public_base_url='')
        status, body, _ = self.request('POST', '/v1/billing/checkout-session',
                                       body={'plan': 'monthly'})
        self.assertEqual(status, 503)
        self.assertEqual(body['error']['code'], 'BILLING_NOT_CONFIGURED')

    def test_portal_without_customer(self):
        status, body, _ = self.request('POST', '/v1/billing/portal-session', body={})
        self.assertEqual(status, 400)
        self.assertEqual(body['error']['code'], 'NO_BILLING_ACCOUNT')

    def test_invalid_plan(self):
        status, body, _ = self.request('POST', '/v1/billing/checkout-session',
                                       body={'plan': 'lifetime'})
        self.assertEqual(status, 400)
        self.assertEqual(body['error']['code'], 'INVALID_PLAN')


class SignatureUnitTests(unittest.TestCase):
    def test_verify_signature(self):
        from scribiva_server.stripe import verify_signature
        payload = b'{"id":"evt_1"}'
        stamp = 1_700_000_000
        digest = hmac.new(b'whsec_test', f'{stamp}.'.encode() + payload,
                          hashlib.sha256).hexdigest()
        header = f't={stamp},v1={digest}'
        self.assertTrue(verify_signature(payload, header, 'whsec_test',
                                         tolerance=300, now=stamp))
        self.assertFalse(verify_signature(payload, header, 'wrong',
                                          tolerance=300, now=stamp))
        self.assertFalse(verify_signature(payload, header, 'whsec_test',
                                          tolerance=300, now=stamp + 301))
        self.assertFalse(verify_signature(payload, None, 'whsec_test', now=stamp))
        self.assertFalse(verify_signature(payload, 'v1=abc', 'whsec_test', now=stamp))


if __name__ == '__main__':
    unittest.main()
