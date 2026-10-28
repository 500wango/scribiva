"""First-run desktop integration for Linux (run as the user, never as root).

- GNOME global shortcut that runs ``scribiva toggle`` (FR-001). Existing custom
  shortcuts are preserved; a binding already used by another custom shortcut
  is reported instead of overwritten.
- GNOME Shell extension state, and enabling it (ADR-012). On Wayland a newly
  installed extension is only loaded after the user logs out and back in.
"""
from __future__ import annotations
import ast
import os
import shutil
import subprocess

SCHEMA = 'org.gnome.settings-daemon.plugins.media-keys'
BASE = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/'
OUR_PATH = BASE + 'scribiva/'
LEGACY_PATHS = (BASE + 'scribiva-alpha/', BASE + 'scribiva-gate1/')
EXTENSION_UUID = 'scribiva@scribiva.app'
DEFAULT_BINDING = '<Primary><Alt>v'


def _gsettings(*args: str) -> str:
    proc = subprocess.run(['gsettings', *args], capture_output=True, text=True,
                          timeout=5, check=True)
    return proc.stdout.strip()


def _item(path: str) -> str:
    return SCHEMA + '.custom-keybinding:' + path


def _paths() -> list[str]:
    raw = _gsettings('get', SCHEMA, 'custom-keybindings')
    if raw.startswith('@as '):
        raw = raw[4:]
    value = ast.literal_eval(raw)
    return [p for p in value if isinstance(p, str)]


def _unquote(value: str) -> str:
    try:
        parsed = ast.literal_eval(value)
        return parsed if isinstance(parsed, str) else value
    except (ValueError, SyntaxError):
        return value


def _is_scribiva(path: str, command: str) -> bool:
    """Shortcuts created by any Scribiva version (incl. Gate/alpha scripts)."""
    return path == OUR_PATH or path in LEGACY_PATHS or 'scribiva' in command.lower()


def _command_target_exists(command: str) -> bool:
    exe = command.strip().split()[0] if command.strip() else ''
    exe = exe.strip('\'"')
    return bool(exe) and (os.path.isfile(exe) and os.access(exe, os.X_OK)
                          or bool(shutil.which(exe)))


def _ours(paths: list[str]) -> list[tuple[str, str, str]]:
    found = []
    for path in paths:
        command = _unquote(_gsettings('get', _item(path), 'command'))
        if _is_scribiva(path, command):
            found.append((path, command, _unquote(_gsettings('get', _item(path), 'binding'))))
    return found


def toggle_command() -> str:
    """Absolute command for the shortcut: the packaged binary if present."""
    for candidate in ('/usr/bin/scribiva-cli', os.path.expanduser('~/.local/bin/scribiva')):
        if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate + ' toggle'
    exe = shutil.which('scribiva-cli') or shutil.which('scribiva')
    if not exe:
        raise RuntimeError('Scribiva command not found')
    return exe + ' toggle'


def hotkey_status() -> dict:
    if not shutil.which('gsettings'):
        return {'available': False, 'installed': False}
    try:
        ours = _ours(_paths())
    except (OSError, subprocess.SubprocessError, ValueError, SyntaxError):
        return {'available': False, 'installed': False}
    if not ours:
        return {'available': True, 'installed': False}
    path, command, binding = next((o for o in ours if o[0] == OUR_PATH), ours[0])
    working = _command_target_exists(command) and path == OUR_PATH and len(ours) == 1
    # "broken": a shortcut exists but runs a deleted script or an old copy.
    return {'available': True, 'installed': working, 'broken': not working,
            'binding': binding, 'command': command}


def bind_hotkey(binding: str = DEFAULT_BINDING) -> dict:
    """Create or update the Scribiva GNOME shortcut; never steals another one."""
    if not shutil.which('gsettings'):
        raise RuntimeError('gsettings not found; set a shortcut to `scribiva-cli toggle` '
                           'in your desktop settings')
    command = toggle_command()
    paths = _paths()
    ours = {path for path, _, _ in _ours(paths)}
    for path in paths:
        if path in ours:
            continue
        if _unquote(_gsettings('get', _item(path), 'binding')) == binding:
            name = _unquote(_gsettings('get', _item(path), 'name'))
            raise RuntimeError(f'{binding} is already used by the custom shortcut "{name}"; '
                               'choose another binding')
    for key, value in (('name', 'Scribiva'), ('command', command), ('binding', binding)):
        _gsettings('set', _item(OUR_PATH), key, value)
    stale = ours - {OUR_PATH}
    updated = [p for p in paths if p not in stale]
    if OUR_PATH not in updated:
        updated.append(OUR_PATH)
    if updated != paths:
        _gsettings('set', SCHEMA, 'custom-keybindings', repr(updated))
    # Older Scribiva entries (Gate scripts, source installs) are cleared so a
    # single shortcut runs the current command.
    for old in stale:
        for key in ('name', 'command', 'binding'):
            subprocess.run(['gsettings', 'reset', _item(old), key],
                           capture_output=True, timeout=5, check=False)
    return {'binding': binding, 'command': command, 'replaced': sorted(stale)}


def repair_hotkey() -> dict | None:
    """Point an existing but broken Scribiva shortcut at the current command,
    keeping the user's key combination. Does nothing when none exists (the
    user has not opted in) or when it already works."""
    status = hotkey_status()
    if not status.get('broken'):
        return None
    return bind_hotkey(status.get('binding') or DEFAULT_BINDING)


def _string_list(schema: str, key: str) -> list[str]:
    raw = _gsettings('get', schema, key)
    if raw.startswith('@as '):
        raw = raw[4:]
    return [x for x in ast.literal_eval(raw) if isinstance(x, str)]


EXTENSION_DIRS = ('/usr/share/gnome-shell/extensions',
                  os.path.expanduser('~/.local/share/gnome-shell/extensions'))


def extension_status() -> dict:
    """installed / enabled (persisted choice) / active (loaded in this session).

    Uses gsettings and the filesystem rather than ``gnome-extensions info``,
    whose output is localised.
    """
    from . import focus
    installed = any(os.path.isfile(os.path.join(d, EXTENSION_UUID, 'metadata.json'))
                    for d in EXTENSION_DIRS)
    enabled = False
    if shutil.which('gsettings'):
        try:
            enabled = (EXTENSION_UUID in _string_list('org.gnome.shell', 'enabled-extensions')
                       and EXTENSION_UUID not in _string_list('org.gnome.shell',
                                                              'disabled-extensions'))
        except (OSError, subprocess.SubprocessError, ValueError, SyntaxError):
            enabled = False
    return {'installed': installed, 'enabled': enabled, 'active': focus.extension_available()}


def enable_extension() -> dict:
    if not extension_status()['installed']:
        raise RuntimeError('The Scribiva GNOME extension is not installed')
    if shutil.which('gnome-extensions'):
        subprocess.run(['gnome-extensions', 'enable', EXTENSION_UUID],
                       capture_output=True, timeout=10, check=False)
    # On Wayland the shell may not have scanned a newly installed extension
    # yet, so the command can fail; the persisted list makes it load at login.
    try:
        enabled = _string_list('org.gnome.shell', 'enabled-extensions')
        if EXTENSION_UUID not in enabled:
            _gsettings('set', 'org.gnome.shell', 'enabled-extensions',
                       repr(enabled + [EXTENSION_UUID]))
        disabled = _string_list('org.gnome.shell', 'disabled-extensions')
        if EXTENSION_UUID in disabled:
            _gsettings('set', 'org.gnome.shell', 'disabled-extensions',
                       repr([x for x in disabled if x != EXTENSION_UUID]))
    except (OSError, subprocess.SubprocessError, ValueError, SyntaxError) as exc:
        raise RuntimeError('Could not enable the extension: ' + str(exc)[:200]) from exc
    status = extension_status()
    status['needs_relogin'] = status['enabled'] and not status['active']
    return status


def status() -> dict:
    desktop = os.environ.get('XDG_CURRENT_DESKTOP', '')
    return {
        'session': os.environ.get('XDG_SESSION_TYPE', 'unknown'),
        'gnome': 'GNOME' in desktop.upper(),
        'hotkey': hotkey_status(),
        'extension': extension_status(),
    }
