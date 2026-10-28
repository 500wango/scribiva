"""Loopback-first HTTP API around DictationService (stdlib only)."""
from __future__ import annotations
import json
import logging
import secrets
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

from .multipart import MultipartError, parse_multipart
from .service import ApiError, DictationService

logger = logging.getLogger('scribiva_server')
JSON_TYPE = 'application/json; charset=utf-8'
MAX_JSON_BODY = 64 * 1024


class ApiHandler(BaseHTTPRequestHandler):
    timeout = 30
    server_version = 'ScribivaAPI/1'
    sys_version = ''

    # -------------------------------------------------------------- plumbing

    def handle_one_request(self):
        self.request_id = 'req_' + secrets.token_hex(8)
        self.started = time.monotonic()
        self.user_id = None
        super().handle_one_request()

    def log_message(self, fmt, *args):
        # Suppress the default stderr log; structured logging happens in _send.
        return

    def _send(self, status: int, payload, headers: dict | None = None,
              content_type: str = JSON_TYPE):
        if isinstance(payload, bytes):
            body = payload
        else:
            body = json.dumps(payload, ensure_ascii=False).encode('utf-8')
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Referrer-Policy', 'no-referrer')
        self.send_header('Content-Length', str(len(body)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        if self.close_connection:
            self.send_header('Connection', 'close')
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True
        elapsed = int((time.monotonic() - self.started) * 1000) if hasattr(self, 'started') else 0
        logger.info('api method=%s path=%s status=%d user=%s request=%s ms=%d',
                    self.command, urlsplit(self.path).path, status,
                    self.user_id or '-', getattr(self, 'request_id', '-'), elapsed)

    def _api_error(self, error: ApiError):
        payload = {
            'error': {
                'code': error.code,
                'message': error.message,
                'retryable': error.retryable,
                'request_id': getattr(self, 'request_id', ''),
            }
        }
        payload['error'].update(error.extra)
        headers = {}
        if error.retry_after:
            headers['Retry-After'] = str(error.retry_after)
        if error.status == 401:
            headers['WWW-Authenticate'] = 'Bearer'
        self._send(error.status, payload, headers=headers)

    def _read_body(self, limit: int) -> bytes:
        if 'chunked' in self.headers.get('Transfer-Encoding', '').lower():
            self.close_connection = True
            raise ApiError(411, 'LENGTH_REQUIRED', 'Chunked uploads are not supported')
        raw_length = self.headers.get('Content-Length')
        if raw_length is None:
            self.close_connection = True
            raise ApiError(411, 'LENGTH_REQUIRED', 'Content-Length is required')
        try:
            length = int(raw_length)
        except ValueError as exc:
            self.close_connection = True
            raise ApiError(400, 'INVALID_CONTENT_LENGTH', 'Content-Length is invalid') from exc
        if length < 0:
            self.close_connection = True
            raise ApiError(400, 'INVALID_CONTENT_LENGTH', 'Content-Length is invalid')
        if length > limit:
            self.close_connection = True
            raise ApiError(413, 'REQUEST_TOO_LARGE', 'Request body exceeds the limit')
        remaining = length
        chunks = []
        while remaining:
            chunk = self.rfile.read(min(65536, remaining))
            if not chunk:
                self.close_connection = True
                raise ApiError(400, 'INCOMPLETE_BODY', 'Request body ended early')
            chunks.append(chunk)
            remaining -= len(chunk)
        return b''.join(chunks)

    def _json_body(self, limit: int = MAX_JSON_BODY) -> dict:
        body = self._read_body(limit)
        if not body:
            return {}
        if 'application/json' not in self.headers.get('Content-Type', '').lower():
            raise ApiError(415, 'UNSUPPORTED_MEDIA_TYPE', 'Expected application/json')
        try:
            value = json.loads(body.decode('utf-8'))
        except (ValueError, UnicodeDecodeError) as exc:
            raise ApiError(400, 'INVALID_JSON', 'Request body is not valid JSON') from exc
        if not isinstance(value, dict):
            raise ApiError(400, 'INVALID_JSON', 'Request body must be a JSON object')
        return value

    def _authenticate(self) -> dict | None:
        try:
            user = self.server.service.authenticate(self.headers.get('Authorization'))
        except ApiError as exc:
            self._api_error(exc)
            return None
        self.user_id = user['id']
        return user

    def _fail_unexpected(self):
        logger.exception('api_unhandled request=%s path=%s',
                         getattr(self, 'request_id', '-'), urlsplit(self.path).path)
        self._api_error(ApiError(500, 'INTERNAL_ERROR', 'Internal server error'))

    # -------------------------------------------------------------------- GET

    def do_GET(self):
        try:
            path = urlsplit(self.path).path.rstrip('/') or '/'
            service = self.server.service
            if path == '/healthz':
                return self._send(200, service.health())
            if path == '/v1/config/public':
                return self._send(200, service.public_config())
            if path == '/':
                return self._send(200,
                                  b'Scribiva API v1. See /v1/config/public and /healthz.\n',
                                  content_type='text/plain; charset=utf-8')
            if path == '/v1/me':
                user = self._authenticate()
                if user:
                    return self._send(200, service.me(user))
                return
            if path == '/v1/me/usage':
                user = self._authenticate()
                if user:
                    return self._send(200, service.usage(user))
                return
            if path == '/v1/me/dictations':
                user = self._authenticate()
                if user:
                    return self._send(200, service.recent(user))
                return
            parts = path.strip('/').split('/')
            if len(parts) == 3 and parts[0] == 'v1' and parts[1] == 'dictations':
                user = self._authenticate()
                if user:
                    status, payload = service.get_job(user, parts[2])
                    return self._send(status, payload)
                return
            raise ApiError(404, 'NOT_FOUND', 'Route not found')
        except ApiError as error:
            self._api_error(error)
        except Exception:
            self._fail_unexpected()

    # ------------------------------------------------------------------- POST

    def do_POST(self):
        try:
            path = urlsplit(self.path).path.rstrip('/') or '/'
            service = self.server.service
            if path == '/v1/webhooks/stripe':
                body = self._read_body(1_000_000)
                result = service.stripe_webhook(body, self.headers.get('Stripe-Signature'))
                return self._send(200, result)

            if path == '/v1/dictations':
                user = self._authenticate()
                if not user:
                    return
                body = self._read_body(self.server.settings.max_request_bytes)
                try:
                    fields, files = parse_multipart(
                        body, self.headers.get('Content-Type', ''))
                except MultipartError as exc:
                    raise ApiError(400, 'INVALID_MULTIPART', str(exc)) from exc
                status, payload = service.create_dictation(
                    user, fields=fields, files=files,
                    idempotency_key=self.headers.get('Idempotency-Key'),
                    request_id=self.request_id)
                return self._send(status, payload)

            parts = path.strip('/').split('/')
            if (len(parts) == 4 and parts[0] == 'v1' and parts[1] == 'dictations'
                    and parts[3] == 'cancel'):
                user = self._authenticate()
                if not user:
                    return
                return self._send(200, service.cancel_job(user, parts[2]))

            if path == '/v1/billing/checkout-session':
                user = self._authenticate()
                if not user:
                    return
                body = self._json_body()
                return self._send(200, service.checkout(user, body))
            if path == '/v1/billing/portal-session':
                user = self._authenticate()
                if not user:
                    return
                return self._send(200, service.portal(user))

            raise ApiError(404, 'NOT_FOUND', 'Route not found')
        except ApiError as error:
            self._api_error(error)
        except Exception:
            self._fail_unexpected()

    # ----------------------------------------------------------------- DELETE

    def do_DELETE(self):
        try:
            path = urlsplit(self.path).path.rstrip('/') or '/'
            if path != '/v1/me/data':
                raise ApiError(404, 'NOT_FOUND', 'Route not found')
            user = self._authenticate()
            if not user:
                return
            body = self._json_body()
            return self._send(202, self.server.service.delete_data(user, body))
        except ApiError as error:
            self._api_error(error)
        except Exception:
            self._fail_unexpected()

    # --------------------------------------------------------------- methods

    def _method_not_allowed(self):
        self._api_error(ApiError(405, 'METHOD_NOT_ALLOWED', 'Method not allowed'))

    do_PUT = _method_not_allowed
    do_PATCH = _method_not_allowed
    do_OPTIONS = _method_not_allowed


class ApiServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, address, handler, service, settings):
        super().__init__(address, handler)
        self.service = service
        self.settings = settings


def create_server(settings, service: DictationService | None = None) -> ApiServer:
    service = service or DictationService(settings)
    return ApiServer((settings.host, settings.port), ApiHandler, service, settings)
