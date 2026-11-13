// Linux desktop integration (FR-001, ADR-012): GNOME shortcut, the Scribiva
// GNOME Shell extension and the KDE KWin focus script. All are user-level and
// need no administrator rights.
import { useCallback, useEffect, useId, useState } from 'react';
import type { LinuxSetup as Setup } from '../backend';
import { useApp } from '../ui';

const MOD_ALIASES: Record<string, string> = {
  ctrl: '<Primary>', control: '<Primary>', primary: '<Primary>',
  alt: '<Alt>', shift: '<Shift>', super: '<Super>', meta: '<Super>', win: '<Super>',
};
const MOD_ORDER = ['<Primary>', '<Alt>', '<Shift>', '<Super>'];

/**
 * `Ctrl+Alt+V` → `<Primary><Alt>v` (GNOME accelerator). A bare `F13`–`F24` or
 * `XF86*` media key is accepted too, matching the backend's
 * `valid_gnome_binding`; other single keys would swallow normal typing.
 */
export function toGnomeBinding(input: string): string | null {
  const parts = input.split('+').map(p => p.trim()).filter(Boolean);
  if (parts.length === 1) {
    if (/^f(1[3-9]|2[0-4])$/i.test(parts[0])) return parts[0].toUpperCase();
    if (/^xf86[a-z0-9]+$/i.test(parts[0])) return parts[0];
    return null;
  }
  if (parts.length < 2) return null;
  const mods: string[] = [];
  for (const raw of parts.slice(0, -1)) {
    const mod = MOD_ALIASES[raw.toLowerCase()];
    if (!mod) return null;
    if (!mods.includes(mod)) mods.push(mod);
  }
  if (!mods.length) return null;
  mods.sort((a, b) => MOD_ORDER.indexOf(a) - MOD_ORDER.indexOf(b));
  let key = parts[parts.length - 1];
  if (key.length === 1) key = key.toLowerCase();
  else if (/^f([1-9]|1[0-9]|2[0-4])$/i.test(key)) key = key.toUpperCase();
  else if (key.toLowerCase() === 'space') key = 'space';
  else if (key.toLowerCase() === 'tab') key = 'Tab';
  else return null;
  return mods.join('') + key;
}

export function LinuxSetup() {
  const { backend, t, refreshStatus } = useApp();
  const [setup, setSetup] = useState<Setup | null>(null);
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const [relogin, setRelogin] = useState(false);
  const [draft, setDraft] = useState('');
  const [hotkeyError, setHotkeyError] = useState('');
  const hotkeyId = useId();

  const refresh = useCallback(async () => {
    try { setSetup(await backend.linuxSetupStatus()); } catch (e) { setError(String(e)); }
  }, [backend]);
  useEffect(() => { refresh(); }, [refresh]);

  useEffect(() => { setDraft(setup?.hotkey.label || 'Ctrl+Alt+V'); },
    [setup?.hotkey.label]);

  async function run(action: 'hotkey' | 'extension' | 'kwin') {
    setBusy(action); setError('');
    try {
      const r = await backend.linuxSetup(action);
      setSetup(r.status);
      if (r.extension?.needs_relogin) setRelogin(true);
      await refreshStatus();
    } catch (e) { setError(String(e)); }
    finally { setBusy(''); }
  }

  async function applyHotkey(value: string) {
    const binding = toGnomeBinding(value);
    if (!binding) { setHotkeyError(t('lx_hotkey_invalid')); return; }
    setBusy('hotkey'); setHotkeyError(''); setError('');
    try {
      const r = await backend.linuxSetup('hotkey', binding);
      setSetup(r.status);
      await refreshStatus();
    } catch (e) {
      const msg = String(e);
      const detail = msg.replace(/^.*HOTKEY_CONFLICT:\s*/i, '');
      setHotkeyError(/hotkey_conflict/i.test(msg)
        ? t('lx_hotkey_conflict', { detail })
        : msg);
    } finally { setBusy(''); }
  }

  if (!setup) return <p className="muted">{error || t('loading')}</p>;
  const ext = setup.extension;
  const extState = ext.active ? 'ok' : ext.enabled ? 'pending' : ext.installed ? 'off' : 'missing';
  const extLabel = { ok: 'lx_ext_ok', pending: 'lx_ext_pending', off: 'lx_ext_off', missing: 'lx_ext_missing' } as const;

  return (
    <div className="linux-setup">
      <div className="setup-row">
        <div>
          <h3>{t('lx_hotkey_title')}</h3>
          <p className="muted">
            {setup.hotkey.installed
              ? t('lx_hotkey_done', { binding: setup.hotkey.label || setup.hotkey.binding || '' })
              : setup.hotkey.broken ? t('lx_hotkey_broken', { binding: setup.hotkey.label || '' })
                : setup.hotkey.available ? t('lx_hotkey_missing') : t('lx_hotkey_manual', { command: setup.command })}
          </p>
          <p className="muted small">{t('lx_cancel_hint')}</p>
        </div>
      </div>
      {setup.hotkey.available && (
        <div className="field">
          <label htmlFor={hotkeyId}>{t('lx_hotkey_custom')}</label>
          <input id={hotkeyId} type="text" value={draft} spellCheck={false} autoComplete="off"
            placeholder="Ctrl+Alt+V" aria-describedby={hotkeyId + '-help'}
            aria-invalid={!!hotkeyError}
            onChange={e => setDraft(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') applyHotkey(draft); }} />
          <p id={hotkeyId + '-help'} className="help">{t('lx_hotkey_custom_help')}</p>
          {hotkeyError && <p className="error-text" role="alert">{hotkeyError}</p>}
          <div className="actions">
            <button type="button" className="soft" disabled={busy !== '' || !draft.trim()}
              onClick={() => applyHotkey(draft)}>{t('apply')}</button>
            <button type="button" className="soft" disabled={busy !== ''}
              onClick={() => { setDraft('Ctrl+Alt+V'); applyHotkey('Ctrl+Alt+V'); }}>
              {t('hotkeyReset')}</button>
          </div>
        </div>
      )}

      {setup.gnome && (
        <div className="setup-row">
          <div>
            <h3>{t('lx_ext_title')}</h3>
            <p className="muted">
              <span className={`state ${extState === 'ok' ? 'ok' : 'warn'}`}>
                {extState === 'ok' ? '✓ ' : '! '}{t(extLabel[extState])}
              </span>
            </p>
            <p className="muted">{t('lx_ext_why')}</p>
          </div>
          {ext.installed && !ext.enabled && (
            <button type="button" disabled={busy !== ''} onClick={() => run('extension')}>
              {t('lx_ext_enable')}
            </button>
          )}
        </div>
      )}
      {setup.kwin?.kde && (
        <div className="setup-row">
          <div>
            <h3>{t('lx_kwin_title')}</h3>
            <p className="muted">
              <span className={`state ${setup.kwin.kwin_focus_active ? 'ok' : 'warn'}`}>
                {setup.kwin.kwin_focus_active ? '✓ ' : '! '}
                {t(setup.kwin.kwin_focus_active ? 'lx_kwin_ok'
                  : setup.kwin.kwin_script_enabled ? 'lx_kwin_waiting'
                    : setup.kwin.kwin_script_installed ? 'lx_kwin_off' : 'lx_kwin_missing')}
              </span>
            </p>
            <p className="muted">{t('lx_kwin_why')}</p>
          </div>
          {setup.kwin.kwin_script_installed && !setup.kwin.kwin_focus_active && (
            <button type="button" disabled={busy !== ''} onClick={() => run('kwin')}>{t('lx_kwin_enable')}</button>
          )}
        </div>
      )}
      {(relogin || extState === 'pending') && <p className="muted" role="status">{t('lx_relogin')}</p>}
      {ext.update_at_next_login && <p className="muted" role="status">{t('lx_ext_update')}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  );
}

