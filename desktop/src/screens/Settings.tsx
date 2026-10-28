// Settings (docs/06 §2): language, writing mode, zh script, dictation
// commands, microphone, max duration, hotkey, insertion policy with risk text.
// Controls render only when the backend reports the key, so options for one
// platform stay hidden on the other.
import { useCallback, useEffect, useId, useState } from 'react';
import { LinuxSetup } from './LinuxSetup';
import type { Devices } from '../backend';
import { SPEECH_LANGUAGES } from '../i18n';
import { has, Section, Select, Toggle, useApp } from '../ui';

export function SettingsScreen() {
  const { backend, t, settings, setSetting, report, status } = useApp();
  const [devices, setDevices] = useState<Devices | null>(null);
  const [saved, setSaved] = useState(false);

  const loadDevices = useCallback(async () => {
    try { setDevices(await backend.devices()); } catch (e) { report(e); }
  }, [backend, report]);
  useEffect(() => { loadDevices(); }, [loadDevices]);

  if (!settings) return <main className="grid"><p className="muted">…</p></main>;
  const s = settings;
  const str = (k: string) => String(s[k] ?? '');
  const bool = (k: string) => s[k] === true || s[k] === 'true';
  async function save(key: string, value: string | number | boolean) {
    setSaved(false);
    if (await setSetting(key, value)) setSaved(true);
  }

  const micKey = 'mic_device';
  const micValue = str(micKey);
  const known = new Set((devices?.devices || []).map(d => d.id));

  return (
    <main className="grid">
      <Section title={t('sec_writing')}>
        <Select label={t('speechLanguage')} value={str('language')} onChange={v => save('language', v)}
          options={[['auto', t('autoDetect')], ...SPEECH_LANGUAGES]} />
        <Select label={t('mode')} value={str('mode')} onChange={v => save('mode', v)}
          help={str('mode') === 'polished' ? t('polishedWarn') : undefined}
          options={[['raw', t('mode_raw')], ['standard', t('mode_standard')], ['polished', t('mode_polished')]]} />
        {has(s, 'output_script') && (
          <Select label={t('script')} value={str('output_script')} onChange={v => save('output_script', v)}
            options={[['', t('script_auto')], ['hans', t('script_hans')], ['hant', t('script_hant')]]} />
        )}
        {has(s, 'dictation_commands') && (
          <Toggle label={t('dictationCommands')} checked={bool('dictation_commands')}
            onChange={v => save('dictation_commands', v)} />
        )}
        {has(s, 'asr_model') && (
          <Select label="ASR model" value={str('asr_model')} onChange={v => save('asr_model', v)}
            options={[['whisper-large-v3-turbo', 'Whisper Large V3 Turbo'], ['whisper-large-v3', 'Whisper Large V3']]} />
        )}
      </Section>

      <Section title={t('sec_audio')}>
        <Select label={t('mic')} value={micValue} onChange={v => save(micKey, v)}
          options={[
            ['', t('micDefault')],
            ...(devices?.devices || []).map(d => [d.id, d.label + (d.isDefault ? ' ★' : '')] as [string, string]),
            ...(micValue && !known.has(micValue) ? [[micValue, micValue + ' (?)'] as [string, string]] : []),
          ]} />
        {devices?.error && <p className="error-text">{devices.error}</p>}
        <div className="actions"><button type="button" className="soft" onClick={loadDevices}>{t('refresh')}</button></div>
        <MicTest disabled={status?.phase === 'recording'} />
        <NumberField label={t('maxDuration')} value={Number(s.max_record_seconds) || 180}
          min={5} max={600} onCommit={v => save('max_record_seconds', v)} />
      </Section>

      <Section title={t('sec_input')}>
        {has(s, 'hotkey') && <HotkeyField value={str('hotkey')} error={status?.hotkeyError}
          onCommit={v => save('hotkey', v)} />}
        {backend.isLinux && <LinuxSetup />}
        {has(s, 'auto_insert') && (
          <Toggle label={t('autoInsert')} checked={bool('auto_insert')} risk={t('autoInsertRisk')}
            onChange={v => save('auto_insert', v)} />
        )}
        {has(s, 'insert_without_focus_check') && (
          <Toggle label={t('insertWithoutFocus')} checked={bool('insert_without_focus_check')}
            risk={t('insertWithoutFocusRisk')} onChange={v => save('insert_without_focus_check', v)} />
        )}
        {has(s, 'restore_text_clipboard') && (
          <Toggle label={t('restoreClipboard')} checked={bool('restore_text_clipboard')}
            onChange={v => save('restore_text_clipboard', v)} />
        )}
        {has(s, 'notifications') && (
          <Toggle label={t('notifications')} checked={bool('notifications')} onChange={v => save('notifications', v)} />
        )}
        {has(s, 'start_at_login') && (
          <Toggle label={t('startAtLogin')} checked={bool('start_at_login')} onChange={v => save('start_at_login', v)} />
        )}
      </Section>

      <Section title={t('sec_privacy')}>
        <p className="muted">{t('privacyNote')}</p>
        {has(s, 'delete_audio_on_success') && (
          <Toggle label="Delete local recording after success" checked={bool('delete_audio_on_success')}
            onChange={v => save('delete_audio_on_success', v)} />
        )}
      </Section>
      {saved && <p className="success toast" role="status">{t('saved')}</p>}
    </main>
  );
}

function NumberField({ label, value, min, max, onCommit }: {
  label: string; value: number; min: number; max: number; onCommit: (v: number) => void;
}) {
  const id = useId();
  const [draft, setDraft] = useState(String(value));
  useEffect(() => setDraft(String(value)), [value]);
  const commit = () => {
    const v = Number(draft);
    if (Number.isInteger(v) && v >= min && v <= max && v !== value) onCommit(v);
    else setDraft(String(value));
  };
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      <input id={id} type="number" inputMode="numeric" min={min} max={max} step={5} value={draft}
        onChange={e => setDraft(e.target.value)} onBlur={commit}
        onKeyDown={e => { if (e.key === 'Enter') commit(); }} />
    </div>
  );
}

function HotkeyField({ value, error, onCommit }: { value: string; error?: string | null; onCommit: (v: string) => void }) {
  const { t } = useApp();
  const id = useId();
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <div className="field">
      <label htmlFor={id}>{t('hotkey')}</label>
      {/* Typed text instead of key capture, so it is fully keyboard operable
          and the current global hotkey cannot fire while editing. */}
      <input id={id} type="text" value={draft} spellCheck={false} autoComplete="off"
        aria-describedby={id + '-help'} aria-invalid={!!error}
        onChange={e => setDraft(e.target.value)}
        onKeyDown={e => { if (e.key === 'Enter' && draft.trim() && draft !== value) onCommit(draft.trim()); }} />
      <p id={id + '-help'} className="help">{t('hotkeyHelp')}</p>
      {error && <p className="error-text" role="alert">{error}</p>}
      <div className="actions">
        <button type="button" className="soft" disabled={draft === value || !draft.trim()}
          onClick={() => onCommit(draft.trim())}>{t('apply')}</button>
        <button type="button" className="soft" onClick={() => onCommit('Ctrl+Alt+V')}>{t('hotkeyReset')}</button>
      </div>
    </div>
  );
}

export function MicTest({ disabled }: { disabled?: boolean }) {
  const { backend, t, report } = useApp();
  const [level, setLevel] = useState<{ peak: number; rms: number } | null>(null);
  const [busy, setBusy] = useState(false);
  async function run() {
    setBusy(true);
    try { setLevel(await backend.micTest()); } catch (e) { report(e); } finally { setBusy(false); }
  }
  const pct = Math.round(Math.min(1, (level?.peak || 0)) * 100);
  return (
    <div className="field">
      <p className="help">{t('micTestHint')}</p>
      <button type="button" className="soft" onClick={run} disabled={busy || disabled} aria-busy={busy}>
        {busy ? '…' : t('micTest')}
      </button>
      {level && (
        <div className="meter-row">
          <meter min={0} max={100} value={pct} aria-label={t('micLevel')} />
          <span>{t('micLevel')}: {pct}%{pct < 2 ? ' ⚠' : ''}</span>
        </div>
      )}
    </div>
  );
}
