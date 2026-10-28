"""v1.0 server features: grace quota, dictionary, commands, hallucination,
output script and device/magic-link login (docs/05-API-AND-DATA.md §8)."""
from __future__ import annotations
import email
import os
import re
import sys
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))

from scribiva_server import postprocess, textguard  # noqa: E402
from server_support import ServerTestCase, wav_bytes  # noqa: E402


class PostprocessTests(unittest.TestCase):
    def test_spoken_commands(self):
        self.assertEqual(postprocess.apply_commands('第一项，换行，第二项'), '第一项\n第二项')
        self.assertEqual(postprocess.apply_commands('标题 新段落 正文'), '标题\n\n正文')
        self.assertEqual(postprocess.apply_commands('列表项 苹果 列表项 香蕉'), '- 苹果\n- 香蕉')
        self.assertEqual(postprocess.apply_commands('Hello. New line. World'), 'Hello\nWorld')
        self.assertEqual(postprocess.apply_commands('no commands here'), 'no commands here')

    def test_known_phantoms_and_silent_segments(self):
        self.assertTrue(postprocess.is_suspected_hallucination('谢谢观看。'))
        self.assertTrue(postprocess.is_suspected_hallucination('Thanks for watching!'))
        self.assertTrue(postprocess.is_suspected_hallucination(
            '字幕由 Amara.org 社区提供'))
        self.assertFalse(postprocess.is_suspected_hallucination('谢谢观看这个演示的各位同事'))
        self.assertTrue(postprocess.is_suspected_hallucination('字幕志愿者 李宗盛谢谢大家'))
        self.assertFalse(postprocess.is_suspected_hallucination('明天下午把字幕文件发给我'))
        silent = [{'no_speech_prob': 0.9, 'avg_logprob': -1.4}]
        self.assertTrue(postprocess.is_suspected_hallucination('some words', silent))
        mixed = silent + [{'no_speech_prob': 0.01, 'avg_logprob': -0.2}]
        self.assertFalse(postprocess.is_suspected_hallucination('some words', mixed))

    def test_dictionary_validation(self):
        self.assertEqual(postprocess.clean_terms([' Kubernetes ', 'kubernetes', '', 'Scribiva']),
                         ['Kubernetes', 'Scribiva'])
        with self.assertRaises(postprocess.DictionaryError):
            postprocess.clean_terms(['x' * 65])
        with self.assertRaises(postprocess.DictionaryError):
            postprocess.clean_terms(['bad\x00term'])
        with self.assertRaises(postprocess.DictionaryError):
            postprocess.clean_terms('not a list')
        prompt = postprocess.asr_prompt(['term%03d' % i for i in range(200)])
        self.assertLessEqual(len(prompt), postprocess.PROMPT_LIMIT)

    def test_guard_protects_terms_and_number_values(self):
        masked, parts = textguard.protect('部署到 Kubernetes 集群', ['Kubernetes'])
        self.assertNotIn('Kubernetes', masked)
        self.assertEqual(list(parts.values()), ['Kubernetes'])
        self.assertEqual(textguard.restore('明天三点半开会', '明天 3:30 开会。', {}),
                         '明天 3:30 开会。')
        self.assertEqual(textguard.restore('总价 1,000 元', '总价 1000 元。', {}), '总价 1000 元。')
        with self.assertRaises(textguard.GuardError):
            textguard.restore('版本 2.5 发布', '版本 2.6 发布', {})


class GraceQuotaTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server(free_seconds=10, grace_seconds=5)

    def test_last_recording_is_charged_down_to_zero_then_blocked(self):
        status, body, _ = self.dictation(audio=wav_bytes(8.2), key='job-grace-0001')
        self.assertEqual(status, 200, body)
        self.assertEqual(body['billing']['charged_audio_seconds'], 9)
        status, body, _ = self.dictation(audio=wav_bytes(4.5), key='job-grace-0002')
        self.assertEqual(status, 200, body)
        self.assertEqual(body['billing']['charged_audio_seconds'], 1)
        self.assertEqual(body['billing']['audio_seconds'], 5)
        self.assertEqual(body['billing']['remaining_seconds'], 0)
        self.assertTrue(body['billing']['resets_at'].endswith('+00:00'))
        status, body, _ = self.dictation(audio=wav_bytes(1.2), key='job-grace-0003')
        self.assertEqual(status, 402)
        _, usage, _ = self.request('GET', '/v1/me/usage')
        self.assertEqual(usage['used_seconds'], 10)
        self.assertEqual(usage['grace_seconds'], 5)

    def test_overshoot_beyond_grace_is_rejected(self):
        status, body, _ = self.dictation(audio=wav_bytes(16.5), key='job-grace-0004')
        self.assertEqual(status, 402, body)
        self.assertEqual(self.provider.transcribe_calls, [])


class DictionaryAndPipelineTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()

    def test_dictionary_roundtrip_and_use(self):
        status, body, _ = self.request('GET', '/v1/me/dictionary')
        self.assertEqual((status, body['terms']), (200, []))
        status, body, _ = self.request('PUT', '/v1/me/dictionary',
                                       body={'terms': ['Kubernetes', 'Scribiva', 'kubernetes']})
        self.assertEqual(status, 200, body)
        self.assertEqual(body['terms'], ['Kubernetes', 'Scribiva'])
        status, body, _ = self.request('PUT', '/v1/me/dictionary', body={'terms': [1]})
        self.assertEqual((status, body['error']['code']), (400, 'INVALID_DICTIONARY'))
        status, body, _ = self.dictation(key='job-dict-0001')
        self.assertEqual(status, 200, body)
        self.assertEqual(self.provider.transcribe_calls[-1]['prompt'], 'Kubernetes, Scribiva')
        self.assertEqual(self.provider.correct_calls[-1]['terms'], ['Kubernetes', 'Scribiva'])

    def test_commands_run_before_grammar_and_can_be_disabled(self):
        self.provider.transcribe_result = '买菜 换行 做饭'
        self.provider.correct_result = ('买菜\n做饭', None)
        status, body, _ = self.dictation(key='job-cmd-00001')
        self.assertEqual(status, 200, body)
        self.assertEqual(self.provider.correct_calls[-1]['text'], '买菜\n做饭')
        self.assertEqual(body['raw_text'], '买菜 换行 做饭')
        status, body, _ = self.dictation(key='job-cmd-00002', fields={
            'language': 'zh', 'grammar_mode': 'raw', 'dictation_commands': '0',
            'client_job_id': 'job-cmd-00002', 'client_version': 'test'})
        self.assertEqual(body['output_text'], '买菜 换行 做饭')

    def test_hallucination_is_flagged_and_not_edited(self):
        self.provider.transcribe_result = '谢谢观看'
        status, body, _ = self.dictation(key='job-hall-0001')
        self.assertEqual(status, 200, body)
        self.assertTrue(body['suspected_hallucination'])
        self.assertTrue(any(w.startswith('SUSPECTED_HALLUCINATION') for w in body['warnings']))
        self.assertEqual(self.provider.correct_calls, [])
        status, replay, _ = self.dictation(key='job-hall-0001')
        self.assertTrue(replay['suspected_hallucination'])

    def test_output_script_validation_and_conversion(self):
        status, body, _ = self.dictation(key='job-script-001', fields={
            'language': 'zh', 'grammar_mode': 'raw', 'output_script': 'latin',
            'client_job_id': 'job-script-001', 'client_version': 'test'})
        self.assertEqual((status, body['error']['code']), (400, 'INVALID_OUTPUT_SCRIPT'))
        self.provider.transcribe_result = '這個軟體很好用'
        status, body, _ = self.dictation(key='job-script-002', fields={
            'language': 'zh', 'grammar_mode': 'raw', 'output_script': 'hans',
            'client_job_id': 'job-script-002', 'client_version': 'test'})
        self.assertEqual(status, 200, body)
        if postprocess._converter('hans') is None:
            self.assertTrue(any(w.startswith('SCRIPT_CONVERSION_UNAVAILABLE')
                                for w in body['warnings']))
        else:
            self.assertEqual(body['output_text'], '这个软体很好用')


class BillingPagesTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()

    def test_stripe_return_pages_exist_and_grant_nothing(self):
        for path in ('/billing/success?session_id=cs_test_1', '/billing/cancel',
                     '/billing/portal-return'):
            status, page, headers = self.request('GET', path, token=False)
            self.assertEqual(status, 200, path)
            self.assertIn('Scribiva', page)
            self.assertIn("default-src 'none'", headers['Content-Security-Policy'])
        _, me, _ = self.request('GET', '/v1/me')
        self.assertEqual(me['plan'], 'free')


class OutcomeAndMetricsTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        self.start_server()

    def test_outcome_is_recorded_once_and_validated(self):
        ids = []
        for i in range(3):
            _, body, _ = self.dictation(key=f'job-outcome-{i:04d}')
            ids.append(body['job_id'])
        status, body, _ = self.request('POST', f'/v1/dictations/{ids[0]}/outcome',
                                       body={'outcome': 'inserted'})
        self.assertEqual((status, body['recorded']), (200, True))
        status, body, _ = self.request('POST', f'/v1/dictations/{ids[0]}/outcome',
                                       body={'outcome': 'clipboard_only'})
        self.assertEqual((status, body['recorded']), (200, False))
        status, body, _ = self.request('POST', f'/v1/dictations/{ids[1]}/outcome',
                                       body={'outcome': 'text: hello'})
        self.assertEqual((status, body['error']['code']), (400, 'INVALID_OUTCOME'))
        status, body, _ = self.request('POST', '/v1/dictations/job_missing/outcome',
                                       body={'outcome': 'inserted'})
        self.assertEqual(status, 404)
        self.request('POST', f'/v1/dictations/{ids[2]}/outcome',
                     body={'outcome': 'dispatched_unverified'})
        from scribiva_server import db
        conn = db.connect(self.settings.db_path)
        try:
            weeks = db.weekly_metrics(conn)
        finally:
            conn.close()
        self.assertEqual(len(weeks), 1)
        w = weeks[0]
        self.assertEqual((w['active_accounts'], w['north_star_accounts'], w['dictations']),
                         (1, 1, 3))
        self.assertEqual((w['inserted'], w['pasted'], w['copied']), (1, 1, 0))


class DeviceLoginTests(ServerTestCase, unittest.TestCase):
    def setUp(self):
        import tempfile
        self.outbox = tempfile.TemporaryDirectory()
        self.addCleanup(self.outbox.cleanup)
        self.start_server(mail_outbox=Path(self.outbox.name))

    def _link(self) -> str:
        files = sorted(Path(self.outbox.name).glob('*.eml'))
        self.assertTrue(files)
        message = email.message_from_bytes(files[-1].read_bytes())
        body = message.get_payload(decode=True).decode('utf-8')
        return re.search(r'https://example\.test(/auth/verify\?token=[A-Za-z0-9_-]+)',
                         body).group(1)

    def _form(self, path: str, data: dict):
        return self.request('POST', path, body=urlencode(data).encode(), token=False,
                            headers={'Content-Type': 'application/x-www-form-urlencoded'})

    def test_full_device_login_flow(self):
        status, start, _ = self.request('POST', '/v1/auth/device',
                                        body={'client_name': 'test'}, token=False)
        self.assertEqual(status, 200, start)
        self.assertRegex(start['user_code'], r'^[A-Z]{4}-[A-Z]{4}$')
        self.assertEqual(start['verification_uri_complete'],
                         'https://example.test/login?code=' + start['user_code'])
        status, poll, _ = self.request('POST', '/v1/auth/device/token',
                                       body={'device_code': start['device_code']}, token=False)
        self.assertEqual((status, poll['error']['code']), (400, 'AUTHORIZATION_PENDING'))

        status, page, headers = self.request('GET', '/login?code=' + start['user_code'],
                                             token=False)
        self.assertEqual(status, 200)
        self.assertIn(start['user_code'], page)
        self.assertIn("default-src 'none'", headers['Content-Security-Policy'])
        status, page, _ = self._form('/login', {'email': 'New@Example.com',
                                                'user_code': start['user_code'].lower()})
        self.assertEqual(status, 200, page)

        verify = self._link()
        status, page, _ = self.request('GET', verify, token=False)
        self.assertEqual(status, 200)
        self.assertIn(start['user_code'], page)
        # GET alone must not approve (mail scanners prefetch links).
        status, poll, _ = self.request('POST', '/v1/auth/device/token',
                                       body={'device_code': start['device_code']}, token=False)
        self.assertIn(poll['error']['code'], ('AUTHORIZATION_PENDING', 'SLOW_DOWN'))

        token = verify.split('token=')[1]
        status, page, _ = self._form('/auth/verify', {'token': token})
        self.assertEqual(status, 200, page)
        status, granted, _ = self.request('POST', '/v1/auth/device/token',
                                          body={'device_code': start['device_code']}, token=False)
        self.assertEqual(status, 200, granted)
        self.assertTrue(granted['access_token'].startswith('scr_'))
        status, me, _ = self.request('GET', '/v1/me', headers={
            'Authorization': 'Bearer ' + granted['access_token']}, token=False)
        self.assertEqual((status, me['user']['email']), (200, 'new@example.com'))
        # The device code and the link are single use.
        status, again, _ = self.request('POST', '/v1/auth/device/token',
                                        body={'device_code': start['device_code']}, token=False)
        self.assertEqual(again['error']['code'], 'INVALID_DEVICE_CODE')
        status, page, _ = self._form('/auth/verify', {'token': token})
        self.assertEqual(status, 400)

    def test_magic_link_rejects_unknown_code_and_bad_email(self):
        status, body, _ = self.request('POST', '/v1/auth/magic-link', token=False,
                                       body={'email': 'a@example.com', 'user_code': 'BCDF-GHJK'})
        self.assertEqual((status, body['error']['code']), (400, 'INVALID_USER_CODE'))
        status, body, _ = self.request('POST', '/v1/auth/magic-link', token=False,
                                       body={'email': 'not-an-email', 'user_code': 'BCDF-GHJK'})
        self.assertEqual((status, body['error']['code']), (400, 'INVALID_EMAIL'))
        self.assertEqual(list(Path(self.outbox.name).glob('*.eml')), [])

    def test_deleted_account_reuses_usage_on_relogin(self):
        from scribiva_server import auth, db
        conn = db.connect(self.settings.db_path)
        try:
            user = db.get_or_create_user_by_email(conn, 'reuse@example.com')
            token = auth.issue_token(conn, user['id'], 30)
        finally:
            conn.close()
        headers = {'Authorization': 'Bearer ' + token}
        self.dictation(key='job-reuse-0001', headers=headers)
        status, _, _ = self.request('DELETE', '/v1/me/data', body={'confirm': 'DELETE'},
                                    headers=headers, token=False)
        self.assertEqual(status, 202)
        conn = db.connect(self.settings.db_path)
        try:
            again = db.get_or_create_user_by_email(conn, 'REUSE@example.com')
            self.assertEqual((again['id'], again['created']), (user['id'], False))
            self.assertEqual(db.usage(conn, user['id'])['used_seconds'], 2)
        finally:
            conn.close()

    def test_login_not_configured_without_mail_on_remote_bind(self):
        self.server.shutdown()
        self.server.server_close()
        self.start_server(mail_outbox=None, smtp_host='', allow_remote=True)
        status, body, _ = self.request('POST', '/v1/auth/device', body={}, token=False)
        self.assertEqual((status, body['error']['code']), (503, 'LOGIN_NOT_CONFIGURED'))

    def test_loopback_dev_login_is_approved_from_the_cli(self):
        from scribiva_server.__main__ import main as server_main
        self.server.shutdown()
        self.server.server_close()
        self.start_server(mail_outbox=None, smtp_host='', public_base_url='')
        status, start, _ = self.request('POST', '/v1/auth/device', body={}, token=False)
        self.assertEqual(status, 200, start)
        self.assertTrue(start['verification_uri'].startswith('http://127.0.0.1:'))
        with patch.dict(os.environ, {'SCRIBIVA_DB': str(self.settings.db_path)}):
            rc = server_main(['approve-device', '--code', start['user_code'].lower(),
                              '--email', 'dev@example.com'])
        self.assertEqual(rc, 0)
        status, granted, _ = self.request('POST', '/v1/auth/device/token',
                                          body={'device_code': start['device_code']}, token=False)
        self.assertEqual(status, 200, granted)
        self.assertTrue(granted['access_token'].startswith('scr_'))


if __name__ == '__main__':
    unittest.main()
