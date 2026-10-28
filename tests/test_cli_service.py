import io
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import patch

from scribiva import cli

CONFIG = {'service_url': 'https://api.example.test'}


class CliServiceTests(unittest.TestCase):
    def run_cli(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = cli.main(argv)
        return code, out.getvalue(), err.getvalue()

    def test_billing_prints_checkout_url_for_selected_plan(self):
        with patch.object(cli, 'read_config', return_value=CONFIG), \
             patch.object(cli.providers, 'service_checkout',
                          return_value={'url': 'https://checkout.stripe.com/x'}) as checkout:
            code, out, err = self.run_cli(
                ['service', 'billing', '--plan', 'annual'])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), 'https://checkout.stripe.com/x')
        self.assertIn('webhook', err.lower())
        self.assertEqual(checkout.call_args.args[1], 'annual')

    def test_portal_prints_portal_url(self):
        with patch.object(cli, 'read_config', return_value=CONFIG), \
             patch.object(cli.providers, 'service_portal',
                          return_value={'url': 'https://billing.stripe.com/x'}):
            code, out, _ = self.run_cli(['service', 'portal'])
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), 'https://billing.stripe.com/x')

    def test_delete_requires_explicit_confirmation(self):
        with patch.object(cli, 'read_config', return_value=CONFIG), \
             patch.object(cli.providers, 'service_delete') as delete:
            code, _, err = self.run_cli(['service', 'delete'])
        self.assertEqual(code, 1)
        self.assertIn('--confirm', err)
        delete.assert_not_called()

    def test_delete_clears_local_credentials(self):
        with patch.object(cli, 'read_config', return_value=CONFIG), \
             patch.object(cli.providers, 'service_delete',
                          return_value={'status': 'deletion_requested'}), \
             patch.object(cli, 'clear_service_token', return_value=True) as clear, \
             patch.object(cli, 'save_config') as save:
            code, out, _ = self.run_cli(['service', 'delete', '--confirm'])
        self.assertEqual(code, 0)
        clear.assert_called_once()
        save.assert_called_once_with({'service_url': ''})
        self.assertIn('deletion_requested', out)

    def test_logout_clears_token_and_url(self):
        with patch.object(cli, 'clear_service_token', return_value=True) as clear, \
             patch.object(cli, 'save_config') as save:
            code, out, _ = self.run_cli(['service', 'logout'])
        self.assertEqual(code, 0)
        clear.assert_called_once()
        save.assert_called_once_with({'service_url': ''})
        self.assertIn('direct Groq', out)


if __name__ == '__main__':
    unittest.main()
