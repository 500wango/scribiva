"""Command line: serve the API and perform local account administration."""
from __future__ import annotations
import argparse
import dataclasses
import ipaddress
import json
import logging
import os
import sys
from datetime import timedelta
from pathlib import Path

from . import __version__, auth, db
from .config import ConfigError, Settings
from .http import create_server


def _settings(args) -> Settings:
    base = Settings.from_env()
    overrides = {}
    if getattr(args, 'db', None):
        overrides['db_path'] = Path(args.db).expanduser()
    if getattr(args, 'host', None):
        overrides['host'] = args.host
    if getattr(args, 'port', None) is not None:
        if not 0 <= args.port <= 65535:
            raise ConfigError('--port must be 0..65535')
        overrides['port'] = args.port
    if overrides:
        if 'db_path' in overrides and overrides['db_path'].suffix == '':
            raise ConfigError('--db must point to a file')
        base = dataclasses.replace(base, **overrides)
        object.__setattr__(base, 'max_request_bytes', base.max_audio_bytes + 1_048_576)
    return base


def _is_loopback(host: str) -> bool:
    if host in ('localhost', ''):
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def _open_db(settings: Settings):
    db.init_db(settings.db_path)
    return db.connect(settings.db_path)


def cmd_serve(args) -> int:
    settings = _settings(args)
    if not _is_loopback(settings.host) and not settings.allow_remote:
        print('Refusing non-loopback bind without SCRIBIVA_ALLOW_REMOTE=1.\n'
              'Put the API behind a TLS reverse proxy in production.', file=sys.stderr)
        return 2
    os.umask(0o077)
    logging.basicConfig(level=logging.INFO,
                        format='%(asctime)s %(levelname)s %(name)s %(message)s')
    server = create_server(settings)
    address = f'http://{settings.host}:{server.server_port}'
    print(f'Scribiva API {__version__} listening on {address}', flush=True)
    print(f'Database: {settings.db_path}', flush=True)
    if not settings.groq_api_key:
        print('WARNING: GROQ_API_KEY is not set; dictation returns 503 PROVIDER_NOT_CONFIGURED',
              file=sys.stderr, flush=True)
    if not _is_loopback(settings.host):
        print('WARNING: remote binding requested; terminate TLS in a reverse proxy and '
              'restrict network access', file=sys.stderr, flush=True)
    try:
        server.serve_forever(poll_interval=0.25)
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


def cmd_init_db(args) -> int:
    settings = _settings(args)
    db.init_db(settings.db_path)
    print(f'Initialized {settings.db_path}')
    return 0


def cmd_create_user(args) -> int:
    settings = _settings(args)
    conn = _open_db(settings)
    try:
        try:
            user = db.create_user(conn, args.email)
        except Exception as exc:
            print('Could not create user: ' + str(exc), file=sys.stderr)
            return 1
        token = auth.issue_token(conn, user['id'], settings.token_ttl_days)
    finally:
        conn.close()
    print(json.dumps({'user_id': user['id'], 'email': user['email'],
                      'token': token,
                      'expires_days': settings.token_ttl_days}, indent=2))
    print('Store this token now; it is not recoverable from the database.', file=sys.stderr)
    return 0


def _find_or_fail(conn, identifier: str):
    user = db.find_user(conn, identifier)
    if not user:
        print('User not found: ' + identifier, file=sys.stderr)
        return None
    return user


def cmd_issue_token(args) -> int:
    settings = _settings(args)
    conn = _open_db(settings)
    try:
        user = _find_or_fail(conn, args.user)
        if not user:
            return 1
        token = auth.issue_token(conn, user['id'], args.days or settings.token_ttl_days)
    finally:
        conn.close()
    print(json.dumps({'user_id': user['id'], 'email': user['email'], 'token': token},
                     indent=2))
    print('Store this token now; it is not recoverable from the database.', file=sys.stderr)
    return 0


def cmd_revoke_token(args) -> int:
    settings = _settings(args)
    conn = _open_db(settings)
    try:
        revoked = auth.revoke_token(conn, args.token)
    finally:
        conn.close()
    print('revoked' if revoked else 'token not found or already revoked')
    return 0 if revoked else 1


def cmd_show_user(args) -> int:
    settings = _settings(args)
    conn = _open_db(settings)
    try:
        user = _find_or_fail(conn, args.user)
        if not user:
            return 1
        subscription = db.get_subscription(conn, user['id'])
        plan = db.effective_plan(subscription)
        usage = db.usage(conn, user['id'])
        limit = settings.limit_for(plan)
        payload = {
            'user': {'id': user['id'], 'email': user['email'],
                     'created_at': user['created_at'],
                     'deletion_requested_at': user['deletion_requested_at']},
            'plan': plan,
            'subscription': subscription,
            'usage': {**usage, 'limit_seconds': limit,
                      'remaining_seconds': max(0, limit - usage['used_seconds']
                                               - usage['reserved_seconds'])},
        }
    finally:
        conn.close()
    print(json.dumps(payload, indent=2, ensure_ascii=False))
    return 0


def cmd_grant_pro(args) -> int:
    settings = _settings(args)
    conn = _open_db(settings)
    try:
        user = _find_or_fail(conn, args.user)
        if not user:
            return 1
        period_end = db.utcnow() + timedelta(days=args.days)
        db.set_subscription(conn, user['id'], status=args.status, plan_code='pro',
                            customer_id=args.customer,
                            current_period_end=db.iso(period_end))
    finally:
        conn.close()
    print(f"{user['id']} plan=pro status={args.status} until {db.iso(period_end)}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog='scribiva-server',
                                     description='Scribiva server proxy and quota skeleton')
    parser.add_argument('--version', action='version', version=__version__)
    sub = parser.add_subparsers(dest='cmd', required=True)

    p = sub.add_parser('serve', help='Run the HTTP API')
    p.add_argument('--host')
    p.add_argument('--port', type=int)
    p.add_argument('--db')
    p.set_defaults(func=cmd_serve)

    p = sub.add_parser('init-db', help='Create the SQLite schema')
    p.add_argument('--db')
    p.set_defaults(func=cmd_init_db)

    p = sub.add_parser('create-user', help='Create a user and print a token once')
    p.add_argument('--email')
    p.add_argument('--db')
    p.set_defaults(func=cmd_create_user)

    p = sub.add_parser('issue-token', help='Issue another token for a user')
    p.add_argument('--user', required=True)
    p.add_argument('--days', type=int)
    p.add_argument('--db')
    p.set_defaults(func=cmd_issue_token)

    p = sub.add_parser('revoke-token', help='Revoke a token')
    p.add_argument('--token', required=True)
    p.add_argument('--db')
    p.set_defaults(func=cmd_revoke_token)

    p = sub.add_parser('show-user', help='Show plan and usage')
    p.add_argument('--user', required=True)
    p.add_argument('--db')
    p.set_defaults(func=cmd_show_user)

    p = sub.add_parser('grant-pro', help='Local development entitlement override')
    p.add_argument('--user', required=True)
    p.add_argument('--status', default='active',
                   choices=['active', 'trialing', 'past_due', 'canceled', 'inactive'])
    p.add_argument('--days', type=int, default=30)
    p.add_argument('--customer')
    p.add_argument('--db')
    p.set_defaults(func=cmd_grant_pro)
    return parser


def main(argv=None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except ConfigError as exc:
        print('Configuration error: ' + str(exc), file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
