// Account & billing (FR-010, FR-019): device login, plan, usage with local
// reset time, Stripe checkout/portal (opened by Rust after host check), and
// server data deletion with typed confirmation.
import { useCallback, useEffect, useId, useState } from 'react';
import type { Account } from '../backend';
import { formatLocal, formatSeconds } from '../i18n';
import { errorText, Section, useApp } from '../ui';

export function LoginPanel() {
  const { backend, t, status, report, refreshStatus } = useApp();
  const id = useId();
  const [url, setUrl] = useState(status?.serviceUrl || '');
  useEffect(() => { if (status?.serviceUrl) setUrl(status.serviceUrl); }, [status?.serviceUrl]);

  async function start() {
    try {
      await backend.login(url.trim());
      await refreshStatus();
    } catch (e) { report(e); }
  }

  const login = status?.login;
  return (
    <div>
      <div className="field">
        <label htmlFor={id}>{t('serviceUrl')}</label>
        <input id={id} type="url" value={url} placeholder="https://api.example.com" spellCheck={false}
          onChange={e => setUrl(e.target.value)} onKeyDown={e => { if (e.key === 'Enter') start(); }} />
      </div>
      <div className="actions">
        <button type="button" onClick={start} disabled={!url.trim() || !!login}>{t('signIn')}</button>
        {login && <button type="button" className="soft" onClick={() => backend.cancelLogin().then(refreshStatus)}>{t('cancel')}</button>}
      </div>
      {login && (
        <div className="callout" role="status" aria-live="polite">
          <p>{t('loginOpened')}</p>
          <p>{t('loginCode')}: <b className="code">{login.user_code}</b></p>
          <p className="muted small">{login.verification_uri}</p>
          <p className={login.error ? 'error-text' : 'muted'}>
            {login.error ? (login.error === 'EXPIRED_TOKEN' ? t('loginExpired') : login.error) : t('loginWaiting')}
          </p>
        </div>
      )}
    </div>
  );
}

export function AccountScreen() {
  const { backend, t, lang, status, report, refreshStatus } = useApp();
  const [account, setAccount] = useState<Account | null>(null);
  const [confirm, setConfirm] = useState('');
  const [deleted, setDeleted] = useState(false);
  const [loadError, setLoadError] = useState('');
  const confirmId = useId();

  const load = useCallback(async () => {
    try { setLoadError(''); setAccount(await backend.account(true)); }
    catch (e) { setLoadError(errorText(e)); }
  }, [backend]);
  useEffect(() => { if (status?.signedIn) load(); else setAccount(null); }, [load, status?.signedIn]);
  useEffect(() => {
    let off = () => {};
    backend.onAccount(a => setAccount(a)).then(f => { off = f; });
    return () => off();
  }, [backend]);

  async function billing(portal: boolean, plan: 'monthly' | 'annual' = 'monthly') {
    try { await backend.openBilling(portal, plan); } catch (e) { report(e); }
  }
  async function del() {
    try {
      await backend.deleteData(confirm);
      setDeleted(true); setConfirm(''); setAccount(null);
      await refreshStatus();
    } catch (e) { report(e); }
  }
  async function signOut() {
    try { await backend.logout(); setAccount(null); await refreshStatus(); } catch (e) { report(e); }
  }

  const u = account?.usage;
  const pct = u && u.limit_seconds > 0 ? Math.min(100, Math.round((u.used_seconds / u.limit_seconds) * 100)) : 0;
  const exhausted = !!u && u.limit_seconds > 0 && u.remaining_seconds <= 0;

  return (
    <main className="grid">
      <Section title={t('nav_account')}>
        {status?.signedIn ? (
          <>
            <p>{t('signedInAs')}: <b>{account?.user?.email || '…'}</b></p>
            <p className="muted small">{status.serviceUrl}</p>
            <div className="actions">
              <button type="button" className="soft" onClick={load}>{t('refresh')}</button>
              <button type="button" className="soft" onClick={signOut}>{t('signOut')}</button>
            </div>
          </>
        ) : <LoginPanel />}
        {loadError && <p className="error-text">{loadError}</p>}
        {deleted && <p className="success" role="status">{t('deleteDone')}</p>}
      </Section>

      {status?.signedIn && account && (
        <Section title={t('plan')}>
          <p className="statusName">{(account.plan || 'free').toUpperCase()}
            {account.subscription?.status && account.subscription.status !== 'inactive' ? ` · ${account.subscription.status}` : ''}</p>
          {u && (
            <>
              <div className="meter-row">
                <meter min={0} max={100} value={pct} aria-label={t('usage')} low={70} high={90} optimum={0} />
                <span>{pct}%</span>
              </div>
              <dl className="facts">
                <dt>{t('usage')}</dt><dd>{formatSeconds(u.used_seconds, lang)} / {formatSeconds(u.limit_seconds, lang)}</dd>
                <dt>{t('remaining')}</dt><dd>{formatSeconds(u.remaining_seconds, lang)}{exhausted ? ' ⛔' : ''}</dd>
                <dt>{t('resetsAt')}</dt><dd>{formatLocal(u.resets_at, lang)}</dd>
                {!!u.grace_seconds && <><dt>{t('grace')}</dt><dd>{formatSeconds(u.grace_seconds, lang)}</dd></>}
              </dl>
              {exhausted && <p className="notice-text">{t('quotaHint', { date: formatLocal(u.resets_at, lang) })}</p>}
            </>
          )}
          {account.billing_configured !== false && (
            <>
              <div className="actions">
                {account.plan !== 'pro' && <>
                  <button type="button" onClick={() => billing(false, 'monthly')}>{t('upgradeMonthly')}</button>
                  <button type="button" className="soft" onClick={() => billing(false, 'annual')}>{t('upgradeAnnual')}</button>
                </>}
                <button type="button" className="soft" onClick={() => billing(true)}>{t('manage')}</button>
              </div>
              <p className="muted small">{t('billingNote')}</p>
            </>
          )}
        </Section>
      )}

      {status?.signedIn && (
        <Section title={t('deleteData')} wide>
          <div className="field">
            <label htmlFor={confirmId}>{t('deleteConfirm')}</label>
            <input id={confirmId} type="text" value={confirm} autoComplete="off" spellCheck={false}
              onChange={e => setConfirm(e.target.value)} />
          </div>
          <button type="button" className="danger" disabled={confirm !== 'DELETE'} onClick={del}>{t('deleteData')}</button>
        </Section>
      )}
    </main>
  );
}
