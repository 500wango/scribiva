// First-run flow (docs/06 §4): value, privacy notice (US + Groq), sign-in,
// microphone test, hotkey. Linux skips the Windows-only steps.
import { useState } from 'react';
import { local } from '../backend';
import { LoginPanel } from './Account';
import { MicTest } from './Settings';
import { useApp } from '../ui';

export function Onboarding({ onDone }: { onDone: () => void }) {
  const { backend, t, status, setSetting } = useApp();
  const steps = ['privacy', 'login', ...(backend.isWindows ? ['mic', 'hotkey'] : ['hotkey'])] as const;
  const [i, setI] = useState(0);
  const [agreed, setAgreed] = useState(false);
  const step = steps[i];

  async function finish() {
    local.set('onboarding_done', '1');
    if (backend.isWindows) await setSetting('onboarding_done', true);
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
            <label className="check">
              <input type="checkbox" checked={agreed} onChange={e => setAgreed(e.target.checked)} />
              <span>{t('ob_agree')}</span>
            </label>
          </>
        )}
        {step === 'login' && (<><h2>{t('ob_login_title')}</h2><LoginPanel /></>)}
        {step === 'mic' && (<><h2>{t('ob_mic_title')}</h2><MicTest /></>)}
        {step === 'hotkey' && (
          <>
            <h2>{t('ob_hotkey_title')}</h2>
            <p>{t('ob_hotkey_body', { hotkey: status?.hotkey || 'Ctrl+Alt+V' })}</p>
            {status?.hotkeyError && <p className="error-text" role="alert">{status.hotkeyError}</p>}
            {!backend.isWindows && <p className="muted">{t('ob_linux_hint')}</p>}
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
