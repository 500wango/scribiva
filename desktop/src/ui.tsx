// Shared app context and small accessible building blocks.
import React, { createContext, useContext, useId } from 'react';
import type { Backend, Settings, Status } from './backend';
import { translate, type Key, type UiLang } from './i18n';

export type Ctx = {
  backend: Backend;
  lang: UiLang;
  t: (key: Key, vars?: Record<string, string | number>) => string;
  status: Status | null;
  settings: Settings | null;
  refreshStatus: () => Promise<void>;
  refreshSettings: () => Promise<void>;
  setSetting: (key: string, value: string | number | boolean) => Promise<boolean>;
  report: (e: unknown) => void;
  setLang: (lang: UiLang) => void;
};

export const AppCtx = createContext<Ctx | null>(null);

export function useApp(): Ctx {
  const ctx = useContext(AppCtx);
  if (!ctx) throw new Error('AppCtx missing');
  return ctx;
}

export function makeT(lang: UiLang) {
  return (key: Key, vars?: Record<string, string | number>) => translate(lang, key, vars);
}

/** Errors from Rust arrive as plain strings like "CODE: message". */
export function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  try { return JSON.stringify(e); } catch { return String(e); }
}

export function has(settings: Settings | null, key: string): boolean {
  return !!settings && Object.prototype.hasOwnProperty.call(settings, key);
}

export function Section({ title, children, wide }: { title: string; children: React.ReactNode; wide?: boolean }) {
  const id = useId();
  return (
    <section className={wide ? 'panel wide' : 'panel'} aria-labelledby={id}>
      <h2 id={id}>{title}</h2>
      {children}
    </section>
  );
}

export function Toggle({ label, checked, onChange, risk, disabled }: {
  label: string; checked: boolean; onChange: (v: boolean) => void; risk?: string; disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="field">
      <label className="check" htmlFor={id}>
        <input id={id} type="checkbox" checked={checked} disabled={disabled}
          aria-describedby={risk ? id + '-risk' : undefined}
          onChange={e => onChange(e.target.checked)} />
        <span>{label}</span>
      </label>
      {risk && <p id={id + '-risk'} className="risk">⚠ {risk}</p>}
    </div>
  );
}

export function Select({ label, value, options, onChange, help }: {
  label: string; value: string; options: [string, string][]; onChange: (v: string) => void; help?: string;
}) {
  const id = useId();
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      <select id={id} value={value} aria-describedby={help ? id + '-help' : undefined}
        onChange={e => onChange(e.target.value)}>
        {options.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
      </select>
      {help && <p id={id + '-help'} className="help">{help}</p>}
    </div>
  );
}

/** Status glyphs give each state a distinct shape, so state is not colour-only. */
export const PHASE_GLYPH: Record<string, string> = {
  idle: '○', recording: '●', processing: '◌', injecting: '◌', ready: '✓', focus_changed: '⚠',
  clipboard_only: '⧉', suspected_noise: '?', quota: '⛔', error: '✕', cancelled: '–', unknown: '·',
};
