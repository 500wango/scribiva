"""Stateful, one-hotkey-toggle dictation engine for Linux developer alpha."""
from __future__ import annotations
import fcntl
import json
import logging
import os
import secrets
import shutil
import subprocess
import sys
import time
from contextlib import contextmanager
from pathlib import Path
from . import __version__, audio, devices, inject, providers
from .config import (AUDIO_DIR, DEFAULT, KEY_PATH, LAST_RAW_PATH, LATEST_PATH,
                     LOCK_PATH, LOG_PATH, MIC_PATH, ROOT_STATE, SERVICE_TOKEN_PATH,
                     STATE_PATH, _write_private, api_key, mic_node, read_config,
                     secure_dirs, service_token)

logger = logging.getLogger('scribiva')


def init_logging():
    secure_dirs()
    if logger.handlers:
        return
    logger.setLevel(logging.INFO)
    handler = logging.FileHandler(LOG_PATH, encoding='utf-8')
    handler.setFormatter(logging.Formatter('%(asctime)s %(levelname)s %(message)s'))
    logger.addHandler(handler)
    LOG_PATH.chmod(0o600)


def notify(title: str, detail: str = ''):
    if shutil.which('notify-send'):
        try:
            subprocess.run(['notify-send', '-t', '2500', 'Scribiva: ' + title, detail],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           timeout=3, check=False)
        except (OSError, subprocess.TimeoutExpired):
            pass


@contextmanager
def locked():
    secure_dirs()
    with LOCK_PATH.open('a+') as fh:
        os.chmod(LOCK_PATH, 0o600)
        fcntl.flock(fh, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(fh, fcntl.LOCK_UN)


def state() -> dict:
    if not STATE_PATH.is_file():
        return {'phase': 'idle'}
    try:
        data = json.loads(STATE_PATH.read_text(encoding='utf-8'))
        if not isinstance(data, dict):
            return {'phase': 'error', 'error': 'Invalid session state'}
        return data
    except (OSError, ValueError):
        return {'phase': 'error', 'error': 'Unreadable session state'}


def set_state(data: dict):
    data = {**data, 'updated': round(time.time(), 3)}
    _write_private(STATE_PATH, json.dumps(data, ensure_ascii=False))


def _belongs_here(record: dict) -> bool:
    try:
        path = Path(record['file']).resolve()
        return path.is_relative_to(AUDIO_DIR.resolve())
    except (KeyError, ValueError, OSError):
        return False


def _session_file(record: dict) -> Path | None:
    try:
        return Path(record['file'])
    except (KeyError, TypeError):
        return None


def launch_aux(cmd: str, job_id: str):
    with LOG_PATH.open('a') as output:
        subprocess.Popen([sys.executable, '-m', 'scribiva', cmd, job_id],
                         start_new_session=True, close_fds=True, stdin=subprocess.DEVNULL,
                         stdout=output, stderr=output)


def toggle() -> str:
    init_logging()
    with locked():
        current = state()
        phase = current.get('phase', 'idle')
        if phase in ('processing', 'injecting'):
            notify('Processing', 'Please do not press the shortcut again yet')
            return 'processing'
        if phase == 'recording':
            if not _belongs_here(current):
                set_state({'phase': 'error', 'error': 'Session audio path invalid'})
                raise RuntimeError('Session audio path invalid')
            path = Path(current['file'])
            job_id = current['id']
            try:
                audio.stop(int(current['pid']), path)
            except (RuntimeError, OSError, ValueError) as exc:
                # Keep the audio path in error state so retry/discard can act on it.
                message = 'Stopping the recorder failed: ' + str(exc)
                set_state({'phase': 'error', 'id': job_id, 'error': message, 'file': str(path)})
                logger.warning('recording_stop_failed job=%s reason=%s', job_id, str(exc)[:180])
                notify('Stop failed', str(exc)[:100])
                raise RuntimeError(message) from exc
            duration = time.time() - float(current['started'])
            if duration < 1.0:
                path.unlink(missing_ok=True)
                set_state({'phase': 'idle'})
                notify('Too short', 'Speak for at least one second')
                return 'too-short'
            set_state({'phase': 'processing', 'id': job_id, 'file': str(path), 'started': current['started']})
            launch_aux('worker', job_id)
            logger.info('recording_stopped job=%s duration=%.2f', job_id, duration)
            notify('Transcribing', 'Recording complete')
            return 'processing'
        config = read_config()
        mic = mic_node(config)
        pid, path = audio.start(mic)
        job_id = secrets.token_hex(8)
        set_state({'phase': 'recording', 'id': job_id, 'pid': pid,
                   'file': str(path), 'started': time.time()})
        launch_aux('watchdog', job_id)
        logger.info('recording_started job=%s pid=%d', job_id, pid)
        notify('Recording', 'Press Ctrl+Alt+V again to stop')
        return 'recording'


def watchdog(job_id: str):
    try:
        seconds = read_config()['max_record_seconds']
        time.sleep(seconds)
        with locked():
            current = state()
            if current.get('phase') == 'recording' and current.get('id') == job_id and _belongs_here(current):
                try:
                    audio.stop(int(current['pid']), Path(current['file']))
                    message = 'Maximum recording duration reached; audio retained'
                except (RuntimeError, OSError, ValueError) as exc:
                    message = 'Maximum duration reached; recorder stop failed: ' + str(exc)
                set_state({'phase': 'error', 'error': message,
                           'file': current['file'], 'id': job_id})
                logger.warning('recording_timeout job=%s reason=%s', job_id, message[:180])
                notify('Recording stopped', message[:100])
    except Exception:
        logger.exception('watchdog_error')


def _recover_error(job_id: str, message: str, file: str | None):
    with locked():
        current = state()
        if current.get('id') == job_id:
            recovered = {'phase': 'error', 'id': job_id, 'error': message, 'file': file}
            if current.get('last_warning'):
                recovered['last_warning'] = current['last_warning']
            set_state(recovered)
    logger.warning('job_failed id=%s reason=%s', job_id, message[:180])
    notify('Error', message[:100])


def worker(job_id: str):
    init_logging()
    current = state()
    if current.get('id') != job_id or current.get('phase') != 'processing' or not _belongs_here(current):
        return
    file = Path(current['file'])
    config = read_config()
    try:
        quality = audio.quality(file)
        if quality['duration'] < 1.0:
            raise RuntimeError('Recording too short')
        if quality['rms'] is not None and quality['rms'] < 50:
            raise RuntimeError('No meaningful audio signal detected')
        if quality['duration'] > config['max_record_seconds'] + 5:
            raise RuntimeError('Recording too long')
        if providers.service_configured(config):
            result = providers.service_dictation(file, config, job_id=job_id)
            raw = result['raw_text']
            edited = result['output_text']
            correction_warning = result.get('warning') or None
            billing = result.get('billing') or {}
            logger.info('service_billing id=%s job=%s charged=%s remaining=%s',
                        job_id, result.get('job_id'),
                        billing.get('charged_audio_seconds'),
                        billing.get('remaining_seconds'))
        else:
            raw = providers.transcribe(file, config)
            # Do not pay for a grammar call if the user cancelled during ASR.
            if state().get('id') != job_id or state().get('phase') != 'processing':
                logger.info('worker_cancelled_after_asr id=%s', job_id)
                notify('Cancelled', 'Result was not applied')
                return
            edited, correction_warning = providers.correct(raw, config)
        # Only the latest uncancelled job may replace the recoverable result.
        with locked():
            fresh = state()
            if fresh.get('id') != job_id or fresh.get('phase') != 'processing':
                logger.info('worker_cancelled id=%s', job_id)
                notify('Cancelled', 'Result was not applied')
                return
            _write_private(LATEST_PATH, edited)
            _write_private(LAST_RAW_PATH, raw)
            set_state({'phase': 'injecting', 'id': job_id, 'file': str(file),
                       'last_warning': correction_warning or ''})
        if correction_warning:
            logger.warning('grammar_fallback job=%s reason=%s', job_id, correction_warning)
        if config['auto_insert']:
            result = inject.dispatch(edited, config['restore_text_clipboard'])
        else:
            inject.copy_text(edited)
            result = 'clipboard_only'
        with locked():
            fresh = state()
            if fresh.get('id') == job_id and fresh.get('phase') == 'injecting':
                set_state({'phase': 'idle', 'last_result': result, 'last_job': job_id,
                           'last_warning': correction_warning or ''})
        if config['delete_audio_on_success']:
            file.unlink(missing_ok=True)
        logger.info('job_finished id=%s output=%s characters=%d', job_id, result, len(edited))
        notify('Complete', 'Paste dispatched (not verified)' if config['auto_insert'] else 'Text copied')
    except Exception as e:
        _recover_error(job_id, str(e), str(file))


def cancel():
    init_logging()
    with locked():
        current = state()
        phase = current.get('phase', 'idle')
        file = _session_file(current)
        retained = phase == 'error' and file is not None and _belongs_here(current) and file.exists()
        if retained:
            raise RuntimeError('Recording is retained for retry; use `scribiva discard` to delete it')
        if phase == 'recording' and _belongs_here(current):
            try:
                audio.stop(int(current['pid']), Path(current['file']))
            except (RuntimeError, OSError, ValueError):
                pass
            if file is not None:
                file.unlink(missing_ok=True)
        if phase != 'idle':
            set_state({'phase': 'idle', 'last_job': current.get('id', '')})
            logger.info('job_cancelled id=%s phase=%s', current.get('id', ''), phase)
    notify('Cancelled')


def discard() -> str:
    """Delete retained failed audio or drop a pending result without inserting it."""
    init_logging()
    with locked():
        current = state()
        phase = current.get('phase', 'idle')
        file = _session_file(current)
        if phase == 'recording' and _belongs_here(current):
            try:
                audio.stop(int(current['pid']), Path(current['file']))
            except (RuntimeError, OSError, ValueError) as exc:
                logger.warning('discard_stop_failed reason=%s', str(exc)[:160])
        if phase in ('processing', 'injecting'):
            set_state({'phase': 'idle', 'last_job': current.get('id', '')})
            logger.info('job_discarded id=%s phase=%s', current.get('id', ''), phase)
            notify('Discarded', 'Pending result will not be inserted')
            return 'cancelled'
        if file is not None and _belongs_here(current):
            file.unlink(missing_ok=True)
        set_state({'phase': 'idle', 'last_job': current.get('id', '')})
        logger.info('job_discarded id=%s phase=%s', current.get('id', ''), phase)
    notify('Discarded')
    return 'discarded'


def retry():
    init_logging()
    with locked():
        current = state()
        if current.get('phase') != 'error' or not _belongs_here(current):
            raise RuntimeError('No recoverable recording')
        file = Path(current['file'])
        if not file.exists():
            raise RuntimeError('Recording file no longer exists')
        # New id prevents late results from an older worker being used.
        job = secrets.token_hex(8)
        set_state({'phase': 'processing', 'id': job, 'file': str(file)})
        launch_aux('worker', job)
    notify('Retrying', 'Processing saved recording')


def _read_limited(path: Path, limit: int = 200_000) -> str:
    try:
        if not path.is_file():
            return ''
        return path.read_text(encoding='utf-8', errors='replace')[:limit]
    except OSError:
        return ''


def last_result() -> dict:
    """Return the last recoverable transcript without exposing audio data."""
    current = state()
    text = _read_limited(LATEST_PATH)
    raw = _read_limited(LAST_RAW_PATH)
    return {
        'available': bool(text or raw),
        'text': text,
        'raw': raw,
        'warning': current.get('last_warning', '') or '',
        'phase': current.get('phase', 'idle'),
        'last_job': current.get('last_job', '') or '',
    }


def clear_last() -> bool:
    """Remove locally stored transcripts (privacy control)."""
    init_logging()
    removed = False
    with locked():
        for path in (LATEST_PATH, LAST_RAW_PATH):
            if path.exists():
                path.unlink(missing_ok=True)
                removed = True
        current = state()
        if any(key in current for key in ('last_result', 'last_warning')):
            current.pop('last_result', None)
            current.pop('last_warning', None)
            set_state(current)
        logger.info('last_result_cleared files_removed=%s', removed)
    return removed


def _check(checks: list, name: str, status: str, detail: str):
    checks.append({'id': name, 'status': status, 'detail': detail[:240]})


def doctor() -> dict:
    checks: list[dict] = []
    try:
        conf = read_config()
        _check(checks, 'settings', 'ok', 'Settings file loaded')
    except (OSError, ValueError) as exc:
        conf = dict(DEFAULT)
        _check(checks, 'settings', 'fail', f'Cannot read settings: {exc}')

    try:
        sources = devices.list_sources()
    except Exception as exc:  # diagnostic command must not crash on environment issues
        sources = {'devices': [], 'default': None, 'error': str(exc)}
    try:
        mic = mic_node(conf, sources)
        mic_error = ''
    except RuntimeError as exc:
        mic = None
        mic_error = str(exc)
    try:
        eitype_path = inject.eitype_command()
    except RuntimeError:
        eitype_path = None
    service_mode = bool(str(conf.get('service_url') or '').strip())
    has_service_token = False
    service_error = ''
    if service_mode:
        try:
            has_service_token = bool(service_token())
        except (RuntimeError, OSError) as exc:
            service_error = str(exc)
    try:
        has_key = bool(api_key())
        key_error = ''
    except RuntimeError as exc:
        has_key = False
        key_error = str(exc)

    commands = {name: bool(shutil.which(name)) for name in
                ('pw-record', 'pw-dump', 'wpctl', 'wl-copy', 'wl-paste', 'curl', 'notify-send')}
    session = os.environ.get('XDG_SESSION_TYPE', 'unknown')
    desktop = os.environ.get('XDG_CURRENT_DESKTOP', 'unknown')
    phase = state().get('phase', 'idle')

    if session == 'wayland':
        _check(checks, 'session', 'ok', 'Wayland session detected')
    elif session == 'x11':
        _check(checks, 'session', 'warn', 'X11 is not implemented in this alpha; GNOME Wayland only')
    else:
        _check(checks, 'session', 'warn', 'XDG_SESSION_TYPE is unknown')
    if 'GNOME' in desktop.upper():
        _check(checks, 'desktop', 'ok', 'GNOME desktop detected')
    else:
        _check(checks, 'desktop', 'warn', f'Desktop {desktop!r} is outside the verified alpha target')

    if not commands['pw-record']:
        _check(checks, 'capture', 'fail', 'pw-record is missing (install pipewire-bin)')
    else:
        runtime = os.environ.get('XDG_RUNTIME_DIR', '')
        socket_ok = bool(runtime) and Path(runtime, 'pipewire-0').exists()
        _check(checks, 'capture', 'ok' if socket_ok else 'warn',
               'pw-record available; PipeWire socket ' + ('found' if socket_ok else 'not found'))

    if sources.get('error'):
        _check(checks, 'devices', 'warn', 'Device scan: ' + str(sources['error']))
    elif sources['devices']:
        _check(checks, 'devices', 'ok', f"{len(sources['devices'])} capture source(s) found")
    else:
        _check(checks, 'devices', 'fail', 'No PipeWire capture source found')
    if mic:
        _check(checks, 'microphone', 'ok', 'Active node: ' + mic)
    else:
        _check(checks, 'microphone', 'fail', mic_error or 'No microphone selected')

    if not commands['wl-copy']:
        _check(checks, 'clipboard', 'fail', 'wl-copy is missing (install wl-clipboard)')
    elif not commands['wl-paste']:
        _check(checks, 'clipboard', 'warn', 'wl-copy available; wl-paste missing (no restore)')
    else:
        _check(checks, 'clipboard', 'ok', 'wl-copy and wl-paste available')

    if eitype_path:
        _check(checks, 'injection', 'ok', 'eitype available: ' + eitype_path)
    elif conf.get('auto_insert'):
        _check(checks, 'injection', 'fail', 'eitype is missing while auto_insert is enabled')
    else:
        _check(checks, 'injection', 'warn', 'eitype missing; clipboard-only mode is configured')

    if service_mode:
        _check(checks, 'service', 'ok' if has_service_token else 'fail',
               ('Service: ' + conf['service_url'] + ' (token available)')
               if has_service_token else
               (service_error or 'Service token missing; run `scribiva service login`'))
        _check(checks, 'api_key', 'ok', 'Service mode: the Groq key stays on the server')
    else:
        _check(checks, 'api_key', 'ok' if has_key else 'fail',
               'Groq key available' if has_key else
               (key_error or 'Groq key missing; run `scribiva key`'))
    if not commands['curl']:
        _check(checks, 'curl', 'fail', 'curl is missing; API requests cannot be sent')
    else:
        _check(checks, 'curl', 'ok', 'curl is available')
    key_mode = None
    if KEY_PATH.exists():
        try:
            key_mode = oct(KEY_PATH.stat().st_mode & 0o777)
            broad = bool(KEY_PATH.stat().st_mode & 0o077)
            _check(checks, 'key_permissions', 'fail' if broad else 'ok',
                   f'Key file permissions {key_mode}' + (' are too broad; chmod 600' if broad else ''))
        except OSError as exc:
            _check(checks, 'key_permissions', 'warn', f'Cannot stat key file: {exc}')
    service_token_mode = None
    if service_mode and SERVICE_TOKEN_PATH.exists():
        try:
            service_token_mode = oct(SERVICE_TOKEN_PATH.stat().st_mode & 0o777)
            broad = bool(SERVICE_TOKEN_PATH.stat().st_mode & 0o077)
            _check(checks, 'service_token_permissions', 'fail' if broad else 'ok',
                   f'Service token permissions {service_token_mode}'
                   + (' are too broad; chmod 600' if broad else ''))
        except OSError as exc:
            _check(checks, 'service_token_permissions', 'warn',
                   f'Cannot stat service token: {exc}')

    try:
        secure_dirs()
        writable = os.access(ROOT_STATE, os.W_OK) and os.access(AUDIO_DIR, os.W_OK)
    except OSError:
        writable = False
    _check(checks, 'state_directory', 'ok' if writable else 'fail',
           'State and audio directories are writable' if writable else 'State directory is not writable')

    if not commands['notify-send']:
        _check(checks, 'notifications', 'warn', 'notify-send is missing; status notifications will not appear')

    if phase == 'error':
        _check(checks, 'last_job', 'warn', 'Last job failed: ' + str(state().get('error', ''))[:160])
    else:
        _check(checks, 'last_job', 'ok', 'Job phase: ' + phase)

    try:
        configured = conf.get('mic_node', '') or (
            MIC_PATH.read_text(encoding='utf-8').strip() if MIC_PATH.exists() else '')
    except OSError:
        configured = conf.get('mic_node', '')

    return {
        'version': __version__,
        'session': session,
        'desktop': desktop,
        'commands': commands,
        'eitype': bool(eitype_path),
        'eitype_path': eitype_path,
        'has_groq_key': has_key,
        'key_mode': key_mode,
        'key_error': key_error,
        'service_url': conf.get('service_url', ''),
        'service_mode': service_mode,
        'has_service_token': has_service_token,
        'service_token_mode': service_token_mode,
        'service_error': service_error,
        'mic_node': mic,
        'configured_source': configured,
        'default_source': sources.get('default'),
        'devices': sources.get('devices', []),
        'device_error': sources.get('error'),
        'mode': conf.get('mode', 'standard'),
        'language': conf.get('language', 'auto'),
        'phase': phase,
        'checks': checks,
        'healthy': not any(check['status'] == 'fail' for check in checks),
    }
