"""Validate uploaded audio and derive duration server-side.

Client-declared duration is never trusted. WAV/RIFF is decoded with the standard
library; FLAC/WebM use ``ffprobe`` when available, otherwise they are rejected
instead of being billed with a guessed duration.
"""
from __future__ import annotations
import io
import math
import shutil
import subprocess
import tempfile
import wave
from pathlib import Path


class AudioError(RuntimeError):
    def __init__(self, code: str, message: str, status: int = 400):
        super().__init__(message)
        self.code = code
        self.message = message
        self.status = status


def _probe_with_ffprobe(data: bytes, suffix: str) -> float:
    exe = shutil.which('ffprobe')
    if not exe:
        raise AudioError('UNSUPPORTED_AUDIO',
                         'This audio format needs ffprobe for server-side validation', 422)
    with tempfile.TemporaryDirectory(prefix='scribiva-audio-') as folder:
        path = Path(folder) / ('upload' + suffix)
        path.write_bytes(data)
        try:
            proc = subprocess.run(
                [exe, '-v', 'error', '-show_entries', 'format=duration',
                 '-of', 'default=noprint_wrappers=1:nokey=1', str(path)],
                capture_output=True, text=True, timeout=10, check=False,
            )
        except (subprocess.TimeoutExpired, OSError) as exc:
            raise AudioError('AUDIO_PROBE_FAILED',
                             'Audio duration could not be determined', 422) from exc
    if proc.returncode != 0:
        raise AudioError('INVALID_AUDIO', 'Audio could not be decoded', 400)
    try:
        duration = float((proc.stdout or '').strip())
    except ValueError as exc:
        raise AudioError('AUDIO_DURATION_INVALID', 'Audio duration is unavailable', 422) from exc
    if duration <= 0:
        raise AudioError('AUDIO_DURATION_INVALID', 'Audio duration is invalid', 422)
    return duration


def probe(data: bytes, *, max_bytes: int, max_seconds: int) -> dict:
    if not data:
        raise AudioError('INVALID_AUDIO', 'Audio payload is empty', 400)
    if len(data) > max_bytes:
        raise AudioError('AUDIO_TOO_LARGE', 'Audio exceeds the upload cap', 413)

    if data[:4] == b'RIFF' and data[8:12] == b'WAVE':
        try:
            with wave.open(io.BytesIO(data), 'rb') as wav:
                frames = wav.getnframes()
                rate = wav.getframerate()
                channels = wav.getnchannels()
                if rate <= 0 or channels <= 0 or frames <= 0:
                    raise AudioError('INVALID_AUDIO', 'WAV stream has no usable frames', 400)
                duration = frames / float(rate)
        except (wave.Error, EOFError, OSError) as exc:
            raise AudioError('INVALID_AUDIO', 'WAV payload is malformed', 400) from exc
        codec = 'wav'
    elif data[:4] == b'fLaC':
        duration = _probe_with_ffprobe(data, '.flac')
        codec = 'flac'
    elif data[:4] == b'\x1aE\xdf\xa3':
        duration = _probe_with_ffprobe(data, '.webm')
        codec = 'webm'
    else:
        raise AudioError('INVALID_AUDIO', 'Unsupported audio container', 400)

    if duration > max_seconds:
        raise AudioError('AUDIO_DURATION_INVALID',
                         f'Audio exceeds the {max_seconds}s limit', 422)
    if duration < 0.2:
        raise AudioError('AUDIO_DURATION_INVALID', 'Audio is shorter than 0.2s', 422)
    return {
        'duration_ms': int(round(duration * 1000)),
        'duration_seconds': duration,
        'billable_seconds': max(1, math.ceil(duration)),
        'codec': codec,
        'bytes': len(data),
    }
