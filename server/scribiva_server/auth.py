"""Bearer access tokens: generated once, stored only as SHA-256 hashes."""
from __future__ import annotations
import hashlib
import hmac
import re
import secrets
import sqlite3
from datetime import timedelta

from . import db

TOKEN_PREFIX = 'scr_'
TOKEN_RE = re.compile(r'^scr_[A-Za-z0-9_-]{20,120}$')


class AuthError(RuntimeError):
    pass


def hash_token(token: str) -> str:
    return hashlib.sha256(token.encode('utf-8')).hexdigest()


def issue_token(conn: sqlite3.Connection, user_id: str, ttl_days: int) -> str:
    token = TOKEN_PREFIX + secrets.token_urlsafe(32)
    token_id = 'tok_' + secrets.token_hex(10)
    now = db.utcnow()
    expires = now + timedelta(days=max(1, ttl_days))
    with db.transaction(conn, immediate=True):
        conn.execute(
            'INSERT INTO access_tokens (id, user_id, token_hash, created_at, expires_at) '
            'VALUES (?, ?, ?, ?, ?)',
            (token_id, user_id, hash_token(token), db.iso(now), db.iso(expires)),
        )
    return token


def revoke_token(conn: sqlite3.Connection, token: str) -> bool:
    with db.transaction(conn, immediate=True):
        cursor = conn.execute(
            'UPDATE access_tokens SET revoked_at = ? WHERE token_hash = ? AND revoked_at IS NULL',
            (db.iso(), hash_token(token)),
        )
        return cursor.rowcount == 1


def parse_bearer(header: str | None) -> str | None:
    if not header:
        return None
    scheme, _, value = header.strip().partition(' ')
    if scheme.lower() != 'bearer':
        return None
    token = value.strip()
    if not TOKEN_RE.fullmatch(token):
        return None
    return token


def authenticate(conn: sqlite3.Connection, header: str | None) -> dict | None:
    token = parse_bearer(header)
    if not token:
        return None
    row = conn.execute(
        """SELECT u.id, u.email, u.deletion_requested_at, t.id AS token_id,
                  t.expires_at, t.revoked_at
           FROM access_tokens t JOIN users u ON u.id = t.user_id
           WHERE t.token_hash = ?""",
        (hash_token(token),),
    ).fetchone()
    if not row:
        return None
    if row['revoked_at']:
        return None
    try:
        from datetime import datetime
        if datetime.fromisoformat(row['expires_at']) <= db.utcnow():
            return None
    except ValueError:
        return None
    if row['deletion_requested_at']:
        return None
    return {'id': row['id'], 'email': row['email'], 'token_id': row['token_id']}


def token_matches(header: str | None, expected: str) -> bool:
    """Constant-time comparison helper for non-database tokens."""
    supplied = parse_bearer(header)
    return bool(supplied and hmac.compare_digest(supplied, expected))
