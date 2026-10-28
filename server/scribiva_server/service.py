"""API service: auth, quota reservation, provider proxy and billing skeleton."""
from __future__ import annotations
import json
import logging
import math
import re
import threading
import time
from collections import defaultdict, deque

from . import __version__, audio, auth, db, stripe as stripe_mod
from .config import Settings
from .provider import GroqProvider, ProviderError

logger = logging.getLogger('scribiva_server')

LANGUAGES = {
    'auto', 'en', 'zh', 'ja', 'ko', 'es', 'fr', 'de', 'pt', 'it', 'ru',
    'ar', 'hi', 'tr', 'nl', 'pl', 'id', 'vi', 'th', 'sv', 'uk',
}
MODES = {'raw', 'standard', 'smart', 'polished', 'polish'}
CLIENT_JOB_RE = re.compile(r'^[A-Za-z0-9._:-]{8,128}$')
PRICE_PLANS = {'monthly', 'annual'}


class ApiError(RuntimeError):
    def __init__(self, status: int, code: str, message: str, *,
                 retryable: bool = False, retry_after: int | None = None,
                 extra: dict | None = None):
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message
        self.retryable = retryable
        self.retry_after = retry_after
        self.extra = extra or {}


class RateLimiter:
    def __init__(self, limit: int, window_seconds: int = 60):
        self.limit = limit
        self.window = window_seconds
        self._events: dict[str, deque] = defaultdict(deque)
        self._lock = threading.Lock()

    def check(self, key: str) -> int:
        """Return 0 when allowed, else seconds until the next slot."""
        now = time.monotonic()
        with self._lock:
            if len(self._events) > 4096:
                cutoff = now - self.window
                for old_key in list(self._events):
                    old_events = self._events[old_key]
                    while old_events and old_events[0] <= cutoff:
                        old_events.popleft()
                    if not old_events:
                        del self._events[old_key]
            events = self._events[key]
            while events and events[0] <= now - self.window:
                events.popleft()
            if len(events) >= self.limit:
                retry = max(1, math.ceil(events[0] + self.window - now))
                return retry
            events.append(now)
            return 0


class DictationService:
    def __init__(self, settings: Settings, provider=None):
        self.settings = settings
        self.provider = provider or GroqProvider(settings)
        self.stripe = stripe_mod.StripeClient(settings)
        self.rate_limiter = RateLimiter(settings.rate_limit_per_minute)
        db.init_db(settings.db_path)

    def _connect(self):
        return db.connect(self.settings.db_path)

    def health(self) -> dict:
        conn = self._connect()
        try:
            conn.execute('SELECT 1').fetchone()
        finally:
            conn.close()
        return {'status': 'ok', 'service': 'scribiva', 'version': __version__,
                'provider_configured': getattr(self.provider, 'available', True),
                'billing_configured': self.settings.billing_configured}

    # ---------------------------------------------------------------- account

    def authenticate(self, authorization: str | None) -> dict:
        conn = self._connect()
        try:
            user = auth.authenticate(conn, authorization)
        finally:
            conn.close()
        if not user:
            raise ApiError(401, 'AUTH_REQUIRED', 'A valid Bearer token is required')
        return user

    def me(self, user: dict) -> dict:
        conn = self._connect()
        try:
            subscription = db.get_subscription(conn, user['id'])
            plan = db.effective_plan(subscription)
            limit = self.settings.limit_for(plan)
            usage = db.usage(conn, user['id'])
        finally:
            conn.close()
        return {
            'user': {'id': user['id'], 'email': user.get('email')},
            'plan': plan,
            'subscription': {
                'status': subscription.get('status'),
                'current_period_end': subscription.get('current_period_end'),
            },
            'usage': self._usage_payload(usage, limit, plan),
            'limits': self._limits(),
            'billing_configured': self.settings.billing_configured,
        }

    def usage(self, user: dict) -> dict:
        conn = self._connect()
        try:
            subscription = db.get_subscription(conn, user['id'])
            plan = db.effective_plan(subscription)
            limit = self.settings.limit_for(plan)
            usage = db.usage(conn, user['id'])
        finally:
            conn.close()
        return self._usage_payload(usage, limit, plan)

    def _limits(self) -> dict:
        return {
            'monthly_free_seconds': self.settings.free_seconds,
            'monthly_pro_seconds': self.settings.pro_seconds,
            'max_audio_seconds': self.settings.max_audio_seconds,
            'max_audio_bytes': self.settings.max_audio_bytes,
            'result_ttl_seconds': self.settings.result_ttl_seconds,
        }

    def _usage_payload(self, usage: dict, limit: int, plan: str) -> dict:
        remaining = max(0, limit - usage['used_seconds'] - usage['reserved_seconds'])
        return {
            'period_start': usage['period_start'],
            'used_seconds': usage['used_seconds'],
            'reserved_seconds': usage['reserved_seconds'],
            'limit_seconds': limit,
            'remaining_seconds': remaining,
            'plan': plan,
        }

    def public_config(self) -> dict:
        return {
            'service': 'scribiva',
            'api_version': 'v1',
            'server_version': __version__,
            'client_min_version': '0.2.0-alpha.1',
            'features': {
                'dictation': True,
                'grammar': True,
                'billing': self.settings.billing_configured,
                'stripe_webhooks': self.settings.webhook_configured,
            },
            'limits': self._limits(),
            'provider': {
                'name': 'groq',
                'asr_model': self.settings.asr_model,
                'grammar_model': self.settings.grammar_model,
                'languages': sorted(LANGUAGES),
            },
            'privacy': {
                'audio_retention': 'processed in memory; not stored by this server',
                'text_retention_seconds': self.settings.result_ttl_seconds,
                'server_holds_provider_key': bool(self.settings.groq_api_key),
            },
        }

    # -------------------------------------------------------------- dictation

    def create_dictation(self, user: dict, *, fields: dict, files: dict,
                         idempotency_key: str | None, request_id: str) -> tuple[int, dict]:
        retry = self.rate_limiter.check(user['id'])
        if retry:
            raise ApiError(429, 'RATE_LIMITED', 'Too many dictation requests',
                           retryable=True, retry_after=retry)

        if not idempotency_key:
            raise ApiError(400, 'IDEMPOTENCY_KEY_REQUIRED',
                           'Idempotency-Key header is required')
        if not CLIENT_JOB_RE.fullmatch(idempotency_key):
            raise ApiError(400, 'INVALID_IDEMPOTENCY_KEY',
                           'Idempotency-Key must be 8-128 safe characters')
        field_client_id = fields.get('client_job_id', '').strip()
        if field_client_id and field_client_id != idempotency_key:
            raise ApiError(400, 'IDEMPOTENCY_KEY_MISMATCH',
                           'client_job_id must match Idempotency-Key')

        language = (fields.get('language') or 'auto').strip()
        if language not in LANGUAGES:
            raise ApiError(400, 'UNSUPPORTED_LANGUAGE', 'Unsupported language code')
        mode = (fields.get('grammar_mode') or 'standard').strip().lower()
        if mode not in MODES:
            raise ApiError(400, 'INVALID_GRAMMAR_MODE', 'grammar_mode is invalid')
        mode = {'smart': 'standard', 'polish': 'polished'}.get(mode, mode)
        client_version = (fields.get('client_version') or 'unknown').strip()[:64]

        upload = files.get('audio')
        if not upload or not upload.get('data'):
            raise ApiError(400, 'INVALID_AUDIO', 'Missing audio form field', retryable=False)
        if not getattr(self.provider, 'available', True):
            raise ApiError(503, 'PROVIDER_NOT_CONFIGURED',
                           'Server transcription provider is not configured', retryable=True)
        try:
            probed = audio.probe(upload['data'], max_bytes=self.settings.max_audio_bytes,
                                 max_seconds=self.settings.max_audio_seconds)
        except audio.AudioError as exc:
            raise ApiError(exc.status, exc.code, exc.message) from exc

        conn = self._connect()
        try:
            subscription = db.get_subscription(conn, user['id'])
            plan = db.effective_plan(subscription)
            limit = self.settings.limit_for(plan)
            try:
                job, created = db.create_or_get_job(
                    conn, user_id=user['id'], client_job_id=idempotency_key,
                    billable_seconds=probed['billable_seconds'], language=language,
                    grammar_mode=mode, client_version=client_version,
                    timeout_seconds=self.settings.job_timeout_seconds,
                    limit_seconds=limit,
                )
            except db.DbConflict as exc:
                status = 402 if exc.code == 'QUOTA_EXCEEDED' else 409
                raise ApiError(status, exc.code, exc.message,
                               retryable=exc.retryable) from exc
        finally:
            conn.close()

        if not created:
            return self._existing_job(user, job)

        return self._run_job(user, job, upload, language, mode, probed, request_id)

    def _existing_job(self, user: dict, job: dict) -> tuple[int, dict]:
        state = job['state']
        if state == 'completed':
            if db.job_is_expired(job):
                conn = self._connect()
                try:
                    db.clear_expired_result(conn, job)
                finally:
                    conn.close()
                raise ApiError(410, 'RESULT_EXPIRED', 'The stored result has expired')
            conn = self._connect()
            try:
                return 200, self._job_payload(conn, user, job)
            finally:
                conn.close()
        if state == 'processing':
            return 202, {'job_id': job['id'], 'status': 'processing', 'poll_after_ms': 500}
        if state == 'cancelled':
            raise ApiError(409, 'JOB_CANCELLED', 'The original job was cancelled')
        raise ApiError(409, 'IDEMPOTENCY_CONFLICT',
                       'The original job failed; use a new Idempotency-Key',
                       extra={'job_id': job['id']})

    def _run_job(self, user: dict, job: dict, upload: dict, language: str,
                 mode: str, probed: dict, request_id: str) -> tuple[int, dict]:
        asr_started = time.monotonic()
        try:
            raw = self.provider.transcribe(
                upload['data'],
                filename=upload.get('filename') or 'audio.wav',
                content_type=upload.get('content_type') or 'application/octet-stream',
                language=language,
                timeout=self.settings.job_timeout_seconds,
            )
            asr_ms = int((time.monotonic() - asr_started) * 1000)
            conn = self._connect()
            try:
                if not db.job_still_processing(conn, user['id'], job['id']):
                    raise ApiError(409, 'JOB_CANCELLED', 'Job was cancelled during transcription')
            finally:
                conn.close()

            grammar_started = time.monotonic()
            if mode == 'raw':
                output, warning = raw, None
            else:
                output, warning = self.provider.correct(
                    raw, mode=mode, language=language)
            grammar_ms = int((time.monotonic() - grammar_started) * 1000)
        except ApiError:
            conn = self._connect()
            try:
                db.fail_job(conn, job['id'], user['id'], 'JOB_CANCELLED', 'Cancelled by user')
            finally:
                conn.close()
            raise
        except ProviderError as exc:
            conn = self._connect()
            try:
                db.fail_job(conn, job['id'], user['id'], exc.code, exc.message)
            finally:
                conn.close()
            logger.warning('provider_error job=%s code=%s retryable=%s',
                           job['id'], exc.code, exc.retryable)
            raise ApiError(exc.status, exc.code, exc.message,
                           retryable=exc.retryable,
                           retry_after=exc.retry_after,
                           extra={'job_id': job['id']}) from exc
        except Exception:
            conn = self._connect()
            try:
                db.fail_job(conn, job['id'], user['id'], 'INTERNAL_ERROR',
                            'Internal processing error')
            finally:
                conn.close()
            logger.exception('job_internal_error job=%s request=%s', job['id'], request_id)
            raise ApiError(500, 'INTERNAL_ERROR', 'Internal processing error',
                           extra={'job_id': job['id']})

        conn = self._connect()
        try:
            committed = db.complete_job(
                conn, job_id=job['id'], user_id=user['id'],
                duration_ms=probed['duration_ms'], raw_text=raw, output_text=output,
                # The JSON ASR format does not report a detected language. Do not
                # echo the requested code as if it were detected.
                detected_language=None,
                warning=warning,
                processing={'asr_ms': asr_ms, 'grammar_ms': grammar_ms},
                result_ttl_seconds=self.settings.result_ttl_seconds,
            )
            if not committed:
                raise ApiError(409, 'JOB_CANCELLED', 'Job was cancelled before completion')
            job = db.get_job(conn, user['id'], job['id'])
            return 200, self._job_payload(conn, user, job)
        finally:
            conn.close()

    def _job_payload(self, conn, user: dict, job: dict) -> dict:
        subscription = db.get_subscription(conn, user['id'])
        plan = db.effective_plan(subscription)
        limit = self.settings.limit_for(plan)
        usage = db.usage(conn, user['id'])
        warnings = [job['warning']] if job.get('warning') else []
        try:
            processing = json.loads(job.get('processing_json') or '{}')
        except ValueError:
            processing = {}
        remaining = max(0, limit - usage['used_seconds'] - usage['reserved_seconds'])
        return {
            'job_id': job['id'],
            'status': 'completed',
            'raw_text': job.get('raw_text'),
            'corrected_text': job.get('output_text'),
            'output_text': job.get('output_text'),
            'language': job.get('language_code'),
            'detected_language': job.get('detected_language'),
            'grammar_mode': job.get('grammar_mode'),
            'billing': {
                'plan': plan,
                'charged_audio_seconds': job.get('billable_seconds', 0),
                'remaining_seconds': remaining,
                'remaining_free_seconds': remaining if plan == 'free' else None,
            },
            'warnings': warnings,
            'processing': processing,
        }

    def get_job(self, user: dict, job_id: str) -> tuple[int, dict]:
        conn = self._connect()
        try:
            job = db.get_job(conn, user['id'], job_id)
            if not job:
                raise ApiError(404, 'JOB_NOT_FOUND', 'Job not found')
            if job['state'] == 'completed':
                if db.job_is_expired(job):
                    db.clear_expired_result(conn, job)
                    raise ApiError(410, 'RESULT_EXPIRED', 'The stored result has expired')
                return 200, self._job_payload(conn, user, job)
            if job['state'] == 'processing':
                return 202, {'job_id': job['id'], 'status': 'processing', 'poll_after_ms': 500}
            if job['state'] == 'cancelled':
                raise ApiError(409, 'JOB_CANCELLED', 'Job was cancelled',
                               extra={'job_id': job['id']})
            raise ApiError(409, 'JOB_FAILED',
                           job.get('error_message') or 'Job failed',
                           extra={'job_id': job['id'], 'job_code': job.get('error_code')})
        finally:
            conn.close()

    def cancel_job(self, user: dict, job_id: str) -> dict:
        conn = self._connect()
        try:
            job = db.cancel_job(conn, user['id'], job_id)
        finally:
            conn.close()
        if not job:
            raise ApiError(404, 'JOB_NOT_FOUND', 'Job not found')
        return {'job_id': job['id'], 'status': job['state']}

    def recent(self, user: dict, limit: int = 20) -> dict:
        conn = self._connect()
        try:
            jobs = db.recent_jobs(conn, user['id'], limit)
        finally:
            conn.close()
        return {'jobs': jobs}

    # ----------------------------------------------------------------- billing

    def checkout(self, user: dict, body: dict) -> dict:
        plan = str(body.get('plan', 'monthly')).lower()
        if plan not in PRICE_PLANS:
            raise ApiError(400, 'INVALID_PLAN', 'plan must be monthly or annual')
        price = (self.settings.stripe_price_monthly if plan == 'monthly'
                 else self.settings.stripe_price_annual)
        conn = self._connect()
        try:
            subscription = db.get_subscription(conn, user['id'])
        finally:
            conn.close()
        try:
            return self.stripe.create_checkout(
                user, price_id=price,
                customer_id=subscription.get('stripe_customer_id'))
        except stripe_mod.BillingError as exc:
            raise ApiError(exc.status, exc.code, exc.message) from exc

    def portal(self, user: dict) -> dict:
        conn = self._connect()
        try:
            subscription = db.get_subscription(conn, user['id'])
        finally:
            conn.close()
        try:
            return self.stripe.create_portal(
                customer_id=subscription.get('stripe_customer_id'))
        except stripe_mod.BillingError as exc:
            raise ApiError(exc.status, exc.code, exc.message) from exc

    def stripe_webhook(self, payload: bytes, signature: str | None) -> dict:
        if not self.settings.webhook_configured:
            raise ApiError(503, 'BILLING_NOT_CONFIGURED', 'Stripe webhook is not configured')
        if not stripe_mod.verify_signature(payload, signature,
                                           self.settings.stripe_webhook_secret):
            raise ApiError(400, 'INVALID_SIGNATURE', 'Stripe signature verification failed')
        try:
            event = json.loads(payload.decode('utf-8'))
        except ValueError as exc:
            raise ApiError(400, 'INVALID_EVENT', 'Stripe event is not valid JSON') from exc
        if not isinstance(event, dict):
            raise ApiError(400, 'INVALID_EVENT', 'Stripe event must be an object')
        event_id = event.get('id')
        if not isinstance(event_id, str) or not event_id or len(event_id) > 200:
            raise ApiError(400, 'INVALID_EVENT', 'Stripe event id is missing')
        if not isinstance(event.get('type'), str) or not event['type']:
            raise ApiError(400, 'INVALID_EVENT', 'Stripe event type is missing')
        conn = self._connect()
        try:
            if db.webhook_seen(conn, event_id):
                return {'received': True, 'duplicate': True, 'applied': False}
            result = stripe_mod.apply_event(conn, event)
            recorded = db.record_webhook_event(conn, event_id)
        finally:
            conn.close()
        return {'received': True, 'duplicate': not recorded,
                'applied': result.get('applied', False),
                'detail': result.get('reason') or result.get('status')}

    # ----------------------------------------------------------------- privacy

    def delete_data(self, user: dict, body: dict) -> dict:
        if not isinstance(body, dict) or body.get('confirm') != 'DELETE':
            raise ApiError(400, 'CONFIRMATION_REQUIRED',
                           'Send {"confirm":"DELETE"} to delete local account data')
        conn = self._connect()
        try:
            result = db.delete_user_data(conn, user['id'])
        finally:
            conn.close()
        return {'status': 'deletion_requested', **result}
