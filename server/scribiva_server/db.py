"""SQLite persistence, quota reservations and idempotent job state.

SQLite keeps the skeleton dependency-free while still giving real transactional
semantics: ``BEGIN IMMEDIATE`` serialises concurrent write transactions, and the
unique ``(user_id, client_job_id)`` constraint makes retries idempotent.
"""
from __future__ import annotations
import hashlib
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

CREATE TABLE IF NOT EXISTS user_dictionary (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  terms_json TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- Device authorization (FR-019). device_code and magic-link tokens are stored
-- only as SHA-256 hashes; user_code is short-lived and shown to the user.
CREATE TABLE IF NOT EXISTS device_authorizations (
  device_code_hash TEXT PRIMARY KEY,
  user_code TEXT NOT NULL UNIQUE,
  client_name TEXT,
  user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  approved_at TEXT,
  consumed_at TEXT,
  last_poll_at TEXT
);

CREATE TABLE IF NOT EXISTS magic_links (
  token_hash TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  user_code TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  used_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_magic_links_email ON magic_links(email, created_at);
"""
SCHEMA_VERSION = 3


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


def period_reset(value: datetime | None = None) -> str:
    """UTC instant at which the current monthly free quota resets."""
    current = (value or utcnow()).astimezone(timezone.utc)
    year, month = (current.year + 1, 1) if current.month == 12 else (current.year, current.month + 1)
    return datetime(year, month, 1, tzinfo=timezone.utc).isoformat(timespec='seconds')


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


def _migrate(conn: sqlite3.Connection) -> None:
    """Forward-only, idempotent schema migrations."""
    version = conn.execute('PRAGMA user_version').fetchone()[0]
    if version < 2:
        # v2: separate the charged seconds from the reserved duration so the
        # month-end grace (charge up to the limit) does not lose information.
        columns = {row['name'] for row in conn.execute('PRAGMA table_info(dictation_jobs)')}
        if 'charge_seconds' not in columns:
            conn.execute('ALTER TABLE dictation_jobs ADD COLUMN charge_seconds INTEGER')
        if 'suspected_hallucination' not in columns:
            conn.execute('ALTER TABLE dictation_jobs ADD COLUMN '
                         'suspected_hallucination INTEGER NOT NULL DEFAULT 0')
        sub_columns = {row['name'] for row in conn.execute('PRAGMA table_info(subscriptions)')}
        if 'last_event_created' not in sub_columns:
            conn.execute('ALTER TABLE subscriptions ADD COLUMN last_event_created INTEGER')
        user_columns = {row['name'] for row in conn.execute('PRAGMA table_info(users)')}
        if 'email_hash' not in user_columns:
            conn.execute('ALTER TABLE users ADD COLUMN email_hash TEXT')
        for row in conn.execute('SELECT id, email FROM users WHERE email IS NOT NULL '
                                'AND email_hash IS NULL').fetchall():
            conn.execute('UPDATE users SET email_hash = ? WHERE id = ?',
                         (email_hash(row['email']), row['id']))
        conn.execute('CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email_hash '
                     'ON users(email_hash)')
    if version < 3:
        # v3: how the result reached the user (north-star metric, docs/11).
        columns = {row['name'] for row in conn.execute('PRAGMA table_info(dictation_jobs)')}
        if 'insert_outcome' not in columns:
            conn.execute('ALTER TABLE dictation_jobs ADD COLUMN insert_outcome TEXT')
        if 'outcome_at' not in columns:
            conn.execute('ALTER TABLE dictation_jobs ADD COLUMN outcome_at TEXT')
    conn.execute(f'PRAGMA user_version = {SCHEMA_VERSION}')


def init_db(path: Path) -> None:
    conn = connect(path)
    try:
        conn.executescript(SCHEMA)
        _migrate(conn)
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


def normalise_email(email: str) -> str:
    return email.strip().lower()


def email_hash(email: str) -> str:
    return hashlib.sha256(normalise_email(email).encode('utf-8')).hexdigest()


def _user_email_hash(conn: sqlite3.Connection, user_id: str) -> str | None:
    row = conn.execute('SELECT email, email_hash FROM users WHERE id = ?', (user_id,)).fetchone()
    if not row:
        return None
    return row['email_hash'] or (email_hash(row['email']) if row['email'] else None)


def create_user(conn: sqlite3.Connection, email: str | None) -> dict:
    user_id = 'usr_' + secrets.token_hex(12)
    email = normalise_email(email) if email else None
    with transaction(conn, immediate=True):
        conn.execute('INSERT INTO users (id, email, email_hash, created_at) VALUES (?, ?, ?, ?)',
                     (user_id, email, email_hash(email) if email else None, iso()))
    return {'id': user_id, 'email': email}


def get_or_create_user_by_email(conn: sqlite3.Connection, email: str) -> dict:
    """One account per email. A deleted account is reactivated, keeping its
    usage ledger, so deletion cannot be used to reset the monthly free tier."""
    email = normalise_email(email)
    digest = email_hash(email)
    with transaction(conn, immediate=True):
        row = conn.execute('SELECT id FROM users WHERE email_hash = ?', (digest,)).fetchone()
        if row:
            conn.execute('UPDATE users SET email = ?, deletion_requested_at = NULL WHERE id = ?',
                         (email, row['id']))
            return {'id': row['id'], 'email': email, 'created': False}
        user_id = 'usr_' + secrets.token_hex(12)
        conn.execute('INSERT INTO users (id, email, email_hash, created_at) VALUES (?, ?, ?, ?)',
                     (user_id, email, digest, iso()))
        return {'id': user_id, 'email': email, 'created': True}


# ------------------------------------------------------------- dictionary

def get_dictionary(conn: sqlite3.Connection, user_id: str) -> list[str]:
    row = conn.execute('SELECT terms_json FROM user_dictionary WHERE user_id = ?',
                       (user_id,)).fetchone()
    if not row:
        return []
    try:
        terms = json.loads(row['terms_json'])
    except ValueError:
        return []
    return [t for t in terms if isinstance(t, str)] if isinstance(terms, list) else []


def set_dictionary(conn: sqlite3.Connection, user_id: str, terms: list[str]) -> None:
    with transaction(conn, immediate=True):
        conn.execute(
            """INSERT INTO user_dictionary (user_id, terms_json, updated_at) VALUES (?, ?, ?)
               ON CONFLICT(user_id) DO UPDATE SET terms_json=excluded.terms_json,
                                                  updated_at=excluded.updated_at""",
            (user_id, json.dumps(terms, ensure_ascii=False), iso()))


# ----------------------------------------------------- device authorization

def secret_hash(value: str) -> str:
    return hashlib.sha256(value.encode('utf-8')).hexdigest()


def create_device_authorization(conn: sqlite3.Connection, *, device_code: str,
                                user_code: str, client_name: str | None,
                                ttl_seconds: int) -> None:
    now = utcnow()
    with transaction(conn, immediate=True):
        conn.execute('DELETE FROM device_authorizations WHERE expires_at < ?',
                     (iso(now - timedelta(days=1)),))
        conn.execute(
            """INSERT INTO device_authorizations
               (device_code_hash, user_code, client_name, created_at, expires_at)
               VALUES (?, ?, ?, ?, ?)""",
            (secret_hash(device_code), user_code, client_name, iso(now),
             iso(now + timedelta(seconds=ttl_seconds))))


def find_pending_user_code(conn: sqlite3.Connection, user_code: str) -> dict | None:
    row = conn.execute(
        """SELECT * FROM device_authorizations WHERE user_code = ? AND approved_at IS NULL
           AND consumed_at IS NULL AND expires_at > ?""", (user_code, iso())).fetchone()
    return dict(row) if row else None


def approve_device(conn: sqlite3.Connection, user_code: str, user_id: str) -> bool:
    with transaction(conn, immediate=True):
        cursor = conn.execute(
            """UPDATE device_authorizations SET user_id = ?, approved_at = ?
               WHERE user_code = ? AND approved_at IS NULL AND consumed_at IS NULL
                 AND expires_at > ?""", (user_id, iso(), user_code, iso()))
        return cursor.rowcount == 1


def poll_device(conn: sqlite3.Connection, device_code: str, min_interval: int) -> tuple[str, str | None]:
    """Return (status, user_id); status in pending|slow_down|expired|invalid|approved."""
    digest = secret_hash(device_code)
    now = utcnow()
    with transaction(conn, immediate=True):
        row = conn.execute('SELECT * FROM device_authorizations WHERE device_code_hash = ?',
                           (digest,)).fetchone()
        if not row or row['consumed_at']:
            return 'invalid', None
        if datetime.fromisoformat(row['expires_at']) <= now:
            return 'expired', None
        last = row['last_poll_at']
        conn.execute('UPDATE device_authorizations SET last_poll_at = ? WHERE device_code_hash = ?',
                     (iso(now), digest))
        if not row['approved_at']:
            if last and (now - datetime.fromisoformat(last)).total_seconds() < min_interval - 0.5:
                return 'slow_down', None
            return 'pending', None
        conn.execute('UPDATE device_authorizations SET consumed_at = ? WHERE device_code_hash = ?',
                     (iso(now), digest))
        return 'approved', row['user_id']


def create_magic_link(conn: sqlite3.Connection, *, token: str, email: str, user_code: str,
                      ttl_seconds: int) -> None:
    now = utcnow()
    with transaction(conn, immediate=True):
        conn.execute('DELETE FROM magic_links WHERE expires_at < ?',
                     (iso(now - timedelta(days=1)),))
        conn.execute(
            """INSERT INTO magic_links (token_hash, email, user_code, created_at, expires_at)
               VALUES (?, ?, ?, ?, ?)""",
            (secret_hash(token), normalise_email(email), user_code, iso(now),
             iso(now + timedelta(seconds=ttl_seconds))))


def recent_magic_links(conn: sqlite3.Connection, email: str, window_seconds: int) -> int:
    cutoff = iso(utcnow() - timedelta(seconds=window_seconds))
    return conn.execute('SELECT COUNT(*) AS n FROM magic_links WHERE email = ? AND created_at > ?',
                        (normalise_email(email), cutoff)).fetchone()['n']


def peek_magic_link(conn: sqlite3.Connection, token: str) -> dict | None:
    row = conn.execute(
        'SELECT * FROM magic_links WHERE token_hash = ? AND used_at IS NULL AND expires_at > ?',
        (secret_hash(token), iso())).fetchone()
    return dict(row) if row else None


def consume_magic_link(conn: sqlite3.Connection, token: str) -> dict | None:
    with transaction(conn, immediate=True):
        row = conn.execute(
            'SELECT * FROM magic_links WHERE token_hash = ? AND used_at IS NULL AND expires_at > ?',
            (secret_hash(token), iso())).fetchone()
        if not row:
            return None
        conn.execute('UPDATE magic_links SET used_at = ? WHERE token_hash = ?',
                     (iso(), secret_hash(token)))
        return dict(row)


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


def link_stripe_customer(conn: sqlite3.Connection, user_id: str, *,
                         customer_id: str | None, subscription_id: str | None) -> None:
    """Record Stripe ids without touching entitlement status."""
    with transaction(conn, immediate=True):
        conn.execute(
            """INSERT INTO subscriptions (user_id, status, plan_code, stripe_customer_id,
                                          stripe_subscription_id, updated_at)
               VALUES (?, 'inactive', 'pro', ?, ?, ?)
               ON CONFLICT(user_id) DO UPDATE SET
                 stripe_customer_id=COALESCE(excluded.stripe_customer_id, subscriptions.stripe_customer_id),
                 stripe_subscription_id=COALESCE(excluded.stripe_subscription_id, subscriptions.stripe_subscription_id),
                 updated_at=excluded.updated_at""",
            (user_id, customer_id, subscription_id, iso()))


def set_subscription(conn: sqlite3.Connection, user_id: str, *, status: str,
                     plan_code: str = 'pro', customer_id: str | None = None,
                     subscription_id: str | None = None,
                     current_period_end: str | None = None,
                     event_created: int | None = None) -> bool:
    """Upsert subscription state. With ``event_created`` (Stripe event time),
    an event older than the last applied one is ignored: Stripe does not
    guarantee delivery order. Returns False when ignored."""
    with transaction(conn, immediate=True):
        if event_created is not None:
            row = conn.execute('SELECT last_event_created FROM subscriptions WHERE user_id = ?',
                               (user_id,)).fetchone()
            if row and row['last_event_created'] is not None \
                    and row['last_event_created'] > event_created:
                return False
        conn.execute(
            """INSERT INTO subscriptions
               (user_id, status, plan_code, stripe_customer_id, stripe_subscription_id,
                current_period_end, last_event_created, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(user_id) DO UPDATE SET
                 status=excluded.status,
                 plan_code=excluded.plan_code,
                 stripe_customer_id=COALESCE(excluded.stripe_customer_id, subscriptions.stripe_customer_id),
                 stripe_subscription_id=COALESCE(excluded.stripe_subscription_id, subscriptions.stripe_subscription_id),
                 current_period_end=COALESCE(excluded.current_period_end, subscriptions.current_period_end),
                 last_event_created=COALESCE(excluded.last_event_created, subscriptions.last_event_created),
                 updated_at=excluded.updated_at""",
            (user_id, status, plan_code, customer_id, subscription_id,
             current_period_end, event_created, iso()),
        )
        return True


def usage(conn: sqlite3.Connection, user_id: str) -> dict:
    period = period_start()
    used = conn.execute(
        """SELECT COALESCE(SUM(billable_seconds), 0) AS total FROM usage_ledger
           WHERE user_id = ? AND period_start = ? AND event_type = 'debit'""",
        (user_id, period),
    ).fetchone()['total']
    reserved = conn.execute(
        """SELECT COALESCE(SUM(COALESCE(charge_seconds, billable_seconds)), 0) AS total
           FROM dictation_jobs WHERE user_id = ? AND state = 'processing'""",
        (user_id,),
    ).fetchone()['total']
    return {'period_start': period, 'used_seconds': int(used),
            'reserved_seconds': int(reserved), 'resets_at': period_reset()}


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
                      limit_seconds: int, grace_seconds: int = 0) -> tuple[dict, bool]:
    """Reserve quota and create a job, or return the existing idempotent job.

    Month-end grace: while some quota remains, one recording that overshoots the
    limit by at most ``grace_seconds`` is accepted and charged only up to the
    limit. Raises DbConflict for concurrency or quota violations.
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
        remaining = limit_seconds - used
        if billable_seconds <= remaining:
            charge = billable_seconds
        elif remaining > 0 and billable_seconds <= remaining + grace_seconds:
            charge = remaining
        else:
            raise DbConflict('QUOTA_EXCEEDED',
                             'Current plan usage limit reached', retryable=False)
        job_id = 'job_' + secrets.token_hex(12)
        now = iso()
        conn.execute(
            """INSERT INTO dictation_jobs
               (id, user_id, client_job_id, state, billable_seconds, charge_seconds,
                language_code, grammar_mode, client_version, created_at, updated_at)
               VALUES (?, ?, ?, 'processing', ?, ?, ?, ?, ?, ?, ?)""",
            (job_id, user_id, client_job_id, billable_seconds, charge, language,
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
                 processing: dict, result_ttl_seconds: int,
                 suspected_hallucination: bool = False) -> bool:
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
                   result_expires_at=?, updated_at=?, suspected_hallucination=?
               WHERE id = ? AND user_id = ?""",
            (duration_ms, raw_text, output_text, detected_language, warning,
             json.dumps(processing, ensure_ascii=False), expires, iso(now),
             1 if suspected_hallucination else 0, job_id, user_id),
        )
        conn.execute(
            """INSERT OR IGNORE INTO usage_ledger
               (id, user_id, job_id, period_start, billable_seconds, event_type, created_at)
               VALUES (?, ?, ?, ?, ?, 'debit', ?)""",
            ('led_' + secrets.token_hex(12), user_id, job_id, period_start(now),
             job['charge_seconds'] if job['charge_seconds'] is not None
             else job['billable_seconds'], iso(now)),
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
        conn.execute('DELETE FROM user_dictionary WHERE user_id = ?', (user_id,))
        conn.execute('DELETE FROM device_authorizations WHERE user_id = ?', (user_id,))
        # The email is replaced by a one-way hash so the same address cannot
        # re-register to obtain a second monthly free quota (FR-019).
        conn.execute(
            """UPDATE users SET email = NULL, email_hash = COALESCE(email_hash, ?),
                      deletion_requested_at = ? WHERE id = ?""",
            (_user_email_hash(conn, user_id), iso(), user_id))
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


INSERT_OUTCOMES = ('inserted', 'dispatched_unverified', 'clipboard_only', 'focus_changed',
                   'suspected_noise', 'own_window')


def record_outcome(conn: sqlite3.Connection, user_id: str, job_id: str, outcome: str) -> bool:
    """Store how a completed job's text reached the user. First report wins."""
    with transaction(conn, immediate=True):
        cursor = conn.execute(
            """UPDATE dictation_jobs SET insert_outcome = ?, outcome_at = ?
               WHERE id = ? AND user_id = ? AND state = 'completed' AND insert_outcome IS NULL""",
            (outcome, iso(), job_id, user_id))
        return cursor.rowcount == 1


def weekly_metrics(conn: sqlite3.Connection, weeks: int = 8) -> list[dict]:
    """North star (PRD §2): accounts with >= 3 completed dictations per ISO week,
    plus insertion outcome counts. Metadata only; no text."""
    since = iso(utcnow() - timedelta(weeks=weeks))
    rows = conn.execute(
        """SELECT strftime('%Y-W%W', created_at) AS week, user_id, COUNT(*) AS n,
                  SUM(insert_outcome = 'inserted') AS inserted,
                  SUM(insert_outcome = 'dispatched_unverified') AS pasted,
                  SUM(insert_outcome IN ('clipboard_only', 'focus_changed', 'own_window')) AS copied,
                  SUM(insert_outcome = 'suspected_noise') AS noise
           FROM dictation_jobs WHERE state = 'completed' AND created_at >= ?
           GROUP BY week, user_id""", (since,)).fetchall()
    weeks_out: dict[str, dict] = {}
    for r in rows:
        w = weeks_out.setdefault(r['week'], {'week': r['week'], 'active_accounts': 0,
                                             'north_star_accounts': 0, 'dictations': 0,
                                             'inserted': 0, 'pasted': 0, 'copied': 0, 'noise': 0})
        w['active_accounts'] += 1
        w['north_star_accounts'] += 1 if r['n'] >= 3 else 0
        w['dictations'] += r['n']
        for key in ('inserted', 'pasted', 'copied', 'noise'):
            w[key] += r[key] or 0
    return [weeks_out[k] for k in sorted(weeks_out)]
