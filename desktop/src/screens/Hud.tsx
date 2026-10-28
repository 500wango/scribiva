// Recording HUD (docs/06 §3): rendered in the separate non-focusable `hud`
// window. Read-only: it never takes focus or input (clicks pass through),
// so cancel stays on Esc (Windows), the tray and the main window.
import { useEffect, useState } from 'react';
import { Backend, type Status } from '../backend';
import { type UiLang } from '../i18n';
import { makeT, PHASE_GLYPH } from '../ui';

function elapsed(startMs: number | null | undefined, now: number): string {
  if (!startMs) return '';
  const s = Math.max(0, Math.floor((now - startMs) / 1000));
  return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
}

export function Hud({ backend, lang }: { backend: Backend; lang: UiLang }) {
  const [status, setStatus] = useState<Status | null>(null);
  const [now, setNow] = useState(Date.now());
  const t = makeT(lang);

  useEffect(() => {
    let off = () => {};
    backend.status().then(setStatus).catch(() => {});
    backend.onStatus(setStatus).then(f => { off = f; });
    return () => off();
  }, [backend]);
  useEffect(() => {
    if (status?.phase !== 'recording') return;
    const id = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(id);
  }, [status?.phase]);

  const phase = status?.phase || 'idle';
  const label = phase === 'recording' ? t('hud_recording')
    : phase === 'injecting' ? t('hud_injecting') : t('hud_processing');
  const time = phase === 'recording' ? elapsed(status?.recordingStartedMs, now) : '';
  // Windows registers Esc globally only while recording.
  const hint = backend.isWindows && phase === 'recording' ? t('hud_hint_esc') : t('hud_hint_tray');
  return (
    <div className={`hud ${phase}`} role="status" aria-live="polite">
      <span className="glyph" aria-hidden="true">{PHASE_GLYPH[phase] || '·'}</span>
      <b>{label}</b>
      {time && <span className="hud-time">{time}</span>}
      <span className="hud-hint">{hint}</span>
    </div>
  );
}
