"""Server configuration from environment variables.

No third-party dependency. Secrets are read from the environment only and are
never written to the SQLite database or logs.
"""
from __future__ import annotations
import os
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_DB = Path(__file__).resolve().parents[1] / 'data' / 'scribiva.sqlite3'


class ConfigError(RuntimeError):
    pass


def _int(name: str, default: int, minimum: int = 0, maximum: int | None = None) -> int:
    raw = os.environ.get(name, '').strip()
    if not raw:
        return default
    try:
        value = int(raw)
    except ValueError as exc:
        raise ConfigError(f'{name} must be an integer') from exc
    if value < minimum:
        raise ConfigError(f'{name} must be >= {minimum}')
    if maximum is not None and value > maximum:
        raise ConfigError(f'{name} must be <= {maximum}')
    return value


def _bool(name: str, default: bool = False) -> bool:
    raw = os.environ.get(name, '').strip().lower()
    if not raw:
        return default
    if raw in ('1', 'true', 'yes', 'on'):
        return True
    if raw in ('0', 'false', 'no', 'off'):
        return False
    raise ConfigError(f'{name} must be true or false')


@dataclass(frozen=True)
class Settings:
    db_path: Path = DEFAULT_DB
    host: str = '127.0.0.1'
    port: int = 8787
    groq_api_key: str = ''
    groq_base_url: str = 'https://api.groq.com/openai/v1'
    asr_model: str = 'whisper-large-v3'
    grammar_model: str = 'openai/gpt-oss-20b'
    free_seconds: int = 1800
    pro_seconds: int = 36000
    max_audio_bytes: int = 25_000_000
    max_audio_seconds: int = 180
    result_ttl_seconds: int = 900
    job_timeout_seconds: int = 120
    rate_limit_per_minute: int = 30
    token_ttl_days: int = 30
    stripe_secret_key: str = ''
    stripe_webhook_secret: str = ''
    stripe_price_monthly: str = ''
    stripe_price_annual: str = ''
    public_base_url: str = ''
    allow_remote: bool = False
    max_request_bytes: int = field(init=False)

    def __post_init__(self):
        object.__setattr__(self, 'max_request_bytes', self.max_audio_bytes + 1_048_576)

    @classmethod
    def from_env(cls) -> 'Settings':
        db_raw = os.environ.get('SCRIBIVA_DB', '').strip()
        settings = cls(
            db_path=Path(db_raw).expanduser() if db_raw else DEFAULT_DB,
            host=os.environ.get('SCRIBIVA_HOST', '127.0.0.1').strip() or '127.0.0.1',
            port=_int('SCRIBIVA_PORT', 8787, 1, 65535),
            groq_api_key=os.environ.get('GROQ_API_KEY', '').strip(),
            groq_base_url=(os.environ.get('GROQ_BASE_URL', 'https://api.groq.com/openai/v1').strip()
                           or 'https://api.groq.com/openai/v1').rstrip('/'),
            asr_model=os.environ.get('SCRIBIVA_ASR_MODEL', 'whisper-large-v3').strip(),
            grammar_model=os.environ.get('SCRIBIVA_GRAMMAR_MODEL', 'openai/gpt-oss-20b').strip(),
            free_seconds=_int('SCRIBIVA_FREE_SECONDS', 1800, 0),
            pro_seconds=_int('SCRIBIVA_PRO_SECONDS', 36000, 0),
            max_audio_bytes=_int('SCRIBIVA_MAX_AUDIO_BYTES', 25_000_000, 4096),
            max_audio_seconds=_int('SCRIBIVA_MAX_AUDIO_SECONDS', 180, 1, 3600),
            result_ttl_seconds=_int('SCRIBIVA_RESULT_TTL_SECONDS', 900, 0),
            job_timeout_seconds=_int('SCRIBIVA_JOB_TIMEOUT_SECONDS', 120, 10),
            rate_limit_per_minute=_int('SCRIBIVA_RATE_LIMIT', 30, 1),
            token_ttl_days=_int('SCRIBIVA_TOKEN_TTL_DAYS', 30, 1),
            stripe_secret_key=os.environ.get('STRIPE_SECRET_KEY', '').strip(),
            stripe_webhook_secret=os.environ.get('STRIPE_WEBHOOK_SECRET', '').strip(),
            stripe_price_monthly=os.environ.get('STRIPE_PRICE_ID_MONTHLY', '').strip(),
            stripe_price_annual=os.environ.get('STRIPE_PRICE_ID_ANNUAL', '').strip(),
            public_base_url=os.environ.get('SCRIBIVA_PUBLIC_BASE_URL', '').strip().rstrip('/'),
            allow_remote=_bool('SCRIBIVA_ALLOW_REMOTE', False),
        )
        if settings.port < 1 or settings.port > 65535:
            raise ConfigError('SCRIBIVA_PORT is out of range')
        if settings.db_path.suffix == '':
            raise ConfigError('SCRIBIVA_DB must point to a file')
        return settings

    @property
    def billing_configured(self) -> bool:
        return bool(self.stripe_secret_key and self.public_base_url)

    @property
    def webhook_configured(self) -> bool:
        return bool(self.stripe_webhook_secret)

    def limit_for(self, plan: str) -> int:
        return self.pro_seconds if plan == 'pro' else self.free_seconds
