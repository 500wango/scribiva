// Linux desktop integration (FR-001, ADR-012): GNOME shortcut and the Scribiva
// GNOME Shell extension. Both are user-level and need no administrator rights.
import { useCallback, useEffect, useState } from 'react';
import type { LinuxSetup as Setup } from '../backend';
import { useApp } from '../ui';

export function LinuxSetup() {
  const { backend, t, refreshStatus } = useApp();
  const [setup, setSetup] = useState<Setup | null>(null);
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const [relogin, setRelogin] = useState(false);

  const refresh = useCallback(async () => {
    try { setSetup(await backend.linuxSetupStatus()); } catch (e) { setError(String(e)); }
  }, [backend]);
  useEffect(() => { refresh(); }, [refresh]);

  async function run(action: 'hotkey' | 'extension') {
    setBusy(action); setError('');
    try {
      const r = await backend.linuxSetup(action);
      setSetup(r.status);
      if (r.extension?.needs_relogin) setRelogin(true);
      await refreshStatus();
    } catch (e) { setError(String(e)); }
    finally { setBusy(''); }
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
        {setup.hotkey.available && (
          <button type="button" disabled={busy !== ''} onClick={() => run('hotkey')}>
            {setup.hotkey.installed ? t('lx_hotkey_rebind') : t('lx_hotkey_bind')}
          </button>
        )}
      </div>

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
      {(relogin || extState === 'pending') && <p className="warning-text" role="status">{t('lx_relogin')}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  );
}

