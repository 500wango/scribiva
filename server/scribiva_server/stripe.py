"""Stripe signature verification and minimal Checkout/Portal/webhook mapping.

This is a skeleton: no Stripe SDK, no card data, and no redirect is trusted for
entitlements. Only a signed webhook can move a subscription to active.
"""
from __future__ import annotations
import hashlib
import hmac
import json
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone

from . import db
from .config import Settings


class BillingError(RuntimeError):
    def __init__(self, code: str, message: str, status: int = 503):
        super().__init__(message)
        self.code = code
        self.message = message
        self.status = status


def verify_signature(payload: bytes, header: str | None, secret: str,
                     tolerance: int = 300, now: int | None = None) -> bool:
    if not header or not secret:
        return False
    timestamp = None
    signatures: list[str] = []
    for part in header.split(','):
        key, _, value = part.strip().partition('=')
        if key == 't':
            try:
                timestamp = int(value)
            except ValueError:
                return False
        elif key == 'v1':
            signatures.append(value.strip())
    if timestamp is None or not signatures:
        return False
    current = int(time.time()) if now is None else now
    if abs(current - timestamp) > tolerance:
        return False
    signed = str(timestamp).encode('ascii') + b'.' + payload
    expected = hmac.new(secret.encode('utf-8'), signed, hashlib.sha256).hexdigest()
    return any(hmac.compare_digest(expected, candidate) for candidate in signatures)


def _stripe_time(value) -> str | None:
    try:
        stamp = int(value)
    except (TypeError, ValueError):
        return None
    return datetime.fromtimestamp(stamp, timezone.utc).isoformat(timespec='seconds')


class StripeClient:
    def __init__(self, settings: Settings):
        self.settings = settings

    def _post(self, path: str, params: dict, timeout: int = 25) -> dict:
        if not self.settings.stripe_secret_key:
            raise BillingError('BILLING_NOT_CONFIGURED', 'Stripe is not configured', 503)
        body = urllib.parse.urlencode(params).encode('utf-8')
        request = urllib.request.Request(
            'https://api.stripe.com' + path, data=body,
            headers={
                'Authorization': 'Bearer ' + self.settings.stripe_secret_key,
                'Content-Type': 'application/x-www-form-urlencoded',
                'User-Agent': 'Scribiva-Server/0.1',
            },
            method='POST',
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                raw = response.read(1_000_000)
        except urllib.error.HTTPError as exc:
            detail = ''
            try:
                parsed = json.loads(exc.read(4000).decode('utf-8', errors='replace'))
                detail = str(parsed.get('error', {}).get('message', ''))[:200]
            except (ValueError, AttributeError):
                pass
            raise BillingError('BILLING_PROVIDER_ERROR',
                               'Stripe request failed' + (': ' + detail if detail else ''),
                               502) from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise BillingError('BILLING_PROVIDER_TIMEOUT', 'Stripe could not be reached', 504) from exc
        try:
            result = json.loads(raw.decode('utf-8'))
        except ValueError as exc:
            raise BillingError('BILLING_PROVIDER_ERROR', 'Stripe returned invalid JSON', 502) from exc
        if not isinstance(result, dict):
            raise BillingError('BILLING_PROVIDER_ERROR', 'Stripe returned unexpected JSON', 502)
        return result

    def create_checkout(self, user: dict, *, price_id: str | None = None,
                        customer_id: str | None = None) -> dict:
        if not self.settings.billing_configured:
            raise BillingError('BILLING_NOT_CONFIGURED',
                               'Stripe secret and SCRIBIVA_PUBLIC_BASE_URL are required', 503)
        price = (price_id or self.settings.stripe_price_monthly).strip()
        if not price:
            raise BillingError('BILLING_NOT_CONFIGURED', 'No Stripe price configured', 503)
        base = self.settings.public_base_url
        params = {
            'mode': 'subscription',
            'line_items[0][price]': price,
            'line_items[0][quantity]': '1',
            'success_url': base + '/billing/success?session_id={CHECKOUT_SESSION_ID}',
            'cancel_url': base + '/billing/cancel',
            'client_reference_id': user['id'],
            'metadata[user_id]': user['id'],
        }
        if customer_id:
            params['customer'] = customer_id
        elif user.get('email'):
            params['customer_email'] = user['email']
        result = self._post('/v1/checkout/sessions', params)
        if not result.get('url'):
            raise BillingError('BILLING_PROVIDER_ERROR', 'Stripe did not return a checkout URL', 502)
        return {'url': result['url'], 'session_id': result.get('id')}

    def create_portal(self, *, customer_id: str | None) -> dict:
        if not self.settings.billing_configured:
            raise BillingError('BILLING_NOT_CONFIGURED',
                               'Stripe secret and SCRIBIVA_PUBLIC_BASE_URL are required', 503)
        if not customer_id:
            raise BillingError('NO_BILLING_ACCOUNT',
                               'No Stripe customer is linked to this account', 400)
        result = self._post('/v1/billing_portal/sessions', {
            'customer': customer_id,
            'return_url': self.settings.public_base_url + '/billing/portal-return',
        })
        if not result.get('url'):
            raise BillingError('BILLING_PROVIDER_ERROR', 'Stripe did not return a portal URL', 502)
        return {'url': result['url']}


def _event_user(conn, event: dict) -> str | None:
    data = event.get('data', {}).get('object', {}) if isinstance(event.get('data'), dict) else {}
    if not isinstance(data, dict):
        return None
    metadata = data.get('metadata')
    if isinstance(metadata, dict):
        candidate = metadata.get('user_id')
        if isinstance(candidate, str) and db.find_user(conn, candidate):
            return candidate
    candidate = data.get('client_reference_id')
    if isinstance(candidate, str) and db.find_user(conn, candidate):
        return candidate
    customer = data.get('customer')
    if isinstance(customer, str) and customer:
        row = conn.execute(
            'SELECT user_id FROM subscriptions WHERE stripe_customer_id = ?',
            (customer,)).fetchone()
        if row:
            return row['user_id']
    return None


def apply_event(conn, event: dict) -> dict:
    """Map a verified Stripe event onto subscription state. Idempotent."""
    event_type = event.get('type')
    data = event.get('data', {})
    obj = data.get('object', {}) if isinstance(data, dict) else {}
    if not isinstance(obj, dict):
        return {'applied': False, 'reason': 'missing object'}
    user_id = _event_user(conn, event)
    if not user_id:
        return {'applied': False, 'reason': 'unknown user'}
    customer = obj.get('customer') if isinstance(obj.get('customer'), str) else None
    subscription_id = obj.get('id') if event_type.startswith('customer.subscription.') else None
    if isinstance(obj.get('subscription'), str):
        subscription_id = obj['subscription']
    metadata = obj.get('metadata') if isinstance(obj.get('metadata'), dict) else {}
    plan_code = metadata.get('plan_code', 'pro')

    if event_type == 'checkout.session.completed':
        db.set_subscription(conn, user_id, status='incomplete', plan_code=plan_code,
                            customer_id=customer,
                            subscription_id=obj.get('subscription') if isinstance(
                                obj.get('subscription'), str) else subscription_id)
        return {'applied': True, 'status': 'incomplete'}

    if event_type in ('customer.subscription.created', 'customer.subscription.updated',
                      'customer.subscription.deleted'):
        status = obj.get('status') or ('canceled' if event_type.endswith('deleted') else 'inactive')
        if event_type.endswith('deleted'):
            status = 'canceled'
        db.set_subscription(
            conn, user_id, status=str(status), plan_code=plan_code,
            customer_id=customer, subscription_id=subscription_id,
            current_period_end=_stripe_time(obj.get('current_period_end')),
        )
        return {'applied': True, 'status': status}

    if event_type in ('invoice.payment_failed', 'invoice.payment_action_required'):
        db.set_subscription(conn, user_id, status='past_due', plan_code=plan_code,
                            customer_id=customer, subscription_id=subscription_id)
        return {'applied': True, 'status': 'past_due'}

    if event_type == 'invoice.paid':
        # Do not infer entitlement from an invoice; subscription events are the
        # authority. Record nothing beyond acknowledging the event.
        return {'applied': False, 'reason': 'subscription event required'}

    return {'applied': False, 'reason': 'ignored event'}
