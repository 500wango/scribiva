"""Deterministic transcript post-processing that must not depend on the LLM.

- Spoken formatting commands (FR-016) become newlines and Markdown list items.
- Whisper hallucinations on silence/noise are flagged (FR-017).
- Chinese output script is normalised (FR-003) when OpenCC is installed.
- Personal dictionary terms (FR-018) are validated and turned into an ASR prompt.


Shared copy: the server package keeps an identical module so it can be
deployed on its own. tests/test_shared_modules.py keeps the two in sync."""
from __future__ import annotations
import re
import unicodedata

# --------------------------------------------------------------- commands

# Longest phrases first so "新段落" wins over a shorter overlapping phrase.
_ZH_COMMANDS = (('新段落', '\n\n'), ('另起一段', '\n\n'), ('换行', '\n'),
                ('列表项', '\n- '))
_EN_COMMANDS = (('new paragraph', '\n\n'), ('new line', '\n'), ('newline', '\n'),
                ('bullet point', '\n- '), ('bullet', '\n- '))
# Whisper often wraps a spoken command with punctuation; absorb it.
_PUNCT = r'[\s,，。.、;；:：!！?？]*'
_ZH_RE = re.compile(_PUNCT + '(' + '|'.join(re.escape(p) for p, _ in _ZH_COMMANDS) + ')' + _PUNCT)
_EN_RE = re.compile(r'[\s,.;:!?]*\b(' + '|'.join(re.escape(p) for p, _ in _EN_COMMANDS)
                    + r')\b[\s,.;:!?]*', re.I)
_ZH_MAP = dict(_ZH_COMMANDS)
_EN_MAP = {p: v for p, v in _EN_COMMANDS}


def apply_commands(text: str) -> str:
    """Replace spoken formatting commands with structure characters."""
    out = _ZH_RE.sub(lambda m: _ZH_MAP[m.group(1)], text)
    out = _EN_RE.sub(lambda m: _EN_MAP[m.group(1).lower()], out)
    # A list item command at the very start should not leave a leading newline.
    out = re.sub(r'^\n(- )', r'\1', out)
    out = re.sub(r'[ \t]+\n', '\n', out)
    out = re.sub(r'\n{3,}', '\n\n', out)
    return out.strip(' \t')


# --------------------------------------------------------- hallucinations

# Phrases Whisper is known to emit on silence, music or noise. Compared after
# normalisation; only an exact (whole-transcript) match is treated as phantom.
_KNOWN_PHANTOMS = {
    '谢谢观看', '谢谢大家', '感谢观看', '请不吝点赞订阅转发打赏支持明镜与点点栏目',
    '字幕由amaraorg社区提供', '中文字幕志愿者', '字幕志愿者', '订阅我的频道',
    'thanksforwatching', 'thankyouforwatching', 'thankyou', 'thanks',
    'pleasesubscribe', 'subscribetomychannel', 'bye', 'you',
    'ご視聴ありがとうございました', 'おやすみなさい',
    '시청해주셔서감사합니다', 'merci', 'untertitelderamaraorgcommunity',
    'субтитрысделалdimatorzok', 'продолжениеследует',
}
NO_SPEECH_THRESHOLD = 0.6
LOGPROB_THRESHOLD = -1.0


def _normalise(text: str) -> str:
    text = unicodedata.normalize('NFKC', text).lower()
    return ''.join(ch for ch in text if ch.isalnum())


def is_suspected_hallucination(text: str, segments: list | None = None) -> bool:
    """True when the transcript is probably phantom text, not speech.

    Segment statistics come from Whisper ``verbose_json``. A segment counts as
    silence when it is both likely non-speech and low confidence, matching the
    reference Whisper decoder's own silence rule.
    """
    norm = _normalise(text)
    if not norm:
        return True
    if norm in _KNOWN_PHANTOMS:
        return True
    if segments:
        silent = 0
        for seg in segments:
            if not isinstance(seg, dict):
                continue
            no_speech = seg.get('no_speech_prob')
            logprob = seg.get('avg_logprob')
            if (isinstance(no_speech, (int, float)) and no_speech > NO_SPEECH_THRESHOLD
                    and isinstance(logprob, (int, float)) and logprob < LOGPROB_THRESHOLD):
                silent += 1
        if silent and silent == len([s for s in segments if isinstance(s, dict)]):
            return True
    return False


# ----------------------------------------------------------------- script

SCRIPTS = {'', 'hans', 'hant'}
_CONVERTERS: dict[str, object] = {}


def _converter(script: str):
    if script in _CONVERTERS:
        return _CONVERTERS[script]
    conv = None
    try:  # Optional dependency; pinned in server/requirements.txt.
        import opencc  # type: ignore
        conv = opencc.OpenCC('t2s.json' if script == 'hans' else 's2t.json')
    except Exception:
        conv = None
    _CONVERTERS[script] = conv
    return conv


def convert_script(text: str, script: str) -> tuple[str, bool]:
    """Return (text, converted). ``converted`` is False when OpenCC is absent."""
    if script not in ('hans', 'hant'):
        return text, True
    conv = _converter(script)
    if conv is None:
        return text, False
    return conv.convert(text), True


def looks_chinese(text: str) -> bool:
    han = sum(1 for ch in text if '一' <= ch <= '鿿')
    return han >= max(1, len(text) // 5)


# ------------------------------------------------------------- dictionary

MAX_TERMS = 200
MAX_TERM_LENGTH = 64
PROMPT_LIMIT = 600  # characters; Whisper honours roughly 224 tokens.


class DictionaryError(ValueError):
    pass


def clean_terms(terms) -> list[str]:
    if not isinstance(terms, list):
        raise DictionaryError('terms must be a list of strings')
    seen: set[str] = set()
    result: list[str] = []
    for term in terms:
        if not isinstance(term, str):
            raise DictionaryError('terms must be strings')
        value = ' '.join(term.split())
        if not value:
            continue
        if len(value) > MAX_TERM_LENGTH:
            raise DictionaryError(f'each term must be at most {MAX_TERM_LENGTH} characters')
        if any(unicodedata.category(ch).startswith('C') for ch in value):
            raise DictionaryError('terms must not contain control characters')
        key = value.casefold()
        if key in seen:
            continue
        seen.add(key)
        result.append(value)
    if len(result) > MAX_TERMS:
        raise DictionaryError(f'at most {MAX_TERMS} terms are allowed')
    return result


def asr_prompt(terms: list[str]) -> str:
    """Comma-separated vocabulary hint, truncated to the provider limit."""
    out = ''
    for term in terms:
        piece = term if not out else ', ' + term
        if len(out) + len(piece) > PROMPT_LIMIT:
            break
        out += piece
    return out
