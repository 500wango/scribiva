// First-run flow (docs/06 §4): value, privacy notice (US + Groq), sign-in,
// microphone test, preferences (language, mode, insertion, HUD, start at
// login), hotkey (Linux: GNOME shortcut + extension setup).
import { useState } from 'react';
import { SPEECH_LANGUAGES } from '../i18n';
import { has, Select, Toggle, useApp } from '../ui';
import { LoginPanel } from './Account';
import { MicTest } from './Settings';
import { LinuxSetup } from './LinuxSetup';

export function Onboarding({ onDone }: { onDone: () => void }) {
  const { backend, t, status, settings, setSetting } = useApp();
  const steps = ['privacy', 'login', 'mic', 'prefs', 'hotkey'] as const;
  const [i, setI] = useState(0);
  const [agreed, setAgreed] = useState(false);
  const step = steps[i];

  async function finish() {
    await setSetting('onboarding_done', true);
    onDone();
  }

  return (
    <main className="onboarding" aria-labelledby="ob-title">
      <h1 id="ob-title">{t('ob_title')}</h1>
      <p className="subtitle">{t('ob_value')}</p>
      <ol className="steps" aria-label="progress">
        {steps.map((s, j) => <li key={s} aria-current={j === i ? 'step' : undefined} className={j < i ? 'done' : ''}>{j + 1}</li>)}
      </ol>
      <section className="panel">
        {step === 'privacy' && (
          <>
            <h2>{t('ob_privacy_title')}</h2>
            <p>{t('ob_privacy')}</p>
            <dl className="facts">
              <dt>{t('privacyRegion')}</dt><dd>{t('privacyRegionValue')}</dd>
              <dt>{t('privacyProviders')}</dt><dd>{t('privacyProvidersValue')}</dd>
            </dl>
            <p className="muted">{t('ob_privacy_outcome')}</p>
            <label className="check">
              <input type="checkbox" checked={agreed} onChange={e => setAgreed(e.target.checked)} />
              <span>{t('ob_agree')}</span>
            </label>
          </>
        )}
        {step === 'login' && (<><h2>{t('ob_login_title')}</h2><LoginPanel /></>)}
        {step === 'mic' && (<><h2>{t('ob_mic_title')}</h2><MicTest /></>)}
        {step === 'prefs' && settings && (
          <>
            <h2>{t('sec_writing')}</h2>
            <Select label={t('speechLanguage')} value={String(settings.language ?? 'auto')}
              onChange={v => setSetting('language', v)}
              options={[['auto', t('autoDetect')], ...SPEECH_LANGUAGES]} />
            <Select label={t('mode')} value={String(settings.mode ?? 'standard')} onChange={v => setSetting('mode', v)}
              help={settings.mode === 'polished' ? t('polishedWarn') : undefined}
              options={[['raw', t('mode_raw')], ['standard', t('mode_standard')], ['polished', t('mode_polished')]]} />
            {has(settings, 'auto_insert') && (
              <Toggle label={t('autoInsert')} checked={settings.auto_insert === true} risk={t('autoInsertRisk')}
                onChange={v => setSetting('auto_insert', v)} />
            )}
            {has(settings, 'show_hud') && status?.hudSupported && (
              <Toggle label={t('showHud')} checked={settings.show_hud === true} onChange={v => setSetting('show_hud', v)} />
            )}
            {has(settings, 'start_at_login') && (
              <Toggle label={backend.isWindows ? t('startAtLoginWin') : t('startAtLogin')}
                checked={settings.start_at_login === true} onChange={v => setSetting('start_at_login', v)} />
            )}
          </>
        )}
        {step === 'hotkey' && (
          <>
            <h2>{t('ob_hotkey_title')}</h2>
            <p>{backend.isWindows
              ? t('ob_hotkey_body', { hotkey: status?.hotkey || 'Ctrl+Alt+V' })
              : t('ob_hotkey_body_linux')}</p>
            {status?.hotkeyError && <p className="error-text" role="alert">{status.hotkeyError}</p>}
            {backend.isLinux && <LinuxSetup />}
          </>
        )}
      </section>
      <div className="actions">
        {i > 0 && <button type="button" className="soft" onClick={() => setI(i - 1)}>{t('ob_back')}</button>}
        {i < steps.length - 1
          ? <button type="button" disabled={step === 'privacy' && !agreed} onClick={() => setI(i + 1)}>{t('ob_next')}</button>
          : <button type="button" onClick={finish}>{t('ob_finish')}</button>}
      </div>
    </main>
  );
}
