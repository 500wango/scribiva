# Scribiva Tauri 2 desktop shell — scaffold

This is a **code scaffold**, not a tested native package. `rustfmt --check` passes, but this workspace does not install `node_modules` or the Tauri system dependencies, so neither `cargo build` nor a real Tauri window was executed.

Current Tauri commands securely invoke the installed `~/.local/bin/scribiva` core on **Linux only**. The bridge covers status, doctor checks, settings (including the optional `service_url`), device enumeration, last-result retrieval/clearing, and recording actions. The service Bearer token is never exposed to the webview; configure it with `scribiva service login`. Windows UI can eventually reuse this frontend, but its native capture, global hotkey, clipboard and focus layers still need to be implemented.

## Local development on Ubuntu

First install the Python core from repository root:

```bash
bash scripts/install-ubuntu.sh
```

Then install Tauri prerequisites from https://v2.tauri.app/start/prerequisites/, install Rust (`rustup`), and in `desktop/` run:

```bash
npm install
npm run tauri dev
```

`bundle.active` is false to prevent accidentally producing an installer that assumes the Python core was bundled. Production packaging must ship a native service or explicitly package and authorize the sidecar.

## Backend boundary

Rust code only invokes a fixed, known CLI executable; it does not accept arbitrary command paths or shell strings. Future migration should replace that CLI bridge with a native Rust orchestration layer and implement Windows-specific platform adapters.
