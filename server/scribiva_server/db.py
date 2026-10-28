"""SQLite persistence, quota reservations and idempotent job state.

SQLite keeps the skeleton dependency-free while still giving real transactional
semantics: ``BEGIN IMMEDIATE`` serialises concurrent write transactions, and the
unique ``(user_id, client_job_id)`` constraint makes retries idempotent.
"""
from __future__ import annotations
import json
import os
import secrets
import sqlite3
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCHEMA = """
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  email TEXT UNIQUE,
  created_at TEXT NOT NULL,
  deletion_requested_at TEXT
);

CREATE TABLE IF NOT EXISTS access_tokens (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_hash TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  revoked_at TEXT,
  last_used_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_tokens_user ON access_tokens(user_id);

CREATE TABLE IF NOT EXISTS subscriptions (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  plan_code TEXT NOT NULL,
  stripe_customer_id TEXT,
  stripe_subscription_id TEXT,
  current_period_end TEXT,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS dictation_jobs (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  client_job_id TEXT NOT NULL,
  state TEXT NOT NULL,
  audio_duration_ms INTEGER,
  billable_seconds INTEGER NOT NULL DEFAULT 0,
  language_code TEXT,
  grammar_mode TEXT,
  client_version TEXT,
  raw_text TEXT,
  output_text TEXT,
  detected_language TEXT,
  warning TEXT,
  processing_json TEXT,
  error_code TEXT,
  error_message TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  result_expires_at TEXT,
  UNIQUE(user_id, client_job_id)
);
CREATE INDEX IF NOT EXISTS idx_jobs_user_state ON dictation_jobs(user_id, state);
CREATE INDEX IF NOT EXISTS idx_jobs_created ON dictation_jobs(created_at);

CREATE TABLE IF NOT EXISTS usage_ledger (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  period_start TEXT NOT NULL,
  billable_seconds INTEGER NOT NULL,
  event_type TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE(job_id, event_type)
);
CREATE INDEX IF NOT EXISTS idx_ledger_user_period ON usage_ledger(user_id, period_start);

CREATE TABLE IF NOT EXISTS processed_webhook_events (
  provider_event_id TEXT PRIMARY KEY,
  received_at TEXT NOT NULL
);
"""


class DbConflict(RuntimeError):
    def __init__(self, code: str, message: str, retryable: bool = False):
        super().__init__(message)
        self.code = code
        self.message = message
        self.retryable = retryable


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


def iso(value: datetime | None = None) -> str:
    return (value or utcnow()).astimezone(timezone.utc).isoformat(timespec='seconds')


def period_start(value: datetime | None = None) -> str:
    current = (value or utcnow()).astimezone(timezone.utc)
    return current.replace(day=1, hour=0, minute=0, second=0, microsecond=0).date().isoformat()


def connect(path: Path) -> sqlite3.Connection:
    path = Path(path)
    parent = path.parent
    existed = parent.exists()
    parent.mkdir(parents=True, exist_ok=True)
    if not existed:
        try:
            os.chmod(parent, 0o700)
        except OSError:
            pass
    conn = sqlite3.connect(str(path), timeout=10, isolation_level=None)
    conn.row_factory = sqlite3.Row
    conn.execute('PRAGMA foreign_keys = ON')
    conn.execute('PRAGMA busy_timeout = 10000')
    conn.execute('PRAGMA journal_mode = WAL')
    conn.execute('PRAGMA synchronous = NORMAL')
    # The main database and its WAL/SHM sidecars contain transcript text.
    for candidate in (path, Path(str(path) + '-wal'), Path(str(path) + '-shm')):
        if candidate.exists():
            try:
                os.chmod(candidate, 0o600)
            except OSError:
                pass
    return conn


def init_db(path: Path) -> None:
    conn = connect(path)
    try:
        conn.executescript(SCHEMA)
        conn.execute('PRAGMA user_version = 1')
    finally:
        conn.close()
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass


@contextmanager
def transaction(conn: sqlite3.Connection, immediate: bool = False):
    conn.execute('BEGIN IMMEDIATE' if immediate else 'BEGIN')
    try:
        yield conn
    except Exception:
        conn.execute('ROLLBACK')
        raise
    else:
        conn.execute('COMMIT')


def create_user(conn: sqlite3.Connection, email: str | None) -> dict:
    user_id = 'usr_' + secrets.token_hex(12)
    email = email.strip().lower() if email else None
    with transaction(conn, immediate=True):
        conn.execute('INSERT INTO users (id, email, created_at) VALUES (?, ?, ?)',
                     (user_id, email, iso()))
    return {'id': user_id, 'email': email}


def find_user(conn: sqlite3.Connection, identifier: str) -> sqlite3.Row | None:
    identifier = identifier.strip()
    row = conn.execute('SELECT * FROM users WHERE id = ?', (identifier,)).fetchone()
    if row:
        return row
    return conn.execute('SELECT * FROM users WHERE lower(email) = lower(?)',
                        (identifier,)).fetchone()


def get_subscription(conn: sqlite3.Connection, user_id: str) -> dict:
    row = conn.execute('SELECT * FROM subscriptions WHERE user_id = ?', (user_id,)).fetchone()
    if not row:
        return {'status': 'inactive', 'plan_code': 'free', 'stripe_customer_id': None,
                'stripe_subscription_id': None, 'current_period_end': None}
    return dict(row)


def effective_plan(subscription: dict) -> str:
    return 'pro' if subscription.get('status') in ('active', 'trialing') else 'free'


def set_subscription(conn: sqlite3.Connection, user_id: str, *, status: str,
                     plan_code: str = 'pro', customer_id: str | None = None,
                     subscription_id: str | None = None,
                     current_period_end: str | None = None) -> None:
    with transaction(conn, immediate=True):
        conn.execute(
            """INSERT INTO subscriptions
               (user_id, status, plan_code, stripe_customer_id, stripe_subscription_id,
                current_period_end, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(user_id) DO UPDATE SET
                 status=excluded.status,
                 plan_code=excluded.plan_code,
                 stripe_customer_id=COALESCE(excluded.stripe_customer_id, subscriptions.stripe_customer_id),
                 stripe_subscription_id=COALESCE(excluded.stripe_subscription_id, subscriptions.stripe_subscription_id),
                 current_period_end=COALESCE(excluded.current_period_end, subscriptions.current_period_end),
                 updated_at=excluded.updated_at""",
            (user_id, status, plan_code, customer_id, subscription_id,
             current_period_end, iso()),
        )


def usage(conn: sqlite3.Connection, user_id: str) -> dict:
    period = period_start()
    used = conn.execute(
        """SELECT COALESCE(SUM(billable_seconds), 0) AS total FROM usage_ledger
           WHERE user_id = ? AND period_start = ? AND event_type = 'debit'""",
        (user_id, period),
    ).fetchone()['total']
    reserved = conn.execute(
        """SELECT COALESCE(SUM(billable_seconds), 0) AS total FROM dictation_jobs
           WHERE user_id = ? AND state = 'processing'""",
        (user_id,),
    ).fetchone()['total']
    return {'period_start': period, 'used_seconds': int(used),
            'reserved_seconds': int(reserved)}


def _expire_stale_jobs(conn: sqlite3.Connection, timeout_seconds: int) -> int:
    cutoff = iso(utcnow() - timedelta(seconds=timeout_seconds))
    cursor = conn.execute(
        """UPDATE dictation_jobs
           SET state='failed', error_code='JOB_TIMEOUT',
               error_message='Job did not finish before the server timeout',
               updated_at=?
           WHERE state='processing' AND created_at < ?""",
        (iso(), cutoff),
    )
    return cursor.rowcount


def create_or_get_job(conn: sqlite3.Connection, *, user_id: str, client_job_id: str,
                      billable_seconds: int, language: str, grammar_mode: str,
                      client_version: str, timeout_seconds: int,
                      limit_seconds: int) -> tuple[dict, bool]:
    """Reserve quota and create a job, or return the existing idempotent job.

    Raises DbConflict for concurrency or quota violations.
    """
    with transaction(conn, immediate=True):
        _expire_stale_jobs(conn, timeout_seconds)
        existing = conn.execute(
            'SELECT * FROM dictation_jobs WHERE user_id = ? AND client_job_id = ?',
            (user_id, client_job_id),
        ).fetchone()
        if existing:
            return dict(existing), False
        processing = conn.execute(
            "SELECT COUNT(*) AS n FROM dictation_jobs WHERE user_id = ? AND state = 'processing'",
            (user_id,),
        ).fetchone()['n']
        if processing:
            raise DbConflict('BUSY', 'Another dictation is still processing', retryable=True)
        used = usage(conn, user_id)['used_seconds']
        if used + billable_seconds > limit_seconds:
            raise DbConflict('QUOTA_EXCEEDED',
                             'Current plan usage limit reached', retryable=False)
        job_id = 'job_' + secrets.token_hex(12)
        now = iso()
        conn.execute(
            """INSERT INTO dictation_jobs
               (id, user_id, client_job_id, state, billable_seconds, language_code,
                grammar_mode, client_version, created_at, updated_at)
               VALUES (?, ?, ?, 'processing', ?, ?, ?, ?, ?, ?)""",
            (job_id, user_id, client_job_id, billable_seconds, language,
             grammar_mode, client_version, now, now),
        )
        row = conn.execute('SELECT * FROM dictation_jobs WHERE id = ?', (job_id,)).fetchone()
        return dict(row), True


def get_job(conn: sqlite3.Connection, user_id: str, job_id: str) -> dict | None:
    row = conn.execute('SELECT * FROM dictation_jobs WHERE id = ? AND user_id = ?',
                       (job_id, user_id)).fetchone()
    return dict(row) if row else None


def job_is_expired(job: dict, now: datetime | None = None) -> bool:
    expires = job.get('result_expires_at')
    if not expires:
        return False
    try:
        return datetime.fromisoformat(expires) <= (now or utcnow())
    except ValueError:
        return True


def complete_job(conn: sqlite3.Connection, *, job_id: str, user_id: str,
                 duration_ms: int, raw_text: str, output_text: str,
                 detected_language: str | None, warning: str | None,
                 processing: dict, result_ttl_seconds: int) -> bool:
    """Commit usage and store the result only while the job is still processing."""
    with transaction(conn, immediate=True):
        job = conn.execute('SELECT * FROM dictation_jobs WHERE id = ? AND user_id = ?',
                           (job_id, user_id)).fetchone()
        if not job or job['state'] != 'processing':
            return False
        now = utcnow()
        expires = (now + timedelta(seconds=result_ttl_seconds)).isoformat(timespec='seconds')
        conn.execute(
            """UPDATE dictation_jobs
               SET state='completed', audio_duration_ms=?, raw_text=?, output_text=?,
                   detected_language=?, warning=?, processing_json=?,
                   result_expires_at=?, updated_at=?
               WHERE id = ? AND user_id = ?""",
            (duration_ms, raw_text, output_text, detected_language, warning,
             json.dumps(processing, ensure_ascii=False), expires, iso(now),
             job_id, user_id),
        )
        conn.execute(
            """INSERT OR IGNORE INTO usage_ledger
               (id, user_id, job_id, period_start, billable_seconds, event_type, created_at)
               VALUES (?, ?, ?, ?, ?, 'debit', ?)""",
            ('led_' + secrets.token_hex(12), user_id, job_id,
             period_start(now), job['billable_seconds'], iso(now)),
        )
        return True


def fail_job(conn: sqlite3.Connection, job_id: str, user_id: str,
             code: str, message: str) -> bool:
    with transaction(conn, immediate=True):
        cursor = conn.execute(
            """UPDATE dictation_jobs
               SET state='failed', error_code=?, error_message=?, updated_at=?
               WHERE id = ? AND user_id = ? AND state='processing'""",
            (code, message[:500], iso(), job_id, user_id),
        )
        return cursor.rowcount == 1


def cancel_job(conn: sqlite3.Connection, user_id: str, job_id: str) -> dict | None:
    with transaction(conn, immediate=True):
        job = conn.execute('SELECT * FROM dictation_jobs WHERE id = ? AND user_id = ?',
                           (job_id, user_id)).fetchone()
        if not job:
            return None
        if job['state'] == 'processing':
            conn.execute(
                """UPDATE dictation_jobs
                   SET state='cancelled', error_code='JOB_CANCELLED',
                       error_message='Cancelled by user', updated_at=?
                   WHERE id = ? AND user_id = ?""",
                (iso(), job_id, user_id),
            )
            job = conn.execute('SELECT * FROM dictation_jobs WHERE id = ?',
                               (job_id,)).fetchone()
        elif job['state'] == 'cancelled':
            pass
        return dict(job)


def job_still_processing(conn: sqlite3.Connection, user_id: str, job_id: str) -> bool:
    row = conn.execute('SELECT state FROM dictation_jobs WHERE id = ? AND user_id = ?',
                       (job_id, user_id)).fetchone()
    return bool(row and row['state'] == 'processing')


def clear_expired_result(conn: sqlite3.Connection, job: dict) -> None:
    if not job_is_expired(job):
        return
    with transaction(conn, immediate=True):
        conn.execute(
            """UPDATE dictation_jobs
               SET raw_text=NULL, output_text=NULL, warning=NULL, processing_json=NULL
               WHERE id = ? AND user_id = ?""",
            (job['id'], job['user_id']),
        )


def webhook_seen(conn: sqlite3.Connection, event_id: str) -> bool:
    return bool(conn.execute(
        'SELECT 1 FROM processed_webhook_events WHERE provider_event_id = ?',
        (event_id,)).fetchone())


def record_webhook_event(conn: sqlite3.Connection, event_id: str) -> bool:
    with transaction(conn, immediate=True):
        cursor = conn.execute(
            'INSERT OR IGNORE INTO processed_webhook_events (provider_event_id, received_at) '
            'VALUES (?, ?)', (event_id, iso()))
        return cursor.rowcount == 1


def delete_user_data(conn: sqlite3.Connection, user_id: str) -> dict:
    """Remove transcripts, jobs and tokens; keep minimum billing metadata."""
    with transaction(conn, immediate=True):
        jobs = conn.execute('DELETE FROM dictation_jobs WHERE user_id = ?', (user_id,)).rowcount
        tokens = conn.execute('DELETE FROM access_tokens WHERE user_id = ?', (user_id,)).rowcount
        conn.execute('UPDATE users SET email = NULL, deletion_requested_at = ? WHERE id = ?',
                     (iso(), user_id))
    return {'deleted_jobs': jobs, 'revoked_tokens': tokens,
            'retained': 'usage ledger and subscription status (billing metadata only)'}


def recent_jobs(conn: sqlite3.Connection, user_id: str, limit: int = 20) -> list[dict]:
    rows = conn.execute(
        """SELECT id, state, billable_seconds, language_code, grammar_mode,
                  created_at, result_expires_at, error_code
           FROM dictation_jobs WHERE user_id = ? ORDER BY created_at DESC LIMIT ?""",
        (user_id, max(1, min(limit, 100))),
    ).fetchall()
    return [dict(row) for row in rows]
