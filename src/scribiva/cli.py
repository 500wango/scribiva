"""Command line and GNOME custom hotkey entrypoints."""
from __future__ import annotations
import argparse
import getpass
import json
import subprocess
import sys
from . import __version__
from . import devices, engine, inject, providers
from .config import (LANGUAGES, LATEST_PATH, MIC_PATH, SERVICE_TOKEN_RE,
                     clear_mic_node, clear_service_token, mic_node, read_config,
                     save_config, service_token, validate_service_url,
                     write_api_key, write_service_token)


def _configured_source(conf: dict) -> str:
    try:
        legacy = MIC_PATH.read_text(encoding='utf-8').strip() if MIC_PATH.exists() else ''
    except OSError:
        legacy = ''
    return conf.get('mic_node', '').strip() or legacy


def _print_devices(as_json: bool) -> None:
    conf = read_config()
    info = devices.list_sources()
    info['configured'] = _configured_source(conf)
    try:
        info['active'] = mic_node(conf, info)
    except RuntimeError:
        info['active'] = None
    if as_json:
        print(json.dumps(info, indent=2, ensure_ascii=False))
        return
    if info.get('error'):
        print('警告: ' + info['error'])
    if not info['devices']:
        print('未发现可用录音设备。')
    for device in info['devices']:
        marks = []
        if device['node'] == info['active']:
            marks.append('当前')
        if device.get('default'):
            marks.append('默认')
        if device['virtual']:
            marks.append('虚拟')
        suffix = ('  [' + '/'.join(marks) + ']') if marks else ''
        print(f"{device['node']}\t{device['label']}{suffix}")
    if info['active']:
        print('\n当前使用: ' + info['active'])
    else:
        print('\n当前使用: 未配置')


def _print_last(raw: bool, as_json: bool) -> None:
    data = engine.last_result()
    if as_json:
        print(json.dumps(data, indent=2, ensure_ascii=False))
        return
    text = data['raw'] if raw else data['text']
    if not text:
        raise RuntimeError('No result yet' if not data['available']
                           else ('No raw transcript saved' if raw else 'No processed text saved'))
    print(text, end='' if text.endswith('\n') else '\n')


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(prog='scribiva', description='Scribiva Linux developer alpha')
    parser.add_argument('--version', action='version', version=__version__)
    commands = parser.add_subparsers(dest='cmd', required=True)
    commands.add_parser('toggle', help='Toggle recording (GNOME shortcut target)')
    commands.add_parser('cancel', help='Cancel recording or prevent a pending result')
    commands.add_parser('discard', help='Delete a retained failed recording or drop a pending result')
    commands.add_parser('retry', help='Retry last failed audio from disk')
    commands.add_parser('status', help='Show current job status (never prints API key)')
    commands.add_parser('doctor', help='Check operating environment')
    commands.add_parser('settings', help='Display settings')
    commands.add_parser('languages', help='Display supported ASR languages')
    commands.add_parser('key', help='Enter Groq development key privately')
    p = commands.add_parser('service', help='Sign in, show quota, or manage billing')
    p.add_argument('action', choices=['login', 'status', 'logout', 'billing', 'portal', 'delete'])
    p.add_argument('--url', help='API base URL (required for the first login)')
    p.add_argument('--plan', choices=['monthly', 'annual'], default='monthly',
                   help='Subscription plan for `service billing`')
    p.add_argument('--confirm', action='store_true',
                   help='Required for `service delete` (server-side account data deletion)')
    commands.add_parser('copy-last', help='Copy last transcript')
    commands.add_parser('paste-last', help='Manually dispatch last transcript once')
    commands.add_parser('clear-last', help='Delete locally stored transcripts')
    commands.add_parser('ui', help='Launch local settings dashboard')
    p = commands.add_parser('devices', help='List PipeWire capture devices')
    p.add_argument('--json', action='store_true', help='Machine-readable output')
    p = commands.add_parser('last', help='Print the last transcript')
    p.add_argument('--raw', action='store_true', help='Print the ASR transcript before editing')
    p.add_argument('--json', action='store_true', help='Include both raw and edited text as JSON')
    p = commands.add_parser('set', help='Update a single setting')
    p.add_argument('name')
    p.add_argument('value')
    p = commands.add_parser('worker', help='Internal: process a queued job')
    p.add_argument('job_id')
    p = commands.add_parser('watchdog', help='Internal: stop an overlong recording')
    p.add_argument('job_id')
    args = parser.parse_args(argv)
    try:
        if args.cmd == 'toggle':
            print(engine.toggle())
        elif args.cmd == 'cancel':
            engine.cancel()
            print('cancelled')
        elif args.cmd == 'discard':
            print(engine.discard())
        elif args.cmd == 'retry':
            engine.retry()
            print('retry started')
        elif args.cmd == 'worker':
            engine.worker(args.job_id)
        elif args.cmd == 'watchdog':
            engine.watchdog(args.job_id)
        elif args.cmd == 'status':
            s = engine.state()
            s.pop('file', None)
            s.pop('pid', None)
            print(json.dumps(s, indent=2, ensure_ascii=False))
        elif args.cmd == 'doctor':
            print(json.dumps(engine.doctor(), indent=2, ensure_ascii=False))
        elif args.cmd == 'settings':
            data = read_config()
            # Proxy credentials never printed.
            if data.get('proxy_url'):
                data['proxy_url'] = '(configured)'
            try:
                data['service_token_present'] = bool(service_token())
            except (RuntimeError, OSError):
                data['service_token_present'] = False
            print(json.dumps(data, indent=2, ensure_ascii=False))
        elif args.cmd == 'languages':
            for code, name in LANGUAGES.items():
                print(code, name)
        elif args.cmd == 'devices':
            _print_devices(args.json)
        elif args.cmd == 'last':
            _print_last(args.raw, args.json)
        elif args.cmd == 'clear-last':
            print('last result cleared' if engine.clear_last() else 'nothing to clear')
        elif args.cmd == 'key':
            entered = getpass.getpass('Groq API Key (not echoed): ')
            write_api_key(entered)
            print('Key saved with mode 0600')
        elif args.cmd == 'service':
            if args.action == 'logout':
                removed = clear_service_token()
                save_config({'service_url': ''})
                print('Service token removed' if removed else 'No service token was stored')
                print('Client is back in direct Groq developer mode')
                return 0
            conf = read_config()
            if args.action == 'login':
                url = validate_service_url(args.url or conf.get('service_url', ''))
                if not url:
                    raise ValueError('Service URL required: scribiva service login --url https://...')
                token = getpass.getpass('Scribiva service token (not echoed): ').strip()
                if not SERVICE_TOKEN_RE.fullmatch(token):
                    raise ValueError('Invalid service token format (expected scr_...)')
                account = providers.service_me({**conf, 'service_url': url}, token=token)
                save_config({'service_url': url})
                write_service_token(token)
                plan = account.get('plan', 'unknown')
                usage = account.get('usage', {}) if isinstance(account.get('usage'), dict) else {}
                print(f'Service configured: {url}')
                print(f"Plan: {plan} · used {usage.get('used_seconds', 0)}s · "
                      f"remaining {usage.get('remaining_seconds', '?')}s")
                return 0
            if not providers.service_configured(conf):
                raise RuntimeError('Service is not configured; run `scribiva service login --url ...`')
            if args.action == 'status':
                data = providers.service_status(conf)
                print(json.dumps(data, indent=2, ensure_ascii=False))
            elif args.action == 'billing':
                data = providers.service_checkout(conf, args.plan)
                print(data['url'])
                print('Open the URL above to complete Stripe Checkout; '
                      'Pro is granted only after a signed subscription webhook.', file=sys.stderr)
            elif args.action == 'portal':
                data = providers.service_portal(conf)
                print(data['url'])
                print('Open the URL above to manage or cancel the subscription.', file=sys.stderr)
            elif args.action == 'delete':
                if not args.confirm:
                    raise ValueError('Refusing to delete server data without --confirm')
                result = providers.service_delete(conf)
                clear_service_token()
                save_config({'service_url': ''})
                print(json.dumps(result, indent=2, ensure_ascii=False))
                print('Local service token and URL cleared')
            else:
                raise ValueError('Unsupported service action')
        elif args.cmd == 'set':
            name = args.name
            if name not in read_config():
                raise ValueError('Unknown setting: ' + name)
            if name == 'mic_node' and args.value.strip().lower() in ('', 'auto', 'default', 'system'):
                clear_mic_node()
                print('mic_node updated (system default)')
                return 0
            previous = read_config()[name]
            if isinstance(previous, bool):
                if args.value.lower() not in ('true', 'false'):
                    raise ValueError('Expected true or false')
                value = args.value.lower() == 'true'
            elif isinstance(previous, int):
                value = int(args.value)
            else:
                value = args.value
            save_config({name: value})
            if name == 'mic_node':
                known = {d['node'] for d in devices.list_sources()['devices']}
                if value not in known:
                    print('Warning: this node is not present right now; it will be used when it appears.',
                          file=sys.stderr)
            print(name + ' updated')
        elif args.cmd in ('copy-last', 'paste-last'):
            if not LATEST_PATH.is_file():
                raise RuntimeError('No result yet')
            text = LATEST_PATH.read_text(encoding='utf-8')
            if args.cmd == 'copy-last':
                inject.copy_text(text)
                print('Last result copied')
            else:
                print(inject.dispatch(text))
        elif args.cmd == 'ui':
            from .dashboard import serve
            serve()
        return 0
    except (RuntimeError, ValueError, OSError, subprocess.CalledProcessError) as e:
        print('Scribiva error: ' + str(e), file=sys.stderr)
        return 1
