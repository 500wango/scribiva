import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scribiva import providers
from server_support import ServerTestCase, wav_bytes

CONFIG = {
    'language': 'en', 'mode': 'standard', 'asr_model': 'whisper-large-v3',
    'grammar_model': 'openai/gpt-oss-20b', 'auto_insert': True,
    'restore_text_clipboard': False, 'mic_node': 'alsa_input.fake',
    'max_record_seconds': 180, 'asr_prompt': '', 'proxy_url': '',
    'service_url': '', 'delete_audio_on_success': False,
}


class FakeStripe:
    """Records billing calls; never reaches the real Stripe API."""

    def __init__(self):
        self.checkout_calls = []
        self.portal_calls = []
        self.checkout_url = 'https://checkout.stripe.com/c/pay/cs_test_local'
        self.portal_url = 'https://billing.stripe.com/p/session/test_local'
        self.checkout_session = 'cs_test_local'

    def create_checkout(self, user, *, price_id=None, customer_id=None):
        self.checkout_calls.append({'user': user['id'], 'price_id': price_id,
                                    'customer_id': customer_id})
        return {'url': self.checkout_url, 'session_id': self.checkout_session}

    def create_portal(self, *, customer_id=None):
        self.portal_calls.append({'customer_id': customer_id})
        return {'url': self.portal_url}


class ServiceClientTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()

    def service_config(self, url=None):
        return {**CONFIG, 'service_url': url or f'http://127.0.0.1:{self.port}'}

    def audio_file(self, seconds=1.5):
        path = Path(self.temp.name) / 'audio.wav'
        path.write_bytes(wav_bytes(seconds))
        return path

    def test_dictation_roundtrip_is_idempotent(self):
        config = self.service_config()
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
            first = providers.service_dictation(
                self.audio_file(), config, job_id='client-job-abcdefgh')
            second = providers.service_dictation(
                self.audio_file(), config, job_id='client-job-abcdefgh')
            account = providers.service_me(config)
            status = providers.service_status(config)
        self.assertEqual(first['raw_text'], 'hello world')
        self.assertEqual(first['output_text'], 'hello world, edited.')
        self.assertEqual(first['billing']['charged_audio_seconds'], 2)
        self.assertEqual(second['job_id'], first['job_id'])
        self.assertEqual(len(self.provider.transcribe_calls), 1)
        self.assertEqual(account['usage']['used_seconds'], 2)
        self.assertEqual(status['server']['api_version'], 'v1')
        self.assertEqual(status['account']['plan'], 'free')

    def test_token_errors_do_not_reach_provider(self):
        config = self.service_config()
        with patch.object(providers, 'service_token', return_value=''):
            with self.assertRaises(providers.ProviderError) as missing:
                providers.service_dictation(self.audio_file(), config,
                                            job_id='client-job-missing-token')
        self.assertEqual(missing.exception.code, 'SERVICE_TOKEN_MISSING')
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': 'scr_' + 'x' * 40}):
            with self.assertRaises(providers.ProviderError) as rejected:
                providers.service_dictation(self.audio_file(), config,
                                            job_id='client-job-bad-token')
        self.assertEqual(rejected.exception.code, 'AUTH_REQUIRED')
        self.assertEqual(self.provider.transcribe_calls, [])

    def test_unreachable_and_invalid_service_urls(self):
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': 'scr_' + 'x' * 40}):
            with self.assertRaises(providers.ProviderError) as down:
                providers.service_dictation(
                    self.audio_file(), self.service_config('http://127.0.0.1:1'),
                    job_id='client-job-unreachable')
        self.assertEqual(down.exception.code, 'SERVICE_UNREACHABLE')
        config = self.service_config('http://example.com')
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': 'scr_' + 'x' * 40}):
            with self.assertRaises(providers.ProviderError) as unsafe:
                providers.service_dictation(self.audio_file(), config,
                                            job_id='client-job-unsafe-url')
        self.assertEqual(unsafe.exception.code, 'SERVICE_URL_INVALID')
        with self.assertRaises(providers.ProviderError) as direct:
            providers.service_dictation(self.audio_file(), CONFIG,
                                        job_id='client-job-direct-mode')
        self.assertEqual(direct.exception.code, 'SERVICE_NOT_CONFIGURED')

    def test_job_id_is_validated(self):
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
            with self.assertRaises(providers.ProviderError):
                providers.service_dictation(self.audio_file(), self.service_config(),
                                            job_id='short')


class BillingClientTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()
        self.stripe = FakeStripe()
        self.service.stripe = self.stripe

    def config(self):
        return {**CONFIG, 'service_url': f'http://127.0.0.1:{self.port}'}

    def test_checkout_returns_stripe_url_and_plan_price(self):
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
            result = providers.service_checkout(self.config(), 'annual')
        self.assertEqual(result['url'], self.stripe.checkout_url)
        self.assertEqual(result['session_id'], 'cs_test_local')
        self.assertEqual(self.stripe.checkout_calls, [{
            'user': self.user['id'], 'price_id': 'price_annual',
            'customer_id': None}])

    def test_portal_returns_stripe_url(self):
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
            result = providers.service_portal(self.config())
        self.assertEqual(result['url'], self.stripe.portal_url)
        self.assertEqual(self.stripe.portal_calls, [{'customer_id': None}])

    def test_delete_revokes_server_tokens_and_results(self):
        config = self.config()
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
            result = providers.service_delete(config)
            with self.assertRaises(providers.ProviderError) as revoked:
                providers.service_me(config)
        self.assertEqual(result['status'], 'deletion_requested')
        self.assertEqual(result['revoked_tokens'], 1)
        self.assertEqual(revoked.exception.status, 401)
        self.assertEqual(self.provider.transcribe_calls, [])

    def test_invalid_plan_is_rejected_before_network(self):
        with self.assertRaises(providers.ProviderError) as invalid:
            providers.service_checkout(self.config(), 'lifetime')
        self.assertEqual(invalid.exception.code, 'INVALID_PLAN')
        self.assertEqual(self.stripe.checkout_calls, [])

    def test_unsafe_or_missing_billing_urls_are_rejected(self):
        for value in ('http://checkout.stripe.com/x', '', None, 'javascript:alert(1)'):
            self.stripe.checkout_url = value
            with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
                with self.assertRaises(providers.ProviderError) as unsafe:
                    providers.service_checkout(self.config())
            self.assertIn(unsafe.exception.code,
                          ('BILLING_URL_INVALID', 'BILLING_URL_MISSING'))


class BillingNotConfiguredTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server(stripe_secret_key='', public_base_url='')

    def test_checkout_and_portal_report_configuration_missing(self):
        config = {**CONFIG, 'service_url': f'http://127.0.0.1:{self.port}'}
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
            for call in (providers.service_checkout, providers.service_portal):
                with self.assertRaises(providers.ProviderError) as missing:
                    call(config)
                self.assertEqual(missing.exception.code, 'BILLING_NOT_CONFIGURED')
                self.assertEqual(missing.exception.status, 503)
        self.assertEqual(self.provider.transcribe_calls, [])


class PortalWithoutCustomerTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()

    def test_portal_without_stripe_customer_is_mapped(self):
        config = {**CONFIG, 'service_url': f'http://127.0.0.1:{self.port}'}
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
            with self.assertRaises(providers.ProviderError) as missing:
                providers.service_portal(config)
        self.assertEqual(missing.exception.code, 'NO_BILLING_ACCOUNT')
        self.assertEqual(missing.exception.status, 400)


class ServiceQuotaTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server(free_seconds=1)

    def test_quota_error_is_mapped(self):
        path = Path(self.temp.name) / 'audio.wav'
        path.write_bytes(wav_bytes(1.5))
        config = {**CONFIG, 'service_url': f'http://127.0.0.1:{self.port}'}
        with patch.dict(os.environ, {'SCRIBIVA_SERVICE_TOKEN': self.token}):
            with self.assertRaises(providers.ProviderError) as quota:
                providers.service_dictation(path, config,
                                            job_id='client-job-quota-test')
        self.assertEqual(quota.exception.code, 'QUOTA_EXCEEDED')
        self.assertEqual(quota.exception.status, 402)
        self.assertEqual(self.provider.transcribe_calls, [])


class ServiceConfigTests(unittest.TestCase):
    def test_url_validation(self):
        from scribiva.config import validate_service_url
        self.assertEqual(validate_service_url(''), '')
        self.assertEqual(validate_service_url('http://127.0.0.1:8787/'),
                         'http://127.0.0.1:8787')
        self.assertEqual(validate_service_url('http://[::1]:8787/api/'),
                         'http://[::1]:8787/api')
        self.assertEqual(validate_service_url('https://api.example.com/base'),
                         'https://api.example.com/base')
        for value in (
                'http://api.example.com', 'ftp://api.example.com',
                'https://user:pass@api.example.com', 'https://api.example.com?x=1',
                'https://api.example.com/#x', 'https://api.example.com:99999',
                'https://api.example.com/a b', 'https://:8787', 'not-a-url'):
            with self.assertRaises(ValueError, msg=value):
                validate_service_url(value)

    def test_token_file_requires_private_permissions(self):
        from scribiva import config
        with tempfile.TemporaryDirectory() as root:
            token_path = Path(root) / 'service-token'
            with patch.object(config, 'SERVICE_TOKEN_PATH', token_path), \
                 patch.object(config, 'secure_dirs'), \
                 patch.dict(os.environ, {}, clear=False):
                os.environ.pop('SCRIBIVA_SERVICE_TOKEN', None)
                config.write_service_token('scr_' + 'a' * 40)
                self.assertEqual(config.service_token(), 'scr_' + 'a' * 40)
                self.assertEqual(token_path.stat().st_mode & 0o777, 0o600)
                token_path.chmod(0o644)
                with self.assertRaisesRegex(RuntimeError, 'permissions'):
                    config.service_token()
                self.assertTrue(config.clear_service_token())
                self.assertEqual(config.service_token(), '')
                with self.assertRaises(ValueError):
                    config.write_service_token('not-a-token')

    def test_environment_token_overrides_file(self):
        from scribiva import config
        with tempfile.TemporaryDirectory() as root:
            token_path = Path(root) / 'service-token'
            with patch.object(config, 'SERVICE_TOKEN_PATH', token_path), \
                 patch.object(config, 'secure_dirs'):
                config.write_service_token('scr_' + 'a' * 40)
                with patch.dict(os.environ,
                                {'SCRIBIVA_SERVICE_TOKEN': 'scr_' + 'b' * 40}):
                    self.assertEqual(config.service_token(), 'scr_' + 'b' * 40)


if __name__ == '__main__':
    unittest.main()
