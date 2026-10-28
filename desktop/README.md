# Scribiva desktop (Tauri 2)

One Tauri 2 + React app for both platforms:

- **Linux**: the Rust shell calls the installed Python CLI (`~/.local/bin/scribiva`) through a fixed whitelist (`src-tauri/src/linux.rs`). The Linux core itself is not reimplemented here.
- **Windows**: native Rust core, no Python (`src-tauri/src/windows/`).

**Status: not run on real Windows.** The Windows code compiles and passes clippy for `x86_64-pc-windows-msvc`, but it has not been linked, installed or exercised on a Windows machine. Hotkey, WASAPI capture, focus/UIPI detection, clipboard restore and paste behaviour all need real-device QA (docs/08) before anyone relies on them.

## Layout

| Path | Platform | Purpose |
|---|---|---|
| `src-tauri/src/core.rs` | all | Dictation state machine (docs/02 §5). One job id per recording; late results for cancelled or superseded jobs are ignored. Last result kept in memory only. |
| `src-tauri/src/audio_dsp.rs` | all | Downmix, resample to 16 kHz mono, i16, WAV encode, < 1 s drop |
| `src-tauri/src/focus_policy.rs` | all | Focus comparison, terminal/shell/elevated rules (ADR-012) |
| `src-tauri/src/api.rs` | all + Windows client | URL validation, §8 response parsing, error mapping; `reqwest` client on Windows |
| `src-tauri/src/storage.rs` | all + Windows IO | Settings model/validation, hotkey normalisation; settings JSON and Credential Manager token on Windows |
| `src-tauri/src/commands.rs` | per OS | Webview command whitelist (`get_*`/`linux_*` on Linux, `win_*` on Windows) |
| `src-tauri/src/linux.rs` | Linux | CLI bridge |
| `src-tauri/src/windows/` | Windows | `app` (orchestrator), `hotkey`, `audio` (cpal), `focus`, `inject` (SendInput), `clipboard`, `tray` |
| `src/` | all | UI: onboarding, status/recovery, settings, dictionary, account & billing; `i18n.ts` (en / zh-CN) |

## Windows behaviour summary

- Toggle hotkey (default `Ctrl+Alt+V`) via `tauri-plugin-global-shortcut`; registration failure surfaces as `HOTKEY_CONFLICT`. Esc is registered globally only while recording.
- Capture only while recording; max duration auto-stops; < 1 s is dropped; cancel discards without upload.
- At stop: snapshot of foreground HWND + pid + exe name + integrity level (no window titles). Before paste: compare again. Changed → `focus_changed`; terminal/shell/elevated/own window/unknown → copy only.
- Paste: `CF_UNICODETEXT` + one `Ctrl+V` via `SendInput` → `dispatched_unverified`. No retry, no Enter. Previous clipboard restored only if it was plain text and `GetClipboardSequenceNumber` is unchanged after ~800 ms.
- `suspected_hallucination` (or a `SUSPECTED_HALLUCINATION` warning) → never inserted; kept for review as `suspected_noise`.
- Device login (§8.1) opens the system browser; the `scr_` token goes to Windows Credential Manager (`keyring`) and never to the webview or a file.
- Stripe URLs are opened only for `https://checkout.stripe.com` / `https://billing.stripe.com`.
- Window hides to tray on close; the app never shows or focuses its window when recording starts.

## Build

```bash
npm install
npm run build              # tsc + vite
cd src-tauri
cargo check && cargo clippy --all-targets -- -D warnings && cargo test
```

Windows from Linux (check only; linking needs a Windows host): `cargo check --target x86_64-pc-windows-msvc` needs a C compiler with MSVC headers for `ring` and `llvm-rc` for the resource file. With [cargo-xwin](https://github.com/rust-cross/cargo-xwin) and LLVM on PATH:

```bash
PATH=/usr/lib/llvm-17/bin:$PATH cargo xwin check --target x86_64-pc-windows-msvc
```

TLS uses rustls with the `ring` provider (not reqwest's default `aws-lc-rs`, whose C build does not cross-compile here) and the platform certificate verifier. No OpenSSL.

## Packaging

`bundle.active` is on: Windows `nsis` (per-user install, no admin), Linux `deb`. The `.deb` does not bundle the Python core; Linux users still install it with `scripts/install-ubuntu.sh`.

**Code signing is not configured.** `bundle.windows.certificateThumbprint` is `null` and `timestampUrl` is empty on purpose. Before release, set them (or `signCommand`) for the real certificate; unsigned installers trigger SmartScreen and fail FR-012.
