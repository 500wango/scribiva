# Scribiva — Linux-first developer alpha

**版本：0.2.0-alpha.1 | 2026-10-26**

This is a **real, runnable Linux dictation core**, not a finished commercial release. Current development target: **Ubuntu 24.04 GNOME Wayland x86_64**, with your previously verified `pw-record + wl-copy + eitype` input path. A stdlib-only **server-side Groq proxy and quota skeleton** is now included; the Windows native adapter, production-ready Stripe deployment, and signed installers are NOT included yet.

## Already implemented

- GNOME custom global shortcut: `Ctrl+Alt+V` toggles recording via a command entrypoint.
- PipeWire **device discovery** with stable `node.name` identifiers (`scribiva devices`), dashboard picker, and automatic system-default fallback.
- Explicit PipeWire microphone node selection (preserves your existing `~/.config/scribiva/mic-node`).
- Unique audio file per recording; guarded process termination with SIGTERM → SIGKILL escalation; auto-stop watchdog (default 180 seconds).
- Groq Whisper Large V3 or Turbo, auto detection + 20 explicit language choices.
- Dictation modes: raw, grammar correction, polished; LLM edits fail closed to raw text.
- GNOME Wayland text injection via `wl-copy` and `eitype` (libei). Unicode, Obsidian Live Preview / Source Mode are key QA cases.
- Last result recovery with both **final and ASR-raw text**, explicit clear, failure-audio discard, cancel, limited retry, local status/config web dashboard.
- Structured `doctor` checks (session, PipeWire, devices, clipboard, injector, key permissions, writable state) with a machine-readable `healthy` flag.
- Developer key stored locally with mode `0600`, never embedded in the program bundle; `curl` sends Authorization through stdin config.
- Optional **subscription service mode**: `scribiva service login --url ...` moves ASR/grammar calls behind the bundled server, stores only a `scr_...` token locally, and keeps the Groq key on the server. The dashboard and CLI show plan/quota and print Stripe Checkout/Portal links.
- Server skeleton: Bearer tokens stored SHA-256-hashed, transactional quota reservation, per-user idempotency, cancel/expiry, rate limiting, signed Stripe webhooks, account-data deletion, and offline tests with mocked providers.

## Install (Ubuntu / Linux Mint)

```bash
sudo apt install ./scribiva_1.0.0_amd64.deb
```

One package contains the desktop app, the dictation core (`scribiva-cli`), the Scribiva GNOME Shell extension and eitype; apt installs the runtime dependencies. Start **Scribiva** from the app menu and follow the setup guide: it binds Ctrl+Alt+V and enables the GNOME extension for your user (log out and back in once). Build the package with the steps in [`desktop/README.md`](desktop/README.md).

## Developer install from source

**Stop any currently active Gate 2B recording first.** From this repository root:

```bash
bash scripts/install-ubuntu.sh
~/.local/bin/scribiva doctor
~/.local/bin/scribiva ui
```

If you already created the Gate 1 GNOME shortcut pointing at `~/.local/bin/scribiva-insert-test`, there is **no need to recreate it**. The installer backs up that script, then replaces it with a shim that calls Scribiva Alpha.

If no shortcut exists, run:

```bash
~/.local/bin/scribiva setup all      # GNOME shortcut + extension
```

Do not execute Scribiva with `sudo`.

### Optional: use the server proxy instead of a local Groq key

```bash
export GROQ_API_KEY='gsk_...'                 # only the server needs this
export SCRIBIVA_DB="$PWD/server/data/scribiva.sqlite3"
export PYTHONPATH=server
python3 -m scribiva_server init-db
python3 -m scribiva_server create-user --email you@example.com   # prints a token once
python3 -m scribiva_server serve --host 127.0.0.1 --port 8787
```

In another terminal:

```bash
scribiva service login --url http://127.0.0.1:8787   # token prompt is hidden
scribiva service status
scribiva service billing --plan monthly               # prints a Stripe Checkout URL
scribiva service portal                               # prints a Stripe Billing Portal URL
scribiva service delete --confirm                     # deletes server data and local token
```

Once `service_url` is set, recordings go to the local server, which enforces the free-plan quota before calling Groq. Remove it with `scribiva service logout`. The local dashboard shows plan, used/reserved/remaining quota and can open Checkout/Portal links, but entitlement is granted only by a signed Stripe webhook. See [`server/README.md`](server/README.md) for API, quota semantics, Stripe variables, privacy deletion, and production limits.

## Typical workflow

1. Open Obsidian in Live Preview or Source Mode and keep the text cursor in the editor.
2. Press **Ctrl+Alt+V** to start recording. A GNOME notification may appear.
3. Speak; press the same shortcut again to stop.
4. The worker uploads the WAV to Groq directly, or through the configured Scribiva service (ASR + optional grammar in one authenticated request), then dispatches a paste command. Both the final text and the pre-edit ASR text are kept locally for recovery.
5. If something fails, read `scribiva status`, `scribiva doctor`, or `tail -n 40 ~/.local/state/scribiva/scribiva.log`. Use `scribiva last`, `scribiva discard`, or `scribiva clear-last` to recover or remove local results.

A dispatched paste **is not proof** that the target app accepted text. GNOME Wayland does not provide a universal focus/clipboard transaction API. This alpha assumes you keep the target editor focused through processing; you can instead disable auto-insert and use manual copy.

## CLI reference

```bash
scribiva ui                           # local settings web console (127.0.0.1 only)
scribiva toggle                       # start or stop recording
scribiva cancel                       # cancel recording / prevent pending processing result
scribiva discard                      # delete retained failed audio or drop a pending result
scribiva retry                        # retry a saved failed recording (may still incur API cost)
scribiva doctor                       # structured environment report
scribiva devices                      # list PipeWire capture sources
scribiva devices --json               # machine-readable device list
scribiva status                       # current stage
scribiva settings                     # settings (never prints the API key)
scribiva languages                    # supported language identifiers
scribiva set language zh              # manual Chinese mode
scribiva set language auto            # auto language recognition
scribiva set mode standard            # raw | standard | polished
scribiva set mode raw                 # no grammar LLM
scribiva set asr_model whisper-large-v3-turbo
scribiva set auto_insert false        # clipboard only; safer for unknown focus
scribiva set mic_node alsa_input.pci-0000_0b_00.4.analog-stereo
scribiva set mic_node auto            # clear the choice; use system default
scribiva key                          # securely enter developer Groq key
scribiva service login --url URL      # authenticate to the quota service (token hidden)
scribiva service status               # server config + plan + usage JSON
scribiva service billing --plan annual # print Stripe Checkout URL (monthly|annual)
scribiva service portal               # print Stripe Billing Portal URL
scribiva service delete --confirm     # delete server jobs/tokens, then clear local token
scribiva service logout               # remove token and return to direct mode
scribiva set service_url URL          # power-user equivalent; token still needs login
scribiva last                         # print last processed text
scribiva last --raw                   # print last ASR text before editing
scribiva copy-last                    # latest result -> clipboard
scribiva paste-last                   # manual re-paste; can duplicate text!
scribiva clear-last                   # delete locally stored transcripts
```

Supported language IDs: `en zh ja ko es fr de pt it ru ar hi tr nl pl id vi th sv uk` plus `auto`.

If previous Groq key is still saved in `~/.config/scribiva/groq-api-key` it is reused. If it has been revoked, run `scribiva key` to replace it. The developer key **must not be shipped to paying users**. The bundled server proxy implements the initial authenticated quota boundary; it is still a single-node SQLite skeleton, not a finished commercial backend.

## Install requirements

```bash
sudo apt install -y python3 pipewire-bin wireplumber wl-clipboard curl libnotify-bin
```

For a source install `eitype` must be installed separately (`cargo install --locked eitype`); the .deb bundles it. The alpha does not install or elevate device drivers. If the GNOME shortcut is missing or conflicts, run `scribiva setup hotkey --binding "<Primary><Alt>space"` (any free combination).

## Repo map

```text
src/scribiva/         Python Linux core (stdlib only)
  devices.py          PipeWire source discovery via pw-dump / wpctl fallback
  providers.py        Direct Groq or authenticated Scribiva service transport
server/scribiva_server/  Groq proxy, SQLite quota/idempotency, Stripe webhook skeleton
web/index.html        Self-contained local dashboard
scripts/              install, uninstall, GNOME optional shortcut
tests/                offline unit/integration tests (mocked providers)
desktop/              Tauri 2 + React scaffolding (NOT compiled here)
docs/                 architecture decisions and release checklist
```

## Tests

```bash
PYTHONPATH=src:server python3 -m unittest discover -s tests -p 'test_*.py' -v
PYTHONPATH=src python3 -m scribiva --help
PYTHONPATH=server python3 -m scribiva_server --help
```

Tests are offline and use mocks. The current suite has **121 tests** covering config, devices, dashboard security, engine races, providers, injection, server auth/quota/idempotency/cancel/expiry/rate-limit/audio/multipart/text-guard, Stripe signature/event mapping, privacy deletion, client/dashboard billing entrypoints, and a real curl client-to-loopback-server integration. They do **not** prove the binary works on your Linux desktop or that Groq will admit API calls; previous Cloudflare 1010 may still require a service-provider support case.

## Privacy / commercial limits

- Audio is sent to Groq directly, or to the self-hosted Scribiva service which forwards it to Groq. In service mode the Groq key exists only on the server; the client stores a revocable Bearer token in `~/.config/scribiva/service-token` with mode `0600`.
- Service mode processes audio in memory (not persisted), stores transcript text only for the result TTL (default 900s), and can delete jobs/tokens with `scribiva service delete --confirm` (`DELETE /v1/me/data`). Billing metadata is retained.
- If mode is not `raw`, recognized text is sent to Groq for editing.
- Local configuration, token, last result, and temporary audio are user-private files. Audio deleted on success by default, retained on failure for retry. `scribiva clear-last` removes the last final/ASR texts, and `scribiva discard` deletes retained failed audio.
- `scribiva doctor` reports device names, key permission mode and environment checks; it never prints the API key or transcript contents.
- Local dashboard only binds `127.0.0.1` with host/origin checks and request token.
- Clipboard input may temporarily overwrite rich clipboard content. The app attempts restoration only for a previously readable UTF-8 text flavor and only if it detects no intervening change; clipboard restoration is best effort.
- No automated focus verification on GNOME Wayland. Never leave the insertion target unclear while dictating sensitive text.
- The bundled server has account tokens, free/Pro quotas, rate limiting and Stripe webhook handling, but **no production payment UI, email verification, multi-instance storage, TLS termination or commercial readiness**. Treat it as a tested skeleton for integration, not a hosted service.

Next: harden the server skeleton into a deployable backend (PostgreSQL, shared rate limits, TLS/proxy, account lifecycle, Stripe end-to-end test mode), replace the Python alpha with a Rust/Tauri native core, build platform adapters for Windows, QA 20 languages, and package/sign installers. See `docs/ROADMAP.md`.

## Existing product specifications

The v1.0 product/engineering specifications live in `docs/` (`docs/README.md` is the index, `docs/01-PRD.md` the PRD). Those documents describe **future commercial requirements**, not features claimed as shipped in this developer alpha.
