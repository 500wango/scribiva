import json
import math
import tempfile
import unittest
import wave
from pathlib import Path
from unittest.mock import patch
from scribiva import audio, config, inject, providers

class ConfigTests(unittest.TestCase):
    def test_twenty_explicit_languages(self):
        self.assertEqual(len(config.LANGUAGES)-1, 20)
        self.assertIn('zh', config.LANGUAGES)
        self.assertIn('ar', config.LANGUAGES)

    def test_mode_validation(self):
        with tempfile.TemporaryDirectory() as root:
            file = Path(root)/'settings.json'
            with patch.object(config, 'CONFIG_PATH', file), patch.object(config, 'secure_dirs'):
                with self.assertRaises(ValueError):
                    config.save_config({'mode': 'translate'})
                with self.assertRaises(ValueError):
                    config.save_config({'language': 'xx'})
                conf = config.save_config({'language':'zh', 'mode':'raw'})
                self.assertEqual(conf['language'], 'zh')

class ProviderTests(unittest.TestCase):
    def test_raw_mode_never_calls_llm(self):
        with patch.object(providers, '_request', side_effect=AssertionError('LLM called')):
            value, warning = providers.correct('你好。', {'mode': 'raw'})
        self.assertEqual(value,'你好。')
        self.assertIsNone(warning)

    def test_protected_literals_are_restored(self):
        source = 'Open `app.py` then visit https://example.com and use 42.'
        def edit(_url, **kwargs):
            val = kwargs['payload']['messages'][1]['content']
            return {'choices':[{'message':{'content': val.replace('Open', 'Please open')}}]}
        with patch.object(providers, '_request', side_effect=edit):
            revised, warning = providers.correct(source, {'mode':'standard', 'grammar_model':'openai/gpt-oss-20b'})
        self.assertIsNone(warning)
        self.assertIn('`app.py`', revised)
        self.assertIn('https://example.com', revised)
        self.assertIn('42', revised)

    def test_numeric_mutation_rejected(self):
        original = 'Price 100.'
        with patch.object(providers, '_request', return_value={'choices':[{'message':{'content':'Price 200.'}}]}):
            final, warning = providers.correct(original, {'mode':'standard', 'grammar_model':'openai/gpt-oss-20b'})
        self.assertEqual(final, original)
        self.assertIn('GRAMMAR_SKIPPED', warning)

    def test_403_cloudflare_does_not_retry(self):
        seen=[]
        def fail_run(cmd, **kwargs):
            seen.append(cmd)
            output = Path(cmd[cmd.index('--output')+1])
            output.write_text(json.dumps({'error_code':1010, 'error_name':'browser_signature_banned','detail':'blocked'}))
            import subprocess
            return subprocess.CompletedProcess(cmd, 0, stdout='403', stderr='')
        with patch.object(providers, 'api_key', return_value='gsk_dummy'), patch.object(providers.subprocess, 'run', side_effect=fail_run):
            with self.assertRaisesRegex(providers.ProviderError,'1010'):
                providers._request('/test', fields=['model=whisper-large-v3'])
        self.assertEqual(len(seen),1)
        self.assertNotIn('gsk_dummy', ' '.join(seen[0]))

class InjectionTests(unittest.TestCase):
    def test_no_auto_repeat_on_dispatch_uncertainty(self):
        import subprocess
        with patch.object(inject,'_clipboard_text',return_value=None),patch.object(inject,'copy_text') as copy:
            with patch.object(inject,'eitype_command',return_value='/fake/eitype'):
                with patch.object(inject.subprocess,'run',side_effect=subprocess.TimeoutExpired('eitype',12)) as run:
                    with self.assertRaisesRegex(RuntimeError, 'do not auto-retry'):
                        inject.dispatch('hello', restore=False)
                    copy.assert_called_once_with('hello')
                    run.assert_called_once()

class AudioTests(unittest.TestCase):
    def test_pcm_quality(self):
        with tempfile.TemporaryDirectory() as temp:
            path=Path(temp)/'test.wav'
            with wave.open(str(path),'wb') as f:
                f.setnchannels(1)
                f.setsampwidth(2)
                f.setframerate(16000)
                import array
                vals=array.array('h',(int(math.sin(i/11)*5000) for i in range(16000)))
                f.writeframes(vals.tobytes())
            q=audio.quality(path)
            self.assertEqual(q['duration'],1)
            self.assertGreater(q['rms'],1000)

if __name__ == '__main__':
    unittest.main()
