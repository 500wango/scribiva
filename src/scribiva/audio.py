"""PipeWire recording and low-cost WAV validation."""
from __future__ import annotations
import array
import math
import os
import secrets
import signal
import subprocess
import time
import wave
from pathlib import Path
from .config import AUDIO_DIR


def new_path() -> Path:
    return AUDIO_DIR / ('record-' + time.strftime('%Y%m%d-%H%M%S-') + secrets.token_hex(6) + '.wav')


def start(node: str) -> tuple[int, Path]:
    path = new_path()
    log_path = AUDIO_DIR / 'recorder-stderr.log'
    with log_path.open('ab') as err:
        proc = subprocess.Popen(
            ['pw-record', '--target=' + node, '--rate', '16000', '--channels', '1', str(path)],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=err,
            start_new_session=True, close_fds=True,
        )
    time.sleep(0.25)
    if proc.poll() is not None:
        raise RuntimeError('pw-record exited immediately; check ' + str(log_path))
    return proc.pid, path


def is_ours(pid: int, path: Path) -> bool:
    try:
        cmd = Path('/proc') / str(pid) / 'cmdline'
        data = cmd.read_bytes()
        parts = data.split(b'\x00')
        return any(b'pw-record' in part for part in parts[:2]) and os.fsencode(path) in parts
    except (OSError, ValueError, IndexError):
        return False


def stop(pid: int, path: Path) -> None:
    if not is_ours(pid, path):
        raise RuntimeError('Recording process is no longer available (refusing to kill unrelated process)')
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    # pw-record is not a child of the second hotkey invocation: poll /proc.
    for _ in range(45):
        if not is_ours(pid, path):
            break
        time.sleep(0.1)
    else:
        # A recorder that ignores SIGTERM must not keep capturing indefinitely.
        # Re-verify the cmdline before escalating so a recycled PID is never hit.
        if is_ours(pid, path):
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            for _ in range(20):
                if not is_ours(pid, path):
                    break
                time.sleep(0.05)
            else:
                raise RuntimeError('Recorder did not terminate after SIGTERM/SIGKILL')
    time.sleep(0.25)
    if not path.is_file() or path.stat().st_size < 4096:
        raise RuntimeError('Audio file is missing or too small')


def quality(path: Path) -> dict:
    with wave.open(str(path), 'rb') as wav:
        duration = wav.getnframes() / wav.getframerate()
        channels = wav.getnchannels()
        rate = wav.getframerate()
        sample_width = wav.getsampwidth()
        raw = wav.readframes(min(wav.getnframes(), rate * 20))
    rms = None
    if sample_width == 2:
        data = array.array('h')
        data.frombytes(raw)
        if data:
            rms = round(math.sqrt(sum(float(x)*x for x in data)/len(data)), 2)
    return {'duration': round(duration, 2), 'rate': rate, 'channels': channels,
            'sample_width': sample_width, 'rms': rms}
