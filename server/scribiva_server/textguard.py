"""Deterministic guards around the optional grammar edit.

The transcript is untrusted input, not an instruction. URLs, code, paths,
emails, numbers and Markdown markers are masked before the model sees the text
and verified afterwards; any mismatch fails closed to the raw transcript.
"""
from __future__ import annotations
import re

PROTECTED_RE = re.compile(
    r'```[\s\S]*?```'
    r'|`[^`\n]+`'
    r'|https?://[^\s)>\]]+'
    r'|(?<![\w.+-])[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}'
    r'|(?<!\w)(?:~/|\.{0,2}/)[A-Za-z0-9_./-]+'
)
NUMBERS_RE = re.compile(r'(?<!\w)\d+(?:[.,:/-]\d+)*(?!\w)')
MARKER_RE = re.compile(r'^\s{0,3}(?:#{1,6}\s|[-*+]\s|\d+[.)]\s|>\s?)', re.M)
MAX_TEXT = 100_000


class GuardError(ValueError):
    pass


def protect(text: str, terms: list[str] | None = None) -> tuple[str, dict[str, str]]:
    """Mask literals and personal dictionary terms (FR-018) before the LLM."""
    parts: dict[str, str] = {}

    def hide(match: re.Match) -> str:
        token = f'SCRIVPROTECT{len(parts):04d}END'
        parts[token] = match.group(0)
        return token

    masked = PROTECTED_RE.sub(hide, text)
    if terms:
        # Longest first so "GitHub Actions" is masked before "GitHub".
        ordered = sorted({t for t in terms if t}, key=len, reverse=True)
        pattern = re.compile('|'.join(
            (r'(?<!\w)' + re.escape(t) + r'(?!\w)') if t[0].isalnum() and t[-1].isalnum()
            and t.isascii() else re.escape(t) for t in ordered))
        masked = pattern.sub(hide, masked)
    return masked, parts


def _number_values(text: str) -> list[str]:
    """Numbers as values: thousands separators do not change the value."""
    values = []
    for raw in NUMBERS_RE.findall(text):
        if re.fullmatch(r'\d{1,3}(?:,\d{3})+(?:\.\d+)?', raw):
            raw = raw.replace(',', '')
        values.append(raw)
    return values


# Spelled-out numbers the editor may legitimately rewrite as digits
# ("三点半" -> "3:30", "twenty five" -> "25"). One run allows one new number.
_ZH_NUMERAL_RUN = re.compile(r'[零〇一二两三四五六七八九十百千万亿半]+')
_EN_NUMERAL_RUN = re.compile(
    r'\b(?:zero|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|'
    r'thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|'
    r'forty|fifty|sixty|seventy|eighty|ninety|hundred|thousand|million|half)'
    r'(?:[\s-]+(?:and[\s-]+)?(?:one|two|three|four|five|six|seven|eight|nine|ten|'
    r'twenty|thirty|forty|fifty|sixty|seventy|eighty|ninety|hundred|thousand|million|'
    r'half))*\b', re.I)


def _numbers_preserved(original: str, revised: str) -> bool:
    """Every digit number survives in order; new digits need a spelled-out source."""
    before, after = _number_values(original), _number_values(revised)
    it = iter(after)
    if not all(any(value == other for other in it) for value in before):
        return False
    spelled = len(_ZH_NUMERAL_RUN.findall(original)) + len(_EN_NUMERAL_RUN.findall(original))
    return len(after) - len(before) <= spelled


def restore(original: str, revised: str, parts: dict[str, str]) -> str:
    if len(revised) > max(100, 3 * len(original)):
        raise GuardError('Expansion exceeds allowed limit')
    for token, value in parts.items():
        if revised.count(token) != 1:
            raise GuardError('Protected token changed')
        revised = revised.replace(token, value)
    if not _numbers_preserved(original, revised):
        raise GuardError('Numeric content changed')
    if MARKER_RE.findall(original) != MARKER_RE.findall(revised):
        raise GuardError('Markdown structure changed')
    return revised


def ensure_text(text: str) -> str:
    if not isinstance(text, str) or not text.strip():
        raise GuardError('Empty text')
    if len(text) > MAX_TEXT:
        raise GuardError('Text is too long')
    return text.strip()
