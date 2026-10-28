// App shell: platform detection, shared state, navigation between the
// screens from docs/06 §2. State is pushed by Rust `status` events on both
// platforms, with a slow poll as a safety net.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Backend, detectPlatform, local, type Settings, type Status } from './backend';
import { type Key, type UiLang } from './i18n';
import { AccountScreen } from './screens/Account';
import { DictionaryScreen } from './screens/Dictionary';
import { Onboarding } from './screens/Onboarding';
import { SettingsScreen } from './screens/Settings';
import { StatusScreen } from './screens/Status';
import { AppCtx, errorText, makeT, PHASE_GLYPH, type Ctx } from './ui';
import './style.css';

type Screen = 'status' | 'settings' | 'dictionary' | 'account' | 'onboarding';
const NAV: [Screen, Key][] = [
  ['status', 'nav_status'], ['settings', 'nav_settings'], ['dictionary', 'nav_dictionary'],
  ['account', 'nav_account'], ['onboarding', 'nav_onboarding'],
];

function initialLang(): UiLang {
  const saved = local.get('ui_language');
  if (saved === 'en' || saved === 'zh') return saved;
  return navigator.language.toLowerCase().startsWith('zh') ? 'zh' : 'en';
}

function App({ backend }: { backend: Backend }) {
  const [lang, setLangState] = useState<UiLang>(initialLang);
  const [status, setStatus] = useState<Status | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState('');
  const [screen, setScreen] = useState<Screen>('status');
  const decided = useRef(false);
  const t = useMemo(() => makeT(lang), [lang]);

  const report = useCallback((e: unknown) => setError(errorText(e)), []);
  const refreshStatus = useCallback(async () => {
    try { setStatus(await backend.status()); } catch (e) { report(e); }
  }, [backend, report]);
  const refreshSettings = useCallback(async () => {
    try {
      const s = await backend.settings();
      setSettings(s);
      if (s.ui_language === 'en' || s.ui_language === 'zh') setLangState(s.ui_language);
    } catch (e) { report(e); }
  }, [backend, report]);
  const setSetting = useCallback(async (key: string, value: string | number | boolean) => {
    try {
      setError('');
      await backend.set(key, value);
      await refreshSettings();
      return true;
    } catch (e) { report(e); await refreshSettings(); return false; }
  }, [backend, refreshSettings, report]);
  const setLang = useCallback((l: UiLang) => {
    setLangState(l);
    local.set('ui_language', l);
    backend.set('ui_language', l).catch(report);
  }, [backend, report]);

  useEffect(() => {
    refreshStatus(); refreshSettings();
    let off = () => {};
    backend.onStatus(setStatus).then(f => { off = f; });
    // Events carry every change; a slow poll keeps the view honest if one was missed.
    const id = window.setInterval(refreshStatus, 5000);
    return () => { off(); window.clearInterval(id); };
  }, [backend, refreshStatus, refreshSettings]);

  // First run → onboarding (stored in settings).
  useEffect(() => {
    if (decided.current || !settings) return;
    decided.current = true;
    if (settings.onboarding_done !== true) setScreen('onboarding');
  }, [settings]);

  // Esc cancels an active recording while the window is focused (docs/06 §9).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && (status?.phase === 'recording' || status?.phase === 'processing')) {
        backend.action('cancel').then(refreshStatus).catch(report);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [backend, status?.phase, refreshStatus, report]);

  useEffect(() => { document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en'; }, [lang]);

  const ctx: Ctx = { backend, lang, t, status, settings, refreshStatus, refreshSettings, setSetting, report, setLang };
  const phase = status?.phase || 'unknown';

  return (
    <AppCtx.Provider value={ctx}>
      <div className="page">
        <a className="skip" href="#content">Skip to content</a>
        <header>
          <div className="brand">
            <div className="logo" aria-hidden="true">S.</div>
            <div><b>scribiva.</b><small>{t('appTagline')}</small></div>
          </div>
          <div className="header-right">
            <span className={`chip ${phase}`} role="status" aria-live="polite">
              <span aria-hidden="true">{PHASE_GLYPH[phase] || '·'}</span> {t(`phase_${phase}` as Key) || phase}
            </span>
            <label className="sr-only" htmlFor="ui-lang">{t('uiLanguage')}</label>
            <select id="ui-lang" className="compact" value={lang} onChange={e => setLang(e.target.value as UiLang)}>
              <option value="en">English</option>
              <option value="zh">简体中文</option>
            </select>
          </div>
        </header>
        <nav aria-label="Main">
          {NAV.map(([id, key]) => (
            <button key={id} type="button" className={screen === id ? 'tab active' : 'tab'}
              aria-current={screen === id ? 'page' : undefined} onClick={() => setScreen(id)}>
              {t(key)}
            </button>
          ))}
        </nav>
        <div id="content" tabIndex={-1}>
          {screen === 'status' && <StatusScreen onOpen={setScreen} />}
          {screen === 'settings' && <SettingsScreen />}
          {screen === 'dictionary' && <DictionaryScreen />}
          {screen === 'account' && <AccountScreen />}
          {screen === 'onboarding' && <Onboarding onDone={() => setScreen('status')} />}
        </div>
        {!!error && (
          <aside role="alert" className="alert">
            <span>{t('error')}: </span><code>{error}</code>
            <button type="button" className="soft small" onClick={() => navigator.clipboard?.writeText(error)}>{t('copy')}</button>
            <button type="button" className="soft small" onClick={() => setError('')}>{t('dismiss')}</button>
          </aside>
        )}
        {backend.platform === 'unsupported' && <p className="error-text">Unsupported platform</p>}
      </div>
    </AppCtx.Provider>
  );
}

detectPlatform().then(platform => {
  createRoot(document.getElementById('root')!).render(
    <React.StrictMode><App backend={new Backend(platform)} /></React.StrictMode>,
  );
});
