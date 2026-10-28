"""The client and server ship identical copies of the deterministic text
modules (the server is deployed without the client package)."""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class SharedModuleTests(unittest.TestCase):
    def test_copies_are_identical(self):
        for name in ('postprocess.py', 'textguard.py'):
            client = (ROOT / 'src' / 'scribiva' / name).read_text(encoding='utf-8')
            server = (ROOT / 'server' / 'scribiva_server' / name).read_text(encoding='utf-8')
            self.assertEqual(client, server, f'{name} differs between src/ and server/')


if __name__ == '__main__':
    unittest.main()
