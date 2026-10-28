"""Device authorization with email magic links (FR-019).

Flow (docs/05-API-AND-DATA.md §8.1): the desktop client starts a device
authorization, opens the browser on /login, the user enters an email, clicks
the emailed link and confirms; the client's poll then receives a Bearer token.
Secrets (device_code, magic-link token) are stored only as SHA-256 hashes.
"""
from __future__ import annotations
import logging
import re
import secrets

from . import auth, db, mailer
from .config import Settings

logger = logging.getLogger('scribiva_server')

# No vowels or look-alike characters: avoids words and O/0, I/1 confusion.
USER_CODE_ALPHABET = 'BCDFGHJKLMNPQRSTVWXZ'
USER_CODE_RE = re.compile(r'^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$')
EMAIL_RE = re.compile(r'^[A-Za-z0-9._%+-]{1,64}@[A-Za-z0-9.-]{1,190}\.[A-Za-z]{2,24}$')
DEVICE_CODE_RE = re.compile(r'^[A-Za-z0-9_-]{32,64}$')
MAGIC_TOKEN_RE = re.compile(r'^[A-Za-z0-9_-]{32,64}$')
POLL_INTERVAL = 3
EMAILS_PER_WINDOW = 3
EMAIL_WINDOW_SECONDS = 900


class LoginError(RuntimeError):
    def __init__(self, status: int, code: str, message: str):
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message


def normalise_user_code(value: str) -> str:
    compact = re.sub(r'[\s-]', '', (value or '').upper())
    return compact[:4] + '-' + compact[4:] if len(compact) == 8 else compact


class LoginService:
    def __init__(self, settings: Settings):
        self.settings = settings

    def _connect(self):
        return db.connect(self.settings.db_path)

    def _require_configured(self):
        if not self.settings.public_base_url or not self.settings.mail_configured:
            raise LoginError(503, 'LOGIN_NOT_CONFIGURED',
                             'Email login is not configured on this server')

    def start_device(self, client_name: str | None) -> dict:
        self._require_configured()
        device_code = secrets.token_urlsafe(32)
        conn = self._connect()
        try:
            for _ in range(5):
                raw = ''.join(secrets.choice(USER_CODE_ALPHABET) for _ in range(8))
                user_code = raw[:4] + '-' + raw[4:]
                try:
                    db.create_device_authorization(
                        conn, device_code=device_code, user_code=user_code,
                        client_name=(client_name or '')[:64] or None,
                        ttl_seconds=self.settings.device_code_ttl_seconds)
                    break
                except db.sqlite3.IntegrityError:
                    continue
            else:
                raise LoginError(503, 'LOGIN_BUSY', 'Could not allocate a login code')
        finally:
            conn.close()
        base = self.settings.public_base_url
        return {
            'device_code': device_code,
            'user_code': user_code,
            'verification_uri': base + '/login',
            'verification_uri_complete': base + '/login?code=' + user_code,
            'expires_in': self.settings.device_code_ttl_seconds,
            'interval': POLL_INTERVAL,
        }

    def poll(self, device_code: str) -> dict:
        if not isinstance(device_code, str) or not DEVICE_CODE_RE.fullmatch(device_code):
            raise LoginError(400, 'INVALID_DEVICE_CODE', 'device_code is invalid')
        conn = self._connect()
        try:
            status, user_id = db.poll_device(conn, device_code, POLL_INTERVAL)
            if status == 'approved' and user_id:
                token = auth.issue_token(conn, user_id, self.settings.token_ttl_days)
                expires = conn.execute(
                    'SELECT expires_at FROM access_tokens WHERE token_hash = ?',
                    (auth.hash_token(token),)).fetchone()['expires_at']
                logger.info('device_login_completed user=%s', user_id)
                return {'access_token': token, 'token_type': 'Bearer', 'expires_at': expires}
        finally:
            conn.close()
        codes = {
            'pending': ('AUTHORIZATION_PENDING', 'Waiting for the user to confirm in the browser'),
            'slow_down': ('SLOW_DOWN', 'Polling too fast; increase the interval by 5 seconds'),
            'expired': ('EXPIRED_TOKEN', 'The login code expired; start again'),
            'invalid': ('INVALID_DEVICE_CODE', 'device_code is unknown or already used'),
        }
        code, message = codes.get(status, codes['invalid'])
        raise LoginError(400, code, message)

    def request_link(self, email: str, user_code: str) -> None:
        """Send a magic link. Silent for unknown emails (no account enumeration)."""
        self._require_configured()
        email = (email or '').strip()
        if not EMAIL_RE.fullmatch(email):
            raise LoginError(400, 'INVALID_EMAIL', 'Enter a valid email address')
        user_code = normalise_user_code(user_code)
        if not USER_CODE_RE.fullmatch(user_code):
            raise LoginError(400, 'INVALID_USER_CODE', 'The code from the app is invalid')
        conn = self._connect()
        try:
            if not db.find_pending_user_code(conn, user_code):
                raise LoginError(400, 'INVALID_USER_CODE',
                                 'The code is unknown or expired; start login again in the app')
            if db.recent_magic_links(conn, email, EMAIL_WINDOW_SECONDS) >= EMAILS_PER_WINDOW:
                # Same response as success: do not reveal throttling per address.
                logger.warning('magic_link_throttled')
                return
            token = secrets.token_urlsafe(32)
            db.create_magic_link(conn, token=token, email=email, user_code=user_code,
                                 ttl_seconds=self.settings.magic_link_ttl_seconds)
        finally:
            conn.close()
        link = self.settings.public_base_url + '/auth/verify?token=' + token
        try:
            mailer.send(self.settings, mailer.login_message(
                self.settings, to=db.normalise_email(email), link=link, user_code=user_code))
        except mailer.MailError as exc:
            logger.error('magic_link_delivery_failed reason=%s', exc)
            raise LoginError(503, 'MAIL_UNAVAILABLE', 'Could not send the email; try again') from exc

    def preview_link(self, token: str) -> dict | None:
        if not isinstance(token, str) or not MAGIC_TOKEN_RE.fullmatch(token):
            return None
        conn = self._connect()
        try:
            return db.peek_magic_link(conn, token)
        finally:
            conn.close()

    def confirm_link(self, token: str) -> dict:
        """Consume the link, create or reuse the account and approve the device."""
        if not isinstance(token, str) or not MAGIC_TOKEN_RE.fullmatch(token):
            raise LoginError(400, 'INVALID_LINK', 'The link is invalid')
        conn = self._connect()
        try:
            link = db.consume_magic_link(conn, token)
            if not link:
                raise LoginError(400, 'INVALID_LINK', 'The link is invalid, used or expired')
            user = db.get_or_create_user_by_email(conn, link['email'])
            if not db.approve_device(conn, link['user_code'], user['id']):
                raise LoginError(400, 'EXPIRED_TOKEN',
                                 'The app login code expired; start login again in the app')
        finally:
            conn.close()
        logger.info('device_login_approved user=%s new_account=%s', user['id'], user['created'])
        return {'user_code': link['user_code'], 'new_account': user['created']}
