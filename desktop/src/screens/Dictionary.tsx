// Personal dictionary editor (FR-018): add, remove, import, export, save.
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import type { Dictionary } from '../backend';
import { Section, useApp } from '../ui';

export function DictionaryScreen() {
  const { backend, t, status, report } = useApp();
  const [dict, setDict] = useState<Dictionary | null>(null);
  const [terms, setTerms] = useState<string[]>([]);
  const [draft, setDraft] = useState('');
  const [dirty, setDirty] = useState(false);
  const [saved, setSaved] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const inputId = useId();

  const load = useCallback(async () => {
    try {
      const d = await backend.dictionary();
      setDict(d); setTerms(d.terms); setDirty(false);
    } catch (e) { report(e); }
  }, [backend, report]);
  useEffect(() => { if (status?.signedIn) load(); }, [load, status?.signedIn]);

  if (!status?.signedIn) {
    return <main className="grid"><Section title={t('nav_dictionary')} wide><p className="muted">{t('dictNeedsLogin')}</p></Section></main>;
  }
  const max = dict?.max_terms ?? 200;
  const maxLen = dict?.max_term_length ?? 64;

  function add(raw: string) {
    const items = raw.split(/[\n,;]/).map(s => s.trim()).filter(Boolean).map(s => s.slice(0, maxLen));
    if (!items.length) return;
    const next = [...terms];
    for (const it of items) if (!next.some(x => x.toLowerCase() === it.toLowerCase())) next.push(it);
    setTerms(next.slice(0, max)); setDirty(true); setSaved(false); setDraft('');
  }
  function remove(i: number) {
    setTerms(terms.filter((_, j) => j !== i)); setDirty(true); setSaved(false);
  }
  async function save() {
    try {
      const d = await backend.setDictionary(terms);
      setDict(d); setTerms(d.terms); setDirty(false); setSaved(true);
    } catch (e) { report(e); }
  }
  function exportFile() {
    const blob = new Blob([terms.join('\n') + '\n'], { type: 'text/plain' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = 'scribiva-dictionary.txt';
    a.click();
    URL.revokeObjectURL(a.href);
  }
  async function importFile(f: File | undefined) {
    if (!f || f.size > 64 * 1024) return;
    add(await f.text());
  }

  return (
    <main className="grid">
      <Section title={t('nav_dictionary')} wide>
        <p className="muted">{t('dictHelp')}</p>
        <div className="field inline">
          <label htmlFor={inputId}>{t('dictAdd')}</label>
          <input id={inputId} type="text" value={draft} maxLength={maxLen * 4}
            onChange={e => setDraft(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') add(draft); }} />
          <button type="button" onClick={() => add(draft)} disabled={!draft.trim() || terms.length >= max}>{t('dictAdd')}</button>
        </div>
        <p className="muted small" aria-live="polite">{t('dictCount', { n: terms.length, max })}</p>
        <ul className="terms">
          {terms.map((term, i) => (
            <li key={term}>
              <span dir="auto">{term}</span>
              <button type="button" className="soft small" aria-label={`${t('dictRemove')}: ${term}`}
                onClick={() => remove(i)}>×</button>
            </li>
          ))}
        </ul>
        <div className="actions">
          <button type="button" onClick={save} disabled={!dirty}>{t('dictSave')}</button>
          <button type="button" className="soft" onClick={() => fileRef.current?.click()}>{t('dictImport')}</button>
          <button type="button" className="soft" onClick={exportFile} disabled={!terms.length}>{t('dictExport')}</button>
          <button type="button" className="soft" onClick={load}>{t('refresh')}</button>
          <input ref={fileRef} type="file" accept=".txt,text/plain" hidden aria-hidden="true"
            onChange={e => { importFile(e.target.files?.[0]); e.target.value = ''; }} />
        </div>
        {saved && <p className="success" role="status">{t('saved')}</p>}
      </Section>
    </main>
  );
}
