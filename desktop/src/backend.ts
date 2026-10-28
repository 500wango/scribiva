// Bridge to the native Rust core. Windows and Linux expose the same commands
// and push `status` / `account` events; `platform` is only used for UI that is
// genuinely platform specific (Linux GNOME setup, Windows hotkey field and Esc).
// Tokens never reach the webview.
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
  /** Display form of the toggle shortcut; empty on Linux when none is set up. */
  hotkey?: string;
  hotkeyError?: string | null;
  signedIn: boolean;
  serviceUrl: string;
  login?: LoginView | null;
};

export type Settings = Record<string, string | number | boolean>;
export type Device = { id: string; label: string; isDefault: boolean };
export type Devices = { devices: Device[]; error?: string | null; active?: string | null };
export type Last = { available: boolean; text: string; raw: string; warnings: string[]; suspectedNoise: boolean; outcome?: string | null };
export type Usage = { used_seconds: number; reserved_seconds?: number; limit_seconds: number; remaining_seconds: number; resets_at?: string | null; grace_seconds?: number | null };
export type Account = { user?: { email?: string | null }; plan?: string | null; subscription?: { status?: string | null; current_period_end?: string | null }; usage: Usage; billing_configured?: boolean };
export type Dictionary = { terms: string[]; max_terms: number; max_term_length: number };
export type Check = { id: string; status: 'ok' | 'warn' | 'fail'; detail: string };
export type Diagnostics = {
  session?: string; desktop?: string; mic_node?: string | null; service_url?: string;
  token_storage?: string; checks: Check[];
};

export async function detectPlatform(): Promise<Platform> {
  try {
    return await invoke<Platform>('platform');
  } catch {
    return 'unsupported';
  }
}

type RawStatus = {
  phase: Phase; error: ErrorInfo | null; notice: string | null; can_retry: boolean;
  recording_started_ms: number | null; max_record_seconds: number; signed_in: boolean;
  service_url: string; hotkey: string; hotkey_error: string | null; login: LoginView | null;
};

function fromRaw(s: RawStatus): Status {
  return {
    phase: s.phase, error: s.error, notice: s.notice, canRetry: s.can_retry,
    recordingStartedMs: s.recording_started_ms, maxRecordSeconds: s.max_record_seconds,
    hotkey: s.hotkey, hotkeyError: s.hotkey_error, signedIn: s.signed_in,
    serviceUrl: s.service_url, login: s.login,
  };
}

export class Backend {
  constructor(readonly platform: Platform) {}

  get isWindows() { return this.platform === 'windows'; }
  get isLinux() { return this.platform === 'linux'; }

  async status(): Promise<Status> {
    return fromRaw(await invoke<RawStatus>('status'));
  }

  async onStatus(cb: (s: Status) => void): Promise<UnlistenFn> {
    return listen<RawStatus>('status', e => cb(fromRaw(e.payload)));
  }

  async onAccount(cb: (a: Account) => void): Promise<UnlistenFn> {
    return listen<Account>('account', e => cb(e.payload));
  }

  async settings(): Promise<Settings> {
    return invoke<Settings>('settings');
  }

  async set(key: string, value: string | number | boolean): Promise<void> {
    await invoke('set_setting', { key, value: String(value) });
  }

  async devices(): Promise<Devices> {
    const d = await invoke<{ devices: { id: string; name: string; default: boolean; virtual: boolean }[]; active?: string | null; error?: string | null }>('devices');
    return {
      devices: d.devices.map(x => ({ id: x.id, label: x.name + (x.virtual ? ' · virtual' : ''), isDefault: x.default })),
      active: d.active, error: d.error,
    };
  }

  async micTest(): Promise<{ peak: number; rms: number } | null> {
    return invoke('mic_test');
  }

  async last(): Promise<Last> {
    const l = await invoke<{ available: boolean; text?: string; raw?: string; warnings?: string[]; suspected_noise?: boolean; outcome?: string | null }>('last');
    return { available: l.available, text: l.text || '', raw: l.raw || '', warnings: l.warnings || [], suspectedNoise: Boolean(l.suspected_noise), outcome: l.outcome };
  }

  async action(name: 'toggle' | 'cancel' | 'retry' | 'discard' | 'copy-last' | 'copy-raw' | 'clear-last' | 'acknowledge'): Promise<void> {
    await invoke('action', { name });
  }

  async login(url: string): Promise<LoginView> {
    return invoke<LoginView>('login', { url });
  }

  async cancelLogin(): Promise<void> {
    await invoke('cancel_login');
  }

  async logout(): Promise<void> {
    await invoke('logout');
  }

  async account(refresh = true): Promise<Account | null> {
    return invoke<Account | null>('account', { refresh });
  }

  async openBilling(portal: boolean, plan: 'monthly' | 'annual' = 'monthly'): Promise<void> {
    await invoke('open_billing', { portal, plan });
  }

  async dictionary(): Promise<Dictionary> {
    return invoke<Dictionary>('dictionary');
  }

  async setDictionary(terms: string[]): Promise<Dictionary> {
    return invoke<Dictionary>('set_dictionary', { terms });
  }

  async deleteData(confirm: string): Promise<void> {
    await invoke('delete_data', { confirm });
  }

  async diagnostics(): Promise<Diagnostics> {
    return invoke<Diagnostics>('diagnostics');
  }

  /** Linux only: GNOME shortcut and extension state. */
  async linuxSetupStatus(): Promise<LinuxSetup> {
    return invoke<LinuxSetup>('setup_status');
  }

  async linuxSetup(action: 'hotkey' | 'extension', binding?: string): Promise<LinuxSetupResult> {
    return invoke<LinuxSetupResult>('setup', { action, binding: binding ?? null });
  }
}

/** Per-viewer UI preferences (interface language before settings load). */
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
  /** Command the shortcut runs, for manual setup on other desktops. */
  command: string;
  hotkey: { available: boolean; installed: boolean; broken?: boolean; binding?: string; label?: string };
  extension: { installed: boolean; enabled: boolean; active: boolean; version: number; update_at_next_login?: boolean };
};
export type LinuxSetupResult = {
  status: LinuxSetup;
  hotkey?: { binding: string; command: string; replaced: string[] };
  extension?: { installed: boolean; enabled: boolean; active: boolean; version: number; needs_relogin?: boolean };
};
