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


def protect(text: str) -> tuple[str, dict[str, str]]:
    parts: dict[str, str] = {}

    def hide(match: re.Match) -> str:
        token = f'SCRIVPROTECT{len(parts):04d}END'
        parts[token] = match.group(0)
        return token

    return PROTECTED_RE.sub(hide, text), parts


def restore(original: str, revised: str, parts: dict[str, str]) -> str:
    if len(revised) > max(100, 3 * len(original)):
        raise GuardError('Expansion exceeds allowed limit')
    for token, value in parts.items():
        if revised.count(token) != 1:
            raise GuardError('Protected token changed')
        revised = revised.replace(token, value)
    if NUMBERS_RE.findall(original) != NUMBERS_RE.findall(revised):
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
