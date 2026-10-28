// Platform bridge. Windows talks to the native Rust core (win_* commands and
// `status` / `account` events). Linux talks to the Python CLI through the
// whitelisted commands in src-tauri/src/linux.rs. The UI only sees the
// unified types below. Tokens never reach the webview on either platform.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type Platform = 'windows' | 'linux' | 'unsupported';

export type Phase =
  | 'idle' | 'recording' | 'processing' | 'injecting' | 'ready' | 'focus_changed'
  | 'clipboard_only' | 'suspected_noise' | 'quota' | 'error' | 'cancelled' | 'unknown';

export type ErrorInfo = { code: string; message: string; class?: string; retryable?: boolean };
export type LoginView = { user_code: string; verification_uri: string; expires_at_ms: number; error?: string | null };

export type Status = {
  phase: Phase;
  error?: ErrorInfo | null;
  notice?: string | null;
  canRetry: boolean;
  recordingStartedMs?: number | null;
  maxRecordSeconds?: number;
  lastOutcome?: string | null;
  hotkey?: string;
  hotkeyError?: string | null;
  signedIn: boolean;
  serviceUrl: string;
  login?: LoginView | null;
  diagnostics?: Record<string, unknown>;
};

export type Settings = Record<string, string | number | boolean>;
export type Device = { id: string; label: string; isDefault: boolean };
export type Devices = { devices: Device[]; error?: string | null; active?: string | null };
export type Last = { available: boolean; text: string; raw: string; warnings: string[]; suspectedNoise: boolean; outcome?: string | null };
export type Usage = { used_seconds: number; reserved_seconds?: number; limit_seconds: number; remaining_seconds: number; resets_at?: string | null; grace_seconds?: number | null };
export type Account = { user?: { email?: string | null }; plan?: string | null; subscription?: { status?: string | null; current_period_end?: string | null }; usage: Usage; billing_configured?: boolean };
export type Dictionary = { terms: string[]; max_terms: number; max_term_length: number };

/** Keys that only exist on one platform; the UI checks presence in Settings. */
export const MIC_KEY: Record<Platform, string> = { windows: 'mic_device', linux: 'mic_node', unsupported: 'mic_device' };

export async function detectPlatform(): Promise<Platform> {
  try {
    return await invoke<Platform>('platform');
  } catch {
    return 'unsupported';
  }
}

type WinStatus = {
  phase: Phase; error: ErrorInfo | null; notice: string | null; can_retry: boolean;
  recording_started_ms: number | null; max_record_seconds: number; signed_in: boolean;
  service_url: string; hotkey: string; hotkey_error: string | null; login: LoginView | null;
};

function fromWin(s: WinStatus): Status {
  return {
    phase: s.phase, error: s.error, notice: s.notice, canRetry: s.can_retry,
    recordingStartedMs: s.recording_started_ms, maxRecordSeconds: s.max_record_seconds,
    hotkey: s.hotkey, hotkeyError: s.hotkey_error, signedIn: s.signed_in,
    serviceUrl: s.service_url, login: s.login,
  };
}

type LinuxStatus = {
  state: { phase?: string; error?: string; last_result?: string; started?: number };
  doctor: Record<string, unknown> & { service_url?: string; service_mode?: boolean; has_service_token?: boolean };
};

function fromLinux(s: LinuxStatus): Status {
  const phase = (s.state.phase || 'unknown') as Phase;
  const started = typeof s.state.started === 'number' ? s.state.started * 1000 : null;
  return {
    phase,
    error: s.state.error ? { code: 'ERROR', message: s.state.error } : null,
    canRetry: phase === 'error',
    recordingStartedMs: phase === 'recording' ? started : null,
    lastOutcome: s.state.last_result || null,
    signedIn: Boolean(s.doctor.service_mode && s.doctor.has_service_token),
    serviceUrl: String(s.doctor.service_url || ''),
    diagnostics: s.doctor,
  };
}

export class Backend {
  constructor(readonly platform: Platform) {}

  get isWindows() { return this.platform === 'windows'; }

  async status(): Promise<Status> {
    if (this.isWindows) return fromWin(await invoke<WinStatus>('win_status'));
    return fromLinux(await invoke<LinuxStatus>('get_status'));
  }

  /** Windows pushes status changes; Linux is polled by the caller. */
  async onStatus(cb: (s: Status) => void): Promise<UnlistenFn> {
    if (!this.isWindows) return () => {};
    return listen<WinStatus>('status', e => cb(fromWin(e.payload)));
  }

  async onAccount(cb: (a: Account) => void): Promise<UnlistenFn> {
    if (!this.isWindows) return () => {};
    return listen<Account>('account', e => cb(e.payload));
  }

  async settings(): Promise<Settings> {
    return invoke<Settings>(this.isWindows ? 'win_settings' : 'get_settings');
  }

  async set(key: string, value: string | number | boolean): Promise<void> {
    if (this.isWindows) await invoke('win_set_setting', { key, value: String(value) });
    else await invoke('set_setting', { key, value: String(value) });
  }

  async devices(): Promise<Devices> {
    if (this.isWindows) {
      const list = await invoke<{ id: string; name: string; default: boolean }[]>('win_devices');
      return { devices: list.map(d => ({ id: d.id, label: d.name, isDefault: d.default })) };
    }
    const d = await invoke<{ devices: { node: string; label: string; virtual: boolean; default: boolean }[]; active?: string | null; error?: string | null }>('get_devices');
    return {
      devices: (d.devices || []).map(x => ({ id: x.node, label: x.label + (x.virtual ? ' · virtual' : ''), isDefault: x.default })),
      active: d.active, error: d.error,
    };
  }

  async micTest(): Promise<{ peak: number; rms: number } | null> {
    if (!this.isWindows) return null;
    return invoke('win_mic_test');
  }

  async last(): Promise<Last> {
    if (this.isWindows) {
      const l = await invoke<{ available: boolean; text?: string; raw?: string; warnings?: string[]; suspected_noise?: boolean; outcome?: string | null }>('win_last');
      return { available: l.available, text: l.text || '', raw: l.raw || '', warnings: l.warnings || [], suspectedNoise: Boolean(l.suspected_noise), outcome: l.outcome };
    }
    const l = await invoke<{ available: boolean; text: string; raw: string; warning: string; phase: string }>('get_last');
    return { available: l.available, text: l.text || '', raw: l.raw || '', warnings: l.warning ? [l.warning] : [], suspectedNoise: false };
  }

  async action(name: 'toggle' | 'cancel' | 'retry' | 'discard' | 'copy-last' | 'copy-raw' | 'clear-last' | 'acknowledge', last?: Last): Promise<void> {
    if (this.isWindows) {
      await invoke('win_action', { name });
      return;
    }
    if (name === 'acknowledge') return;
    if (name === 'clear-last') { await invoke('clear_last'); return; }
    if (name === 'copy-raw') {
      // The CLI has no raw-copy command; the webview clipboard is enough here.
      await navigator.clipboard.writeText(last?.raw || '');
      return;
    }
    await invoke('action', { name });
  }

  async login(url: string): Promise<LoginView | null> {
    if (this.isWindows) return invoke<LoginView>('win_login', { url });
    await invoke('linux_login', { url });
    return null;
  }

  async onLoginLines(line: (l: string) => void, done: (ok: boolean, error?: string | null) => void): Promise<UnlistenFn> {
    if (this.isWindows) return () => {};
    const a = await listen<string>('login-line', e => line(e.payload));
    const b = await listen<{ ok: boolean; error?: string | null }>('login-done', e => done(e.payload.ok, e.payload.error));
    return () => { a(); b(); };
  }

  async cancelLogin(): Promise<void> {
    if (this.isWindows) await invoke('win_cancel_login');
  }

  async logout(): Promise<void> {
    await invoke(this.isWindows ? 'win_logout' : 'linux_logout');
  }

  async account(refresh = true): Promise<Account | null> {
    if (this.isWindows) return invoke<Account | null>('win_account', { refresh });
    const s = await invoke<{ account?: Account }>('linux_service_status');
    return s.account || null;
  }

  async openBilling(portal: boolean, plan: 'monthly' | 'annual' = 'monthly'): Promise<void> {
    await invoke(this.isWindows ? 'win_open_billing' : 'linux_open_billing', { portal, plan });
  }

  async dictionary(): Promise<Dictionary> {
    return invoke<Dictionary>(this.isWindows ? 'win_dictionary' : 'linux_dictionary');
  }

  async setDictionary(terms: string[]): Promise<Dictionary> {
    return invoke<Dictionary>(this.isWindows ? 'win_set_dictionary' : 'linux_set_dictionary', { terms });
  }

  async deleteData(confirm: string): Promise<void> {
    await invoke(this.isWindows ? 'win_delete_data' : 'linux_delete_data', { confirm });
  }
  /** Linux only: GNOME shortcut and extension state (scribiva setup status). */
  async linuxSetupStatus(): Promise<LinuxSetup> {
    const r = await invoke<{ status?: LinuxSetup } & LinuxSetup>('linux_setup_status');
    return (r.status ?? r) as LinuxSetup;
  }
  async linuxSetup(action: 'hotkey' | 'extension', binding?: string): Promise<LinuxSetupResult> {
    return invoke<LinuxSetupResult>('linux_setup', { action, binding: binding ?? null });
  }

}

/** Per-viewer UI preferences that the Linux CLI does not store. */
export const local = {
  get(key: string): string | null {
    try { return window.localStorage.getItem('scribiva.' + key); } catch { return null; }
  },
  set(key: string, value: string) {
    try { window.localStorage.setItem('scribiva.' + key, value); } catch { /* storage unavailable */ }
  },
};

export type LinuxSetup = {
  session: string;
  gnome: boolean;
  hotkey: { available: boolean; installed: boolean; binding?: string };
  extension: { installed: boolean; enabled: boolean; active: boolean };
};
export type LinuxSetupResult = {
  status: LinuxSetup;
  hotkey?: { binding: string };
  extension?: { enabled: boolean; active: boolean; needs_relogin?: boolean };
};
