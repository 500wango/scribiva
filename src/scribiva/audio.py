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


def trim_silence(path: Path, *, frame_ms: int = 30, pad_ms: int = 300,
                 min_rms: float = 120.0) -> dict:
    """Trim leading/trailing silence in place (FR-017: less input for phantom text).

    Energy-based and conservative: the threshold adapts to the quietest part of
    the recording, and ``pad_ms`` of context is kept around detected speech.
    Returns {'trimmed': bool, 'duration': seconds after trimming}.
    """
    with wave.open(str(path), 'rb') as wav:
        params = wav.getparams()
        frames = wav.readframes(wav.getnframes())
    if params.sampwidth != 2 or params.nchannels != 1 or params.framerate <= 0:
        return {'trimmed': False, 'duration': params.nframes / max(1, params.framerate)}
    samples = array.array('h')
    samples.frombytes(frames)
    step = max(1, params.framerate * frame_ms // 1000)
    levels = []
    for start in range(0, len(samples), step):
        chunk = samples[start:start + step]
        levels.append(math.sqrt(sum(float(x) * x for x in chunk) / len(chunk)) if chunk else 0.0)
    if not levels:
        return {'trimmed': False, 'duration': 0.0}
    floor = sorted(levels)[len(levels) // 10]
    threshold = max(min_rms, floor * 3)
    voiced = [i for i, level in enumerate(levels) if level >= threshold]
    if not voiced:
        return {'trimmed': False, 'duration': len(samples) / params.framerate}
    pad = max(1, pad_ms // frame_ms)
    first = max(0, voiced[0] - pad) * step
    last = min(len(samples), (voiced[-1] + 1 + pad) * step)
    if first == 0 and last >= len(samples):
        return {'trimmed': False, 'duration': len(samples) / params.framerate}
    kept = samples[first:last]
    tmp = path.with_name(path.name + '.trim')
    with wave.open(str(tmp), 'wb') as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(params.framerate)
        out.writeframes(kept.tobytes())
    os.replace(tmp, path)
    return {'trimmed': True, 'duration': len(kept) / params.framerate}


def encode_flac(path: Path) -> tuple[Path, str] | None:
    """Losslessly compress for upload (smaller cross-border uploads, PRD §11.2).

    Returns (flac_path, mime) or None when ffmpeg is unavailable or fails, in
    which case the caller uploads the WAV unchanged.
    """
    import shutil
    exe = shutil.which('ffmpeg')
    if not exe:
        return None
    out = path.with_suffix('.flac')
    try:
        proc = subprocess.run(
            [exe, '-nostdin', '-hide_banner', '-loglevel', 'error', '-y', '-i', str(path),
             '-c:a', 'flac', '-compression_level', '5', str(out)],
            stdin=subprocess.DEVNULL, capture_output=True, timeout=30, check=False)
    except (OSError, subprocess.TimeoutExpired):
        out.unlink(missing_ok=True)
        return None
    if proc.returncode != 0 or not out.is_file() or out.stat().st_size < 64:
        out.unlink(missing_ok=True)
        return None
    return out, 'audio/flac'
