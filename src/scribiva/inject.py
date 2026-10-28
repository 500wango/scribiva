"""GNOME Wayland clipboard + libei/eitype output; best-effort dispatch.

There is no generic confirmation that a different app accepted paste. Never
silently retry if eitype may already have sent Ctrl+V.
"""
from __future__ import annotations
import os
import shutil
import subprocess
import time


# Bundled by the .deb (Apache-2.0, license in /usr/share/doc/scribiva/).
BUNDLED_EITYPE = '/usr/lib/scribiva/bin/eitype'


def eitype_command() -> str:
    for candidate in (os.path.expanduser('~/.cargo/bin/eitype'), BUNDLED_EITYPE):
        if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
    exe = shutil.which('eitype')
    if not exe:
        raise RuntimeError('eitype missing; set up libei/eitype on GNOME Wayland')
    return exe


def _clipboard_text() -> bytes | None:
    if not shutil.which('wl-paste'):
        return None
    try:
        proc = subprocess.run(['wl-paste', '--no-newline', '--type', 'text/plain;charset=utf-8'],
                              capture_output=True, timeout=2, check=False)
        if proc.returncode == 0 and len(proc.stdout) < 2_000_000:
            return proc.stdout
    except (OSError, subprocess.TimeoutExpired):
        pass
    return None


def copy_text(text: str) -> None:
    subprocess.run(['wl-copy', '--type', 'text/plain;charset=utf-8'],
                   input=text.encode('utf-8'), check=True, timeout=5)


def dispatch(text: str, restore: bool = True) -> str:
    """Return 'dispatched_unverified'. Raised error may still have sent hotkey."""
    existing = _clipboard_text() if restore else None
    copy_text(text)
    time.sleep(0.12)
    try:
        subprocess.run([eitype_command(), '-M', 'ctrl', 'v'], timeout=12, check=True)
    except (OSError, subprocess.TimeoutExpired, subprocess.CalledProcessError) as exc:
        # Clipboard intentionally retains result for manual paste.
        raise RuntimeError('Paste dispatch failed/uncertain, text remains in clipboard; do not auto-retry') from exc
    # Clipboard manager and target app may read asynchronously. Avoid restoring
    # too quickly, or clobbering user changes made after our paste.
    if existing is not None:
        time.sleep(0.8)
        if _clipboard_text() == text.encode('utf-8'):
            try:
                subprocess.run(['wl-copy', '--type', 'text/plain;charset=utf-8'],
                               input=existing, timeout=5, check=True)
            except (OSError, subprocess.TimeoutExpired, subprocess.CalledProcessError):
                pass
    return 'dispatched_unverified'
