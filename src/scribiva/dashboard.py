"""Local-only developer UI; never exposed on LAN."""
from __future__ import annotations
import json
import secrets
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit
from .config import LANGUAGES, mic_node, read_config, save_config, service_token
from . import devices, engine, inject, providers

HTML = Path(__file__).resolve().parents[2] / 'web' / 'index.html'
MAX_BODY = 4096
CSP = ("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; "
       "connect-src 'self'; img-src 'self' data:; base-uri 'none'; frame-ancestors 'none'")


def make_server(token: str | None = None):
    """Create the loopback server. Exposed separately so tests can bind port 0."""
    token = token or secrets.token_urlsafe(24)

    class Handler(BaseHTTPRequestHandler):
        def _valid_host(self):
            return self.headers.get('Host', '') == f'127.0.0.1:{self.server.server_port}'

        def _send(self, code, data, kind='application/json; charset=utf-8'):
            payload = data if isinstance(data, bytes) else json.dumps(data, ensure_ascii=False).encode()
            self.send_response(code)
            self.send_header('Content-Type', kind)
            self.send_header('Cache-Control', 'no-store')
            self.send_header('X-Content-Type-Options', 'nosniff')
            self.send_header('X-Frame-Options', 'DENY')
            self.send_header('Referrer-Policy', 'no-referrer')
            self.send_header('Content-Security-Policy', CSP)
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def do_GET(self):
            if not self._valid_host():
                return self._send(403, {'error': 'Invalid host'})
            path = urlsplit(self.path).path
            if path == '/':
                source = HTML.read_text(encoding='utf-8')
                self._send(200, source.replace('__CSRF_TOKEN__', token).encode('utf-8'),
                           'text/html; charset=utf-8')
            elif path == '/api/status':
                info = engine.state()
                for key in ('pid', 'file'):
                    info.pop(key, None)
                self._send(200, {'state': info, 'doctor': engine.doctor()})
            elif path == '/api/config':
                info = read_config()
                if info.get('proxy_url'):
                    info['proxy_url'] = '(configured; edit via CLI)'
                try:
                    token_present = bool(service_token())
                except (RuntimeError, OSError):
                    token_present = False
                self._send(200, {'settings': info, 'languages': LANGUAGES,
                                 'service_configured': bool(info.get('service_url')),
                                 'service_token_present': token_present})
            elif path == '/api/devices':
                conf = read_config()
                info = devices.list_sources()
                info['configured'] = conf.get('mic_node', '').strip()
                try:
                    info['active'] = mic_node(conf, info)
                except RuntimeError:
                    info['active'] = None
                self._send(200, info)
            elif path == '/api/last':
                self._send(200, engine.last_result())
            elif path == '/api/service':
                try:
                    conf = read_config()
                except (ValueError, OSError) as exc:
                    return self._send(200, {'configured': False,
                                            'error': str(exc)[:240]})
                if not providers.service_configured(conf):
                    return self._send(200, {'configured': False})
                try:
                    data = providers.service_status(conf)
                except (providers.ProviderError, RuntimeError, OSError) as exc:
                    return self._send(200, {'configured': True,
                                            'error': str(exc)[:240]})
                return self._send(200, {'configured': True, **data})
            else:
                self._send(404, {'error': 'Not found'})

        def do_POST(self):
            if not self._valid_host() or self.headers.get('X-Scribiva-Token') != token:
                return self._send(403, {'error': 'Invalid request token or host'})
            origin = self.headers.get('Origin')
            if origin and origin != f'http://127.0.0.1:{self.server.server_port}':
                return self._send(403, {'error': 'Unexpected origin'})
            try:
                size = int(self.headers.get('Content-Length', '0') or '0')
            except ValueError:
                return self._send(413, {'error': 'Invalid Content-Length'})
            if not 0 <= size <= MAX_BODY:
                return self._send(413, {'error': 'Request too large'})
            try:
                raw = self.rfile.read(size) if size else b''
                value = json.loads(raw) if raw else {}
                route = urlsplit(self.path).path
                if route == '/api/settings':
                    accepted = {'language', 'mode', 'asr_model', 'auto_insert',
                                'restore_text_clipboard', 'mic_node', 'max_record_seconds',
                                'delete_audio_on_success', 'service_url'}
                    if not isinstance(value, dict) or set(value) - accepted:
                        raise ValueError('Unsupported setting')
                    save_config(value)
                    result = {'status': 'saved'}
                elif route == '/api/toggle':
                    result = {'status': engine.toggle()}
                elif route == '/api/cancel':
                    engine.cancel()
                    result = {'status': 'cancelled'}
                elif route == '/api/discard':
                    result = {'status': engine.discard()}
                elif route == '/api/retry':
                    engine.retry()
                    result = {'status': 'retrying'}
                elif route == '/api/clear-last':
                    result = {'removed': engine.clear_last()}
                elif route == '/api/service/checkout':
                    if not isinstance(value, dict) or set(value) - {'plan'}:
                        raise ValueError('Unsupported checkout request')
                    plan = value.get('plan') or 'monthly'
                    if plan not in ('monthly', 'annual'):
                        raise ValueError('Unsupported plan')
                    conf = read_config()
                    if not providers.service_configured(conf):
                        raise ValueError('Service is not configured')
                    try:
                        result = providers.service_checkout(conf, plan)
                    except providers.ProviderError as exc:
                        return self._send(502, {'error': str(exc)[:200]})
                elif route == '/api/service/portal':
                    if not isinstance(value, dict) or value:
                        raise ValueError('Unsupported portal request')
                    conf = read_config()
                    if not providers.service_configured(conf):
                        raise ValueError('Service is not configured')
                    try:
                        result = providers.service_portal(conf)
                    except providers.ProviderError as exc:
                        return self._send(502, {'error': str(exc)[:200]})
                elif route == '/api/copy-last':
                    data = engine.last_result()
                    if not data['text']:
                        raise ValueError('No result yet')
                    inject.copy_text(data['text'])
                    result = {'status': 'copied'}
                else:
                    return self._send(404, {'error': 'Not found'})
                self._send(200, result)
            except (ValueError, RuntimeError, OSError) as e:
                self._send(400, {'error': str(e)[:200]})

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.daemon_threads = True
    return server, token


def serve():
    server, _token = make_server()
    address = f'http://127.0.0.1:{server.server_port}/'
    print('Scribiva local dashboard:', address, flush=True)
    webbrowser.open(address)
    try:
        server.serve_forever(poll_interval=.25)
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
