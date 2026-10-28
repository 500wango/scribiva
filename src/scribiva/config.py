"""Configuration and secure local file paths. No third-party Python dependencies."""
from __future__ import annotations
import ipaddress
import json
import os
import re
from pathlib import Path
from urllib.parse import urlsplit

APP = 'scribiva'
ROOT_CONFIG = Path.home() / '.config' / APP
ROOT_STATE = Path.home() / '.local' / 'state' / APP
AUDIO_DIR = ROOT_STATE / 'audio'
CONFIG_PATH = ROOT_CONFIG / 'settings.json'
KEY_PATH = ROOT_CONFIG / 'groq-api-key'
SERVICE_TOKEN_PATH = ROOT_CONFIG / 'service-token'
MIC_PATH = ROOT_CONFIG / 'mic-node'
STATE_PATH = ROOT_STATE / 'session.json'
LOCK_PATH = ROOT_STATE / 'session.lock'
LATEST_PATH = ROOT_STATE / 'last-result.txt'
LAST_RAW_PATH = ROOT_STATE / 'last-raw.txt'
LOG_PATH = ROOT_STATE / 'scribiva.log'

LANGUAGES = {
    'auto': '自动检测', 'en': 'English', 'zh': '中文', 'ja': '日本語',
    'ko': '한국어', 'es': 'Español', 'fr': 'Français', 'de': 'Deutsch',
    'pt': 'Português', 'it': 'Italiano', 'ru': 'Русский', 'ar': 'العربية',
    'hi': 'हिन्दी', 'tr': 'Türkçe', 'nl': 'Nederlands', 'pl': 'Polski',
    'id': 'Bahasa Indonesia', 'vi': 'Tiếng Việt', 'th': 'ไทย',
    'sv': 'Svenska', 'uk': 'Українська',
}
MODES = {'raw', 'standard', 'polished'}
ASR_MODELS = {'whisper-large-v3', 'whisper-large-v3-turbo'}
OUTPUT_SCRIPTS = {'', 'hans', 'hant'}
DEFAULT = {
    'language': 'auto',
    'mode': 'standard',
    'asr_model': 'whisper-large-v3-turbo',
    'grammar_model': 'openai/gpt-oss-20b',
    'auto_insert': True,
    'restore_text_clipboard': True,
    'mic_node': '',
    'max_record_seconds': 180,
    'asr_prompt': '',
    'proxy_url': '',
    'service_url': '',
    'delete_audio_on_success': True,
    # v1.0 (FR-003, FR-016, ADR-012)
    'output_script': '',
    'dictation_commands': True,
    'insert_without_focus_check': False,
}

SERVICE_TOKEN_RE = re.compile(r'^scr_[A-Za-z0-9_-]{20,120}$')


def secure_dirs():
    os.umask(0o077)
    for path in (ROOT_CONFIG, ROOT_STATE, AUDIO_DIR):
        path.mkdir(parents=True, exist_ok=True)
        path.chmod(0o700)


def _write_private(path: Path, content: str):
    secure_dirs()
    tmp = path.with_name(path.name + f'.{os.getpid()}.tmp')
    try:
        with tmp.open('x', encoding='utf-8') as fh:
            fh.write(content)
        os.chmod(tmp, 0o600)
        os.replace(tmp, path)
    finally:
        tmp.unlink(missing_ok=True)


def _is_loopback_host(host: str | None) -> bool:
    if not host:
        return False
    if host.lower() in ('localhost', 'localhost.localdomain'):
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def validate_service_url(value: str) -> str:
    """Accept an empty value (direct mode) or a safe HTTP(S) API base URL."""
    if not isinstance(value, str) or len(value) > 512:
        raise ValueError('service_url must be a short string')
    value = value.strip()
    if not value:
        return ''
    if any(ch.isspace() for ch in value):
        raise ValueError('service_url must not contain whitespace')
    parts = urlsplit(value)
    if parts.scheme not in ('http', 'https') or not parts.netloc:
        raise ValueError('service_url must be an http(s) URL')
    if not parts.hostname:
        raise ValueError('service_url must include a host')
    if parts.username or parts.password:
        raise ValueError('service_url must not embed credentials')
    if parts.query or parts.fragment:
        raise ValueError('service_url must not contain a query or fragment')
    try:
        port = parts.port
    except ValueError as exc:
        raise ValueError('service_url has an invalid port') from exc
    if port is not None and not 1 <= port <= 65535:
        raise ValueError('service_url has an invalid port')
    if parts.scheme == 'http' and not _is_loopback_host(parts.hostname):
        raise ValueError('service_url must use HTTPS unless it points to loopback')
    path = parts.path.rstrip('/')
    if path and not re.fullmatch(r'/[A-Za-z0-9._~/-]*', path):
        raise ValueError('service_url path contains unsupported characters')
    host = parts.hostname or ''
    netloc = f'[{host}]' if ':' in host else host
    if port:
        netloc += f':{port}'
    return f'{parts.scheme}://{netloc}{path}'


def read_config() -> dict:
    secure_dirs()
    user = {}
    if CONFIG_PATH.exists():
        user = json.loads(CONFIG_PATH.read_text(encoding='utf-8'))
        if not isinstance(user, dict):
            raise ValueError('Settings must be a JSON object')
    result = {**DEFAULT, **user}
    if result['language'] not in LANGUAGES:
        result['language'] = 'auto'
    if result['mode'] not in MODES:
        result['mode'] = 'standard'
    if result['asr_model'] not in ASR_MODELS:
        result['asr_model'] = DEFAULT['asr_model']
    if result['output_script'] not in OUTPUT_SCRIPTS:
        result['output_script'] = ''
    result['service_url'] = validate_service_url(result.get('service_url', ''))
    return result


def save_config(partial: dict) -> dict:
    if not isinstance(partial, dict):
        raise ValueError('Settings update must be an object')
    unknown = set(partial) - set(DEFAULT)
    if unknown:
        raise ValueError('Unknown settings: ' + ', '.join(sorted(unknown)))
    new = {**read_config(), **partial}
    if new['language'] not in LANGUAGES:
        raise ValueError('Unsupported language')
    if new['mode'] not in MODES:
        raise ValueError('Unsupported mode')
    if new['asr_model'] not in ASR_MODELS:
        raise ValueError('Unsupported ASR model')
    if not isinstance(new['max_record_seconds'], int) or not 5 <= new['max_record_seconds'] <= 600:
        raise ValueError('max_record_seconds must be 5..600')
    if new['output_script'] not in OUTPUT_SCRIPTS:
        raise ValueError('output_script must be empty, hans or hant')
    for key in ('auto_insert', 'restore_text_clipboard', 'delete_audio_on_success',
                'dictation_commands', 'insert_without_focus_check'):
        if not isinstance(new[key], bool):
            raise ValueError(f'{key} must be boolean')
    for key in ('mic_node', 'asr_prompt', 'grammar_model', 'proxy_url', 'service_url'):
        if not isinstance(new[key], str) or len(new[key]) > 512:
            raise ValueError(f'{key} must be a short string')
    if new['proxy_url'] and not re.match(r'^https?://[^\s]+$', new['proxy_url']):
        raise ValueError('Proxy URL must be http(s)')
    new['service_url'] = validate_service_url(new['service_url'])
    _write_private(CONFIG_PATH, json.dumps(new, ensure_ascii=False, indent=2) + '\n')
    return new


def api_key() -> str:
    key = os.environ.get('GROQ_API_KEY', '').strip()
    if key:
        return key
    if KEY_PATH.exists():
        if KEY_PATH.stat().st_mode & 0o077:
            raise RuntimeError('Key permissions too broad; chmod 600 ~/.config/scribiva/groq-api-key')
        return KEY_PATH.read_text(encoding='utf-8').strip()
    return ''


def write_api_key(value: str):
    value = value.strip()
    if not value or len(value) > 512 or any(x in value for x in ('\n', '\r', '"')):
        raise ValueError('Invalid key format')
    _write_private(KEY_PATH, value + '\n')


def service_token() -> str:
    """Read the subscription API token from the environment or a 0600 file."""
    token = os.environ.get('SCRIBIVA_SERVICE_TOKEN', '').strip()
    source = 'SCRIBIVA_SERVICE_TOKEN'
    if not token and SERVICE_TOKEN_PATH.exists():
        if SERVICE_TOKEN_PATH.stat().st_mode & 0o077:
            raise RuntimeError('Service token permissions too broad; '
                               'chmod 600 ~/.config/scribiva/service-token')
        token = SERVICE_TOKEN_PATH.read_text(encoding='utf-8').strip()
        source = str(SERVICE_TOKEN_PATH)
    if token and not SERVICE_TOKEN_RE.fullmatch(token):
        raise RuntimeError(f'Service token from {source} has an invalid format')
    return token


def write_service_token(value: str):
    value = value.strip()
    if not SERVICE_TOKEN_RE.fullmatch(value):
        raise ValueError('Invalid service token format (expected scr_...)')
    _write_private(SERVICE_TOKEN_PATH, value + '\n')


def clear_service_token() -> bool:
    if SERVICE_TOKEN_PATH.exists():
        SERVICE_TOKEN_PATH.unlink(missing_ok=True)
        return True
    return False


def mic_node(config: dict, sources: dict | None = None) -> str:
    """Resolve the capture node: explicit setting, legacy file, then system default."""
    if config['mic_node'].strip():
        return config['mic_node'].strip()
    if MIC_PATH.exists():
        legacy = MIC_PATH.read_text(encoding='utf-8').strip()
        if legacy:
            return legacy
    if sources is None:
        from . import devices
        sources = devices.list_sources()
    node = sources.get('default') if isinstance(sources, dict) else None
    if isinstance(node, str) and node.strip():
        return node.strip()
    raise RuntimeError('No microphone configured: use scribiva devices, then '
                       'scribiva set mic_node <PipeWire node.name>')


def clear_mic_node() -> dict:
    """Remove the explicit choice and stale legacy node so the system default is used."""
    MIC_PATH.unlink(missing_ok=True)
    return save_config({'mic_node': ''})
