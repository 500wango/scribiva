"""PipeWire audio source discovery with stable ``node.name`` identifiers.

``pw-dump`` is a single read-only call that exposes both the source nodes and
the WirePlumber default-source metadata, so listing devices stays consistent
even while the default changes. If ``pw-dump`` is unavailable we fall back to
``wpctl inspect @DEFAULT_AUDIO_SOURCE@`` and report a diagnostic string rather
than failing the whole command.
"""
from __future__ import annotations
import json
import re
import shutil
import subprocess
from typing import Any

SOURCE_CLASSES = {'Audio/Source', 'Audio/Source/Virtual'}
NODE_NAME_RE = re.compile(r'node\.name\s*=\s*"([^"\n]+)"')


class DeviceError(RuntimeError):
    pass


def _text(value: Any, limit: int = 200) -> str:
    if not isinstance(value, str):
        return ''
    cleaned = ' '.join(value.split())
    return cleaned[:limit]


def _int(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def _truthy(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        return value.strip().lower() in ('1', 'true', 'yes')
    return False


def _label(props: dict, node: str) -> str:
    for key in ('node.description', 'node.nick', 'device.description'):
        value = _text(props.get(key), 160)
        if value:
            return value
    return node


def _rank(device: dict) -> tuple:
    # Lower is better: physical before virtual, high session priority, low id.
    return (device['virtual'], -device['priority'], device['id'] if device['id'] is not None else 1 << 31)


def _default_name(value: Any) -> str | None:
    name = None
    if isinstance(value, dict):
        name = value.get('name')
    elif isinstance(value, str):
        try:
            parsed = json.loads(value)
            if isinstance(parsed, dict):
                name = parsed.get('name')
        except ValueError:
            name = value
    return _text(name) or None


def parse_pw_dump(data: Any) -> dict:
    """Convert parsed ``pw-dump`` JSON into sources and the default node."""
    if not isinstance(data, list):
        raise DeviceError('pw-dump returned unexpected JSON')
    sources: dict[str, dict] = {}
    default = None
    for obj in data:
        if not isinstance(obj, dict):
            continue
        kind = obj.get('type')
        if kind == 'PipeWire:Interface:Node':
            info = obj.get('info')
            props = info.get('props') if isinstance(info, dict) else None
            if not isinstance(props, dict):
                continue
            media_class = props.get('media.class')
            if media_class not in SOURCE_CLASSES:
                continue
            node = _text(props.get('node.name'))
            if not node:
                continue
            candidate = {
                'node': node,
                'label': _label(props, node),
                'virtual': media_class == 'Audio/Source/Virtual' or _truthy(props.get('node.virtual')),
                'priority': _int(props.get('priority.session')),
                'id': obj.get('id') if isinstance(obj.get('id'), int) else None,
            }
            existing = sources.get(node)
            if existing is None or _rank(candidate) < _rank(existing):
                sources[node] = candidate
        elif kind == 'PipeWire:Interface:Metadata' and default is None:
            props = obj.get('props')
            if not isinstance(props, dict) or props.get('metadata.name') != 'default':
                continue
            metadata = obj.get('metadata')
            if not isinstance(metadata, list):
                continue
            for item in metadata:
                if isinstance(item, dict) and item.get('key') == 'default.audio.source':
                    default = _default_name(item.get('value')) or default
    devices = sorted(
        sources.values(),
        key=lambda d: (d['node'] != default, d['virtual'], -d['priority'],
                       d['label'].casefold(), d['node']),
    )
    for device in devices:
        device['default'] = device['node'] == default
    return {'devices': devices, 'default': default, 'error': None, 'source': 'pw-dump'}


def _run(command: list[str], timeout: float):
    try:
        return subprocess.run(command, capture_output=True, text=True,
                              timeout=timeout, check=False)
    except subprocess.TimeoutExpired as exc:
        raise DeviceError(f'{command[0]} timed out') from exc
    except OSError as exc:
        raise DeviceError(f'{command[0]} unavailable: {exc}') from exc


def _wpctl_source(timeout: float = 5) -> dict | None:
    if not shutil.which('wpctl'):
        return None
    try:
        proc = _run(['wpctl', 'inspect', '@DEFAULT_AUDIO_SOURCE@'], timeout)
    except DeviceError:
        return None
    if proc.returncode != 0:
        return None
    output = proc.stdout or ''
    match = NODE_NAME_RE.search(output)
    if not match:
        return None
    node = _text(match.group(1))
    desc = re.search(r'node\.description\s*=\s*"([^"\n]+)"', output)
    label = _text(desc.group(1), 160) if desc else node
    return {'node': node, 'label': label, 'virtual': False,
            'priority': 0, 'id': None, 'default': True}


def _fallback(error: str, timeout: float) -> dict:
    source = _wpctl_source(timeout)
    return {'devices': [source] if source else [],
            'default': source['node'] if source else None,
            'error': error, 'source': 'wpctl' if shutil.which('wpctl') else 'none'}


def list_sources(timeout: float = 5) -> dict:
    """Return ``{devices, default, error, source}``; never raises for environment issues."""
    if not shutil.which('pw-dump'):
        return _fallback('pw-dump is not installed; install pipewire-bin', timeout)
    try:
        proc = _run(['pw-dump'], timeout)
    except DeviceError as exc:
        return _fallback(str(exc), timeout)
    if proc.returncode != 0:
        detail = _text(proc.stderr, 160) or f'exit {proc.returncode}'
        return _fallback('pw-dump failed: ' + detail, timeout)
    try:
        result = parse_pw_dump(json.loads(proc.stdout or '[]'))
    except (ValueError, DeviceError) as exc:
        return _fallback('Cannot parse pw-dump output: ' + str(exc), timeout)
    if not result['default']:
        fallback = _wpctl_source(timeout)
        if fallback:
            result['default'] = fallback['node']
            if not any(device['node'] == fallback['node'] for device in result['devices']):
                result['devices'].append(fallback)
        for device in result['devices']:
            device.setdefault('default', False)
            device['default'] = device['node'] == result['default']
        result['devices'].sort(
            key=lambda d: (d['node'] != result['default'], d['virtual'],
                           -d['priority'], d['label'].casefold(), d['node']))
    return result


def default_source(timeout: float = 5) -> str | None:
    return list_sources(timeout).get('default')
