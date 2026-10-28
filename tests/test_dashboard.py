import http.client
import json
import threading
import unittest
from unittest.mock import patch
from scribiva import dashboard


class DashboardTests(unittest.TestCase):
    def setUp(self):
        self.server, self.token = dashboard.make_server('test-token')
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       kwargs={'poll_interval': 0.05}, daemon=True)
        self.thread.start()
        self.addCleanup(self._stop)
        self.port = self.server.server_port

    def _stop(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def request(self, method, path, body=None, token=None, host=None, origin=None):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=3)
        headers = {}
        if host:
            headers['Host'] = host
        if token:
            headers['X-Scribiva-Token'] = token
        if origin:
            headers['Origin'] = origin
        payload = None
        if body is not None:
            payload = json.dumps(body)
            headers['Content-Type'] = 'application/json'
        try:
            connection.request(method, path, body=payload, headers=headers)
            response = connection.getresponse()
            raw = response.read()
            return response.status, raw
        finally:
            connection.close()

    def get_json(self, path):
        status, raw = self.request('GET', path)
        self.assertEqual(status, 200)
        return json.loads(raw)

    def test_index_contains_csrf_token_and_security_headers(self):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=3)
        connection.request('GET', '/')
        response = connection.getresponse()
        body = response.read().decode('utf-8')
        connection.close()
        self.assertEqual(response.status, 200)
        self.assertIn('test-token', body)
        self.assertIn('Scribiva', body)
        self.assertEqual(response.headers['Cache-Control'], 'no-store')
        self.assertIn("default-src 'none'", response.headers['Content-Security-Policy'])
        self.assertEqual(response.headers['X-Frame-Options'], 'DENY')

    def test_invalid_host_is_rejected(self):
        status, raw = self.request('GET', '/', host='evil.example')
        self.assertEqual(status, 403)
        self.assertIn('Invalid host', raw.decode('utf-8'))

    def test_status_and_config_masking(self):
        with patch.object(dashboard.engine, 'state', return_value={'phase': 'idle', 'file': '/secret.wav'}), \
             patch.object(dashboard.engine, 'doctor', return_value={'healthy': True}), \
             patch.object(dashboard, 'read_config',
                          return_value={'language': 'zh', 'proxy_url': 'http://user:pass@proxy'}):
            status = self.get_json('/api/status')
            config = self.get_json('/api/config')
        self.assertEqual(status['state']['phase'], 'idle')
        self.assertNotIn('file', status['state'])
        self.assertEqual(config['settings']['proxy_url'], '(configured; edit via CLI)')
        self.assertIn('zh', config['languages'])

    def test_service_config_never_exposes_token(self):
        with patch.object(dashboard, 'read_config',
                          return_value={'service_url': 'https://api.example.test'}):
            with patch.object(dashboard, 'service_token',
                              return_value='scr_' + 's' * 40):
                status, raw = self.request('GET', '/api/config')
        body = json.loads(raw)
        self.assertEqual(status, 200)
        self.assertEqual(body['settings']['service_url'], 'https://api.example.test')
        self.assertTrue(body['service_configured'])
        self.assertTrue(body['service_token_present'])
        self.assertNotIn(b'scr_' + b's' * 40, raw)

    def test_devices_endpoint_merges_active_source(self):
        with patch.object(dashboard, 'read_config', return_value={'mic_node': 'alsa_input.usb'}), \
             patch.object(dashboard.devices, 'list_sources',
                          return_value={'devices': [{'node': 'alsa_input.usb'}],
                                        'default': 'alsa_input.usb', 'error': None}), \
             patch.object(dashboard, 'mic_node', return_value='alsa_input.usb'):
            data = self.get_json('/api/devices')
        self.assertEqual(data['active'], 'alsa_input.usb')
        self.assertEqual(data['configured'], 'alsa_input.usb')

    def test_service_endpoint_direct_mode(self):
        with patch.object(dashboard, 'read_config', return_value={}):
            data = self.get_json('/api/service')
        self.assertEqual(data, {'configured': False})

    def test_service_endpoint_reports_account_without_token(self):
        payload = {
            'service_url': 'https://api.example.test',
            'server': {'api_version': 'v1', 'features': {'billing': True}},
            'account': {'plan': 'free',
                        'usage': {'used_seconds': 12, 'remaining_seconds': 1788}},
        }
        with patch.object(dashboard, 'read_config',
                          return_value={'service_url': 'https://api.example.test'}), \
             patch.object(dashboard.providers, 'service_status', return_value=payload):
            status, raw = self.request('GET', '/api/service')
        body = json.loads(raw)
        self.assertEqual(status, 200)
        self.assertTrue(body['configured'])
        self.assertEqual(body['account']['usage']['used_seconds'], 12)
        self.assertNotIn(b'scr_', raw)

    def test_service_endpoint_surfaces_errors(self):
        error = dashboard.providers.ProviderError('Token rejected', code='AUTH_REQUIRED')
        with patch.object(dashboard, 'read_config',
                          return_value={'service_url': 'https://api.example.test'}), \
             patch.object(dashboard.providers, 'service_status', side_effect=error):
            data = self.get_json('/api/service')
        self.assertTrue(data['configured'])
        self.assertIn('Token rejected', data['error'])

    def test_checkout_creates_session(self):
        with patch.object(dashboard, 'read_config',
                          return_value={'service_url': 'https://api.example.test'}), \
             patch.object(dashboard.providers, 'service_checkout',
                          return_value={'url': 'https://checkout.stripe.com/c/pay/x',
                                        'session_id': 'cs_1'}) as checkout:
            status, raw = self.request('POST', '/api/service/checkout',
                                       token='test-token', body={'plan': 'annual'})
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(raw)['url'], 'https://checkout.stripe.com/c/pay/x')
        checkout.assert_called_once()
        self.assertEqual(checkout.call_args.args[1], 'annual')

    def test_checkout_validates_configuration_plan_and_fields(self):
        with patch.object(dashboard, 'read_config', return_value={}):
            status, _ = self.request('POST', '/api/service/checkout',
                                     token='test-token', body={'plan': 'monthly'})
        self.assertEqual(status, 400)
        with patch.object(dashboard, 'read_config',
                          return_value={'service_url': 'https://api.example.test'}):
            status, _ = self.request('POST', '/api/service/checkout',
                                     token='test-token', body={'plan': 'lifetime'})
            self.assertEqual(status, 400)
            status, _ = self.request('POST', '/api/service/checkout', token='test-token',
                                     body={'plan': 'monthly', 'extra': 1})
            self.assertEqual(status, 400)

    def test_checkout_provider_error_is_502(self):
        error = dashboard.providers.ProviderError('Stripe rejected',
                                                  code='BILLING_PROVIDER_ERROR')
        with patch.object(dashboard, 'read_config',
                          return_value={'service_url': 'https://api.example.test'}), \
             patch.object(dashboard.providers, 'service_checkout', side_effect=error):
            status, raw = self.request('POST', '/api/service/checkout',
                                       token='test-token', body={'plan': 'monthly'})
        self.assertEqual(status, 502)
        self.assertIn('Stripe rejected', raw.decode('utf-8'))

    def test_portal_creates_session_and_validates_body(self):
        with patch.object(dashboard, 'read_config',
                          return_value={'service_url': 'https://api.example.test'}), \
             patch.object(dashboard.providers, 'service_portal',
                          return_value={'url': 'https://billing.stripe.com/p/session/x'}) as portal:
            status, raw = self.request('POST', '/api/service/portal',
                                       token='test-token', body={})
            self.assertEqual(status, 200)
            self.assertEqual(json.loads(raw)['url'],
                             'https://billing.stripe.com/p/session/x')
            portal.assert_called_once()
            status, _ = self.request('POST', '/api/service/portal',
                                     token='test-token', body={'extra': 1})
            self.assertEqual(status, 400)

    def test_last_endpoint(self):
        with patch.object(dashboard.engine, 'last_result',
                          return_value={'available': True, 'text': '你好', 'raw': '你 好'}):
            data = self.get_json('/api/last')
        self.assertEqual(data['text'], '你好')

    def test_post_requires_token_and_origin(self):
        status, _ = self.request('POST', '/api/toggle', body={}, token='wrong')
        self.assertEqual(status, 403)
        status, _ = self.request('POST', '/api/toggle', body={}, token='test-token',
                                 origin='https://evil.example')
        self.assertEqual(status, 403)

    def test_settings_post_accepts_new_fields(self):
        with patch.object(dashboard, 'save_config', return_value={}) as save:
            status, raw = self.request('POST', '/api/settings', token='test-token',
                                       body={'mic_node': 'alsa_input.usb',
                                             'max_record_seconds': 120,
                                             'delete_audio_on_success': False,
                                             'service_url': 'https://api.example.test'})
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(raw), {'status': 'saved'})
        save.assert_called_once_with({'mic_node': 'alsa_input.usb',
                                      'max_record_seconds': 120,
                                      'delete_audio_on_success': False,
                                      'service_url': 'https://api.example.test'})

    def test_settings_rejects_unknown_key(self):
        status, raw = self.request('POST', '/api/settings', token='test-token',
                                   body={'api_key': 'gsk_nope'})
        self.assertEqual(status, 400)
        self.assertIn('Unsupported setting', raw.decode('utf-8'))

    def test_clear_and_copy_last(self):
        with patch.object(dashboard.engine, 'clear_last', return_value=True) as clear, \
             patch.object(dashboard.engine, 'last_result',
                          return_value={'available': True, 'text': 'hello'}):
            with patch.object(dashboard.inject, 'copy_text') as copy:
                status, raw = self.request('POST', '/api/copy-last', token='test-token', body={})
                self.assertEqual(status, 200)
                copy.assert_called_once_with('hello')
                status, raw = self.request('POST', '/api/clear-last', token='test-token', body={})
                self.assertEqual(status, 200)
                self.assertEqual(json.loads(raw), {'removed': True})
                clear.assert_called_once()

    def test_copy_last_without_result_is_400(self):
        with patch.object(dashboard.engine, 'last_result',
                          return_value={'available': False, 'text': ''}):
            status, raw = self.request('POST', '/api/copy-last', token='test-token', body={})
        self.assertEqual(status, 400)
        self.assertIn('No result yet', raw.decode('utf-8'))

    def test_invalid_content_length_is_rejected(self):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=3)
        connection.putrequest('POST', '/api/toggle')
        connection.putheader('Host', f'127.0.0.1:{self.port}')
        connection.putheader('X-Scribiva-Token', 'test-token')
        connection.putheader('Content-Length', 'not-a-number')
        connection.endheaders()
        response = connection.getresponse()
        response.read()
        connection.close()
        self.assertEqual(response.status, 413)


if __name__ == '__main__':
    unittest.main()
