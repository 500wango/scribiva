"""Focused-window identity for the Linux focus guard (ADR-012).

Sources, in order:
- GNOME Wayland: the Scribiva GNOME Shell extension on the session bus
  (``app.scribiva.Shell1.GetFocus``), queried with ``gdbus``.
- X11: ``_NET_ACTIVE_WINDOW`` via ``xprop``.
Otherwise the focus is ``None`` (unknown), and the caller copies instead of
pasting. Window titles are never read.
"""
from __future__ import annotations
import json
import os
import re
import shutil
import subprocess

DBUS_DEST = 'app.scribiva.Shell'
DBUS_PATH = '/app/scribiva/Shell'
DBUS_IFACE = 'app.scribiva.Shell1'

# Matched against app_id / wm_class (case-insensitive). Terminals get copy-only
# so a dictated command is never pasted into a shell (FR-005).
_TERMINAL_RE = re.compile(
    r'(terminal|konsole|kitty|alacritty|wezterm|foot|tilix|terminator|xterm|urxvt|'
    r'rxvt|guake|yakuake|ptyxis|blackbox|contour|st-256color|termite|sakura|qterminal|'
    r'lxterminal|mate-terminal|xfce4-terminal|ghostty|warp)', re.I)
_GDBUS_RE = re.compile(r"^\('((?:[^'\\]|\\.)*)', '((?:[^'\\]|\\.)*)', (?:uint64 )?(\d+), "
                       r"(?:uint32 )?(\d+)\)$")


def _run(cmd: list[str], timeout: float = 1.5) -> str | None:
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return proc.stdout.strip() if proc.returncode == 0 else None


def parse_gdbus_focus(output: str) -> dict | None:
    match = _GDBUS_RE.match(output.strip())
    if not match:
        return None
    app_id, wm_class, window_id, pid = match.groups()
    if int(window_id) == 0:
        return None
    return {'source': 'gnome-extension', 'app_id': app_id, 'wm_class': wm_class,
            'window_id': int(window_id), 'pid': int(pid)}


def _gnome_focus() -> dict | None:
    if not shutil.which('gdbus'):
        return None
    out = _run(['gdbus', 'call', '--session', '--dest', DBUS_DEST, '--object-path', DBUS_PATH,
                '--method', DBUS_IFACE + '.GetFocus'])
    return parse_gdbus_focus(out) if out else None


def _x11_focus() -> dict | None:
    if not os.environ.get('DISPLAY') or not shutil.which('xprop'):
        return None
    out = _run(['xprop', '-root', '_NET_ACTIVE_WINDOW'])
    match = re.search(r'window id # (0x[0-9a-fA-F]+)', out or '')
    if not match or int(match.group(1), 16) == 0:
        return None
    window = match.group(1)
    props = _run(['xprop', '-id', window, 'WM_CLASS', '_NET_WM_PID']) or ''
    classes = re.findall(r'"([^"]*)"', props.split('_NET_WM_PID')[0])
    pid = re.search(r'_NET_WM_PID\(CARDINAL\) = (\d+)', props)
    return {'source': 'x11', 'app_id': classes[-1] if classes else '',
            'wm_class': classes[0] if classes else '', 'window_id': int(window, 16),
            'pid': int(pid.group(1)) if pid else 0}


def extension_available() -> bool:
    if not shutil.which('gdbus'):
        return False
    out = _run(['gdbus', 'call', '--session', '--dest', 'org.freedesktop.DBus',
                '--object-path', '/org/freedesktop/DBus',
                '--method', 'org.freedesktop.DBus.NameHasOwner', DBUS_DEST])
    return bool(out and 'true' in out)


def snapshot() -> dict | None:
    """Identity of the focused window, or None when it cannot be determined."""
    session = os.environ.get('XDG_SESSION_TYPE', '')
    if session == 'x11':
        return _x11_focus()
    return _gnome_focus()


def same_target(before: dict | None, after: dict | None) -> bool:
    if not before or not after:
        return False
    return (before.get('source') == after.get('source')
            and before.get('window_id') == after.get('window_id')
            and before.get('app_id') == after.get('app_id'))


def is_own_window(target: dict | None) -> bool:
    """Scribiva's own window: clicking its record button focuses it, and the
    result must not be typed back into Scribiva itself."""
    if not target:
        return False
    ids = ((target.get('app_id') or '') + ' ' + (target.get('wm_class') or '')).lower()
    return 'scribiva' in ids


def is_terminal(target: dict | None) -> bool:
    if not target:
        return False
    return bool(_TERMINAL_RE.search(target.get('app_id') or '')
                or _TERMINAL_RE.search(target.get('wm_class') or ''))


def extension_version() -> int:
    """Interface version of the running GNOME extension, 0 when unavailable."""
    if not shutil.which('busctl'):
        return 0
    out = _run(['busctl', '--user', '--json=short', 'get-property', DBUS_DEST, DBUS_PATH,
                DBUS_IFACE, 'Version'])
    try:
        return int(json.loads(out)['data']) if out else 0
    except (ValueError, KeyError, TypeError):
        return 0


# Outcomes of InsertText. "committed" (input method) and "pasted" mean the
# text was sent to the remembered window; the others mean nothing was typed.
INSERTED = {'committed', 'pasted'}


def insert_text(text: str, target: dict, *, single_line: bool = False,
                paste_shortcut: str = 'ctrl+v', restore_clipboard: bool = True,
                timeout: float = 6.0) -> str | None:
    """Type ``text`` into ``target`` through the GNOME extension (v2+).

    The extension re-checks inside the compositor that ``target`` is still the
    focused window and commits through the input method, falling back to
    clipboard + one paste chord. Returns the outcome string, or None when the
    extension cannot be used (caller falls back to the client-side paste).
    """
    if not target or target.get('source') != 'gnome-extension' or not shutil.which('busctl'):
        return None
    if extension_version() < 2:
        return None
    args = ['busctl', '--user', '--json=short', '--timeout=' + str(int(timeout)), '--',
            'call', DBUS_DEST, DBUS_PATH, DBUS_IFACE, 'InsertText', 'sta{sv}',
            text, str(int(target['window_id'])), '3',
            'single_line', 'b', 'true' if single_line else 'false',
            'paste_shortcut', 's', paste_shortcut,
            'restore_clipboard', 'b', 'true' if restore_clipboard else 'false']
    out = _run(args, timeout=timeout + 1)
    try:
        return str(json.loads(out)['data'][0]) if out else None
    except (ValueError, KeyError, IndexError, TypeError):
        return None


def set_indicator(phase: str) -> None:
    """Best-effort top-bar indicator through the GNOME extension; never raises."""
    if phase not in ('idle', 'recording', 'processing', 'error') or not shutil.which('gdbus'):
        return
    _run(['gdbus', 'call', '--session', '--dest', DBUS_DEST, '--object-path', DBUS_PATH,
          '--method', DBUS_IFACE + '.SetState', phase], timeout=1.0)
