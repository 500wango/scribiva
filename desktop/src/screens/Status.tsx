// Status + recovery screen (docs/06 §3, PRD §7.5).
import { useCallback, useEffect, useState } from 'react';
import type { Diagnostics as DiagnosticsData, Last } from '../backend';
import { noticeKey, phaseKey } from '../i18n';
import { errorText, PHASE_GLYPH, Section, useApp } from '../ui';

function elapsed(startMs: number | null | undefined, now: number): string {
  if (!startMs) return '';
  const s = Math.max(0, Math.floor((now - startMs) / 1000));
  return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
}

export function StatusScreen({ onOpen }: { onOpen: (screen: 'account' | 'settings') => void }) {
  const { backend, t, status, report, refreshStatus } = useApp();
  const [last, setLast] = useState<Last | null>(null);
  const [showRaw, setShowRaw] = useState(false);
  const [now, setNow] = useState(Date.now());
  const [copied, setCopied] = useState('');

  const phase = status?.phase || 'unknown';
  const loadLast = useCallback(async () => {
    try { setLast(await backend.last()); } catch (e) { report(e); }
  }, [backend, report]);

  // Reload the last result whenever the state changes.
  useEffect(() => { loadLast(); }, [loadLast, phase]);
  useEffect(() => {
    if (phase !== 'recording') return;
    const id = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(id);
  }, [phase]);

  async function act(name: Parameters<typeof backend.action>[0]) {
    try {
      setCopied('');
      await backend.action(name);
      if (name === 'copy-last' || name === 'copy-raw') setCopied(t('phase_clipboard_only'));
      await refreshStatus();
      await loadLast();
    } catch (e) { report(e); }
  }

  // Linux: the GNOME shortcut, empty when none is set up yet.
  const hotkey = status?.hotkey || (backend.isWindows ? 'Ctrl+Alt+V' : t('lx_no_hotkey'));
  const busy = phase === 'recording' || phase === 'processing' || phase === 'injecting';
  const notice = status?.notice ? noticeKey(status.notice) : null;
  const inserted = phase === 'ready' && last?.outcome === 'inserted';
  const hint = phase === 'idle' ? t('hint_idle', { hotkey })
    : phase === 'recording' ? t(backend.isWindows ? 'hint_recording' : 'hint_recording_linux', { hotkey })
      : phase === 'ready' ? t(inserted ? 'hint_inserted' : 'hint_ready') : '';
  const text = showRaw ? last?.raw : last?.text;
  const quotaReset = status?.error?.class === 'quota' || phase === 'quota';
  const noise = phase === 'suspected_noise';
  const canInsert = !!status?.canInsert;

  return (
    <main className="grid">
      <Section title={t('nav_status')}>
        <div className={`status ${phase}`} role="status" aria-live="polite">
          <span className="glyph" aria-hidden="true">{PHASE_GLYPH[phase] || '·'}</span>
          <div>
            <div className="statusName">
              {t(inserted ? 'phase_inserted' : phaseKey(phase))}
              {phase === 'recording' && status?.recordingStartedMs ? ` · ${elapsed(status.recordingStartedMs, now)}` : ''}
            </div>
            {hint && <p className="muted">{hint}</p>}
            {notice && <p className="notice-text">{t(notice)}</p>}
            {noise && <p className="notice-text">{t('noise_explain')}</p>}
            {status?.error && <p className="error-text">{status.error.code}: {status.error.message}</p>}
            {status?.hotkeyError && <p className="error-text">{status.hotkeyError}</p>}
          </div>
        </div>
        <div className="actions">
          <button type="button" onClick={() => act('toggle')} disabled={phase === 'processing'}>
            {phase === 'recording' ? t('stop') : t('start')}
          </button>
          <button type="button" className="soft" onClick={() => act('cancel')} disabled={!busy}>{t('cancel')}</button>
          <button type="button" className="soft" onClick={() => act('retry')} disabled={!status?.canRetry}>{t('retry')}</button>
          <button type="button" className="soft" onClick={() => act('discard')} disabled={phase !== 'error' && !busy}>{t('discard')}</button>
          {!busy && phase !== 'idle' &&
            <button type="button" className="soft" onClick={() => act('acknowledge')}>{t('dismiss')}</button>}
        </div>
        {noise && (
          <div className="callout" role="group" aria-label={t('phase_suspected_noise')}>
            <button type="button" onClick={() => act('insert-last')} disabled={!canInsert}
              aria-describedby="insert-hint">{t('insertAnyway')}</button>
            <button type="button" className="soft" onClick={() => act('discard-last')}>{t('discardResult')}</button>
            <p id="insert-hint" className="muted small">{t('insertHint')}</p>
          </div>
        )}
        <UpdateBanner />
        {quotaReset && (
          <div className="callout">
            <button type="button" onClick={() => onOpen('account')}>{t('nav_account')}</button>
          </div>
        )}
        {!status?.signedIn && (
          <div className="callout">
            <span>{t('notSignedIn')}</span>
            <button type="button" className="soft" onClick={() => onOpen('account')}>{t('signIn')}</button>
          </div>
        )}
      </Section>

      <Section title={t('lastResult')}>
        <div className="actions">
          <button type="button" className="soft" disabled={!last?.text} onClick={() => act('copy-last')}>{t('copy')}</button>
          <button type="button" className="soft" disabled={!last?.raw} onClick={() => act('copy-raw')}>{t('copyRaw')}</button>
          <button type="button" className="soft" disabled={!canInsert} onClick={() => act('insert-last')}
            aria-describedby="insert-again-hint">{noise ? t('insertAnyway') : t('insertAgain')}</button>
          <button type="button" className="soft" disabled={!last?.available} aria-pressed={showRaw}
            onClick={() => setShowRaw(!showRaw)}>{showRaw ? t('showFinal') : t('showRaw')}</button>
          <button type="button" className="soft" disabled={!last?.available} onClick={() => act('clear-last')}>{t('clear')}</button>
        </div>
        {/* dir="auto" keeps RTL transcripts (Arabic) readable. */}
        <pre className="lastText" dir="auto" tabIndex={0} aria-label={t('lastResult')}>
          {last?.available ? (text || '—') : t('noResult')}
        </pre>
        {copied && <p className="success" role="status">{copied}</p>}
        <p id="insert-again-hint" className="muted small">{t('insertHint')}</p>
        {!!last?.warnings.length && (
          <p className="muted">{t('warnings')}: {last.warnings.join(' · ')}</p>
        )}
        {last?.suspectedNoise && <p className="notice-text">{t('phase_suspected_noise')}</p>}
        <p className="muted small">{t('memoryOnly')}</p>
      </Section>

      <Diagnostics />
      {status?.login && (
        <Section title={t('signIn')}>
          <p>{t('loginCode')}: <b className="code">{status.login.user_code}</b></p>
          <p className="muted">{status.login.error ? errorText(status.login.error) : t('loginWaiting')}</p>
        </Section>
      )}
    </main>
  );
}

/** FR-012: signed update ready / available (never installed while busy). */
export function UpdateBanner() {
  const { backend, t, status, report } = useApp();
  const u = status?.update;
  if (!u || u.state === 'downloading') return null;
  const busy = status?.phase === 'recording' || status?.phase === 'processing' || status?.phase === 'injecting';
  async function run(name: 'install-update' | 'open-release-page') {
    try { await backend.action(name); } catch (e) { report(e); }
  }
  return (
    <div className="callout" role="status">
      {u.state === 'ready' && (<>
        <p>{t('updateReady', { version: u.version })}</p>
        <button type="button" onClick={() => run('install-update')} disabled={busy}>{t('updateRestart')}</button>
        {busy && <p className="muted small">{t('updateBusy')}</p>}
      </>)}
      {u.state === 'manual' && (<>
        <p>{t('updateManual', { version: u.version })}</p>
        <button type="button" className="soft" onClick={() => run('open-release-page')}>{t('updateDownload')}</button>
      </>)}
      {u.state === 'failed' && <p className="error-text">{t('updateFailed', { version: u.version, error: u.error || '' })}</p>}
    </div>
  );
}

function Diagnostics() {
  const { backend, t, report } = useApp();
  const [data, setData] = useState<DiagnosticsData | null>(null);
  const [busy, setBusy] = useState(false);
  const load = useCallback(async () => {
    setBusy(true);
    try { setData(await backend.diagnostics()); } catch (e) { report(e); } finally { setBusy(false); }
  }, [backend, report]);
  useEffect(() => { load(); }, [load]);
  return (
    <Section title={t('diagnostics')} wide>
      {data && (
        <p className="diagnostic">
          session: {data.session || '-'} · desktop: {data.desktop || '-'}<br />
          mic: {data.mic_node || '-'} · service: {data.service_url || '-'}
        </p>
      )}
      <ul className="checks">
        {(data?.checks || []).map(c => (
          <li key={c.id} className="checkItem">
            <b>{c.id} <span className={`state ${c.status}`}>[{c.status}]</span></b>
            <span>{c.detail}</span>
          </li>
        ))}
      </ul>
      <div className="actions">
        <button type="button" className="soft" onClick={load} disabled={busy} aria-busy={busy}>{t('refresh')}</button>
      </div>
    </Section>
  );
}
