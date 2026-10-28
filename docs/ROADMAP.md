# Development roadmap — Linux-first development; Windows and Linux ship together in v1.0

**Current delivery**: source-level developer alpha. All phases below are work remaining until explicitly marked shipped.

| Phase | Deliverable | State |
|---|---|---|
| 0 | Ubuntu GNOME Wayland: manual clipboard -> Obsidian, keyboard paste | Previously user-verified |
| 1 | GNOME global shortcut -> Obsidian, Browser, repeated insert + Undo | Previously user-verified |
| 2A | PipeWire selected microphone + Groq Whisper API | Previously user-verified |
| 2B | Integrated CLI state machine, multi-language config, grammar flow, browser UI | **Implemented in source; not yet host-verified** |
| 2C | PipeWire device discovery/selection, structured diagnostics, raw-result recovery, clear/discard, cancellation race fixes | **Implemented in alpha.2; unit and simulated-workflow verified** |
| 3 | Rust-native orchestrator / desktop build / focus safety | Not started |
| 4 | X11 + KDE Wayland, Windows native audio/input | Not started |
| 5 | 20-language quality matrix, Obsidian regression, device selection UI | Not started |
| 6 | Accounts, Stripe, server-side ASR proxy, entitlement enforcement | **Skeleton implemented in 0.2.0-alpha.1; not production-deployed** |
| 7 | Signed install packages, crash reporting opt-in, docs, privacy/release QA | Not started |

## Security release gates

- Never ship the development Groq key or allow the paying client to call models with a shared unmetered provider key.
- Correct focus-target validation before auto-paste. When unknown, copy-only mode must be the safe default for production.
- Mitigate clipboard ownership/rich clipboard type loss before claiming transparent clipboard preservation.
- Document 20-language quality individually and mark untested languages appropriately.
- Secure identity, encrypted transport, privacy notice and data retention policies before actual user acquisition.
- Windows native acceptance must take place on Windows, not only on Linux or through compilation in CI.

## Suggested next implementation tickets

1. Replace GNOME shortcut CLI shim with persistent Rust core service and state event bus.
2. Add portal/libei permission diagnostics and no-focus-steal recording overlay.
   *Alpha.2 ships the structured diagnostics and device checks; the overlay remains open.*
3. Add profile selection + microphones dropdown with stable PipeWire node names.
   *Shipped in alpha.2: `scribiva devices`, dashboard picker, system-default fallback.*
4. Unit and host integration coverage of WAV lifecycle / cancellation races / injection failures.
   *Alpha.2 adds mocked engine races, dashboard HTTP tests and device parsing; real-host injection races remain open.*
5. Introduce backend proxy to Groq (with user auth, quota ledger, rate limits, webhook checks).
   *0.2.0-alpha.1 ships the stdlib/SQLite skeleton plus client `service` mode. v1.0 keeps SQLite on a single US server (ADR-011); production deployment remains open.*
6. Design and QA 20-language dataset: 30 utterances per language minimum for smoke coverage, plus code-switch cases.
7. Build Windows adapter and run Obsidian Live Preview / Source Mode on actual Windows.

## Alpha.2 delivery record

- Device discovery parses `pw-dump` nodes and WirePlumber default metadata in one call; `wpctl inspect @DEFAULT_AUDIO_SOURCE@` is the fallback.
- Microphone selection is exposed through `scribiva devices`, `scribiva set mic_node`, the local dashboard and the Tauri command bridge. `mic_node auto` clears both the setting and the legacy `~/.config/scribiva/mic-node` file.
- `doctor` now returns per-check `ok/warn/fail` entries plus a `healthy` flag; the API key value is still never printed.
- Cancelling during ASR skips the grammar call; a result that arrives after cancellation is neither stored nor injected.
- `scribiva discard` deletes retained failed audio; `scribiva last --raw` and `scribiva clear-last` expose/remove both transcript variants.
- Audio stop escalates SIGTERM → SIGKILL only after re-verifying the recorder cmdline.
- Still not host-verified on a real microphone/Obsidian session: see `TEST_REPORT.md`.

## 0.2.0-alpha.1 delivery record

- New `server/scribiva_server/` package (Python stdlib + SQLite): Bearer tokens stored SHA-256-hashed, transactional quota reservation, `(user_id, client_job_id)` idempotency, one in-flight job per user, cancel and result expiry, in-process rate limit, WAV duration validation, text-guard, Groq adapter, Stripe Checkout/Portal calls and signed webhook mapping.
- Client `providers.py` gains a service transport: multipart `POST /v1/dictations`, async polling, server error mapping, and a token stored at `~/.config/scribiva/service-token` (`0600`). `engine.worker` uses the service in one request when `service_url` is set, preserving direct-Groq mode otherwise.
- CLI: `service login|status|logout|billing|portal|delete`; the dashboard shows plan/quota and opens Checkout/Portal links, but never exposes the token. `doctor` distinguishes direct-key and service-token checks.
- Account deletion (`DELETE /v1/me/data`) revokes server tokens and removes jobs/transcripts; `service delete --confirm` also clears local credentials.
- 121 offline tests pass, including client curl → local server integration with a fake provider, signed Stripe events, and billing-entrypoint/error-mapping checks.
- Still open: TLS termination, backups, magic-link login, Tauri billing UI, production Stripe test-mode E2E, and actual US deployment. PostgreSQL/multi-instance rate limits are deferred (ADR-011).

## v1.0 implementation status (2026-10-29)

Built and covered by automated tests; not yet verified on real hardware unless noted.

- Desktop (Rust + Tauri 2, one core for Windows and Linux): device login, dictation with FLAC upload, silence trim, local mic-signal and no-speech checks, focus guard, GNOME in-compositor insertion, KDE KWin focus script, X11 fallback, recording HUD (Windows/X11), tray, autostart, signed auto-update (inactive until a key is configured), outcome reporting, settings/onboarding/recovery in zh/en.
- GNOME extension with hot-reloadable implementation (no logout after updates). Loaded in a real session by the developer: v1 only.
- Server: auth, quota with month-end grace, dictionary, commands, hallucination flag, OpenCC, Stripe (events ordered, return pages), outcome metrics, backups, deployment files.
- Linux .deb without Python, tested by install/remove in a clean Ubuntu 24.04 container. Windows NSIS built only by CI (not run yet).
- CI and draft-release workflows written and linted; not yet run on GitHub.

Open before launch: real-device QA (Windows, GNOME, KDE, X11), 20-language quality set, Obsidian undo checks, production deployment and Groq check from the US server, Stripe test-mode end to end, code-signing certificate, updater key, legal review, pricing.
