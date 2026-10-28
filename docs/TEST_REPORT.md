# Scribiva Alpha verification record

Date: 2026-10-26 · Build: 0.2.0-alpha.1

## Tests run in the delivery environment

| Verification | Result | Coverage |
|---|---|---|
| Python syntax compileall | PASS | All `src/`, `server/` and `tests/` Python sources |
| `unittest` suite | PASS, 121/121 | Client: config/device parsing, legacy mic migration, engine success/cancellation/discard/clear races, service-mode worker, doctor direct/service checks, dashboard HTTP security, protected content, numeric integrity, 403/1010 non-retry, insertion uncertainty, WAV RMS, state transitions. Server: auth expiry/revocation, quota reservation, idempotency, concurrency, cancel race, result expiry, rate limit, audio/multipart bounds, text guard, provider fail-closed/no charge, Stripe signature + event mapping/dedupe, privacy deletion. Billing entrypoints: Checkout/Portal URL validation and error mapping, delete token revocation, dashboard `/api/service` routes, CLI confirmation/cleanup. Integration: real curl client → loopback server with fake provider |
| Server CLI smoke | PASS | `init-db`, `create-user`, `show-user` with a temporary SQLite database; token is emitted once and usage starts at 0/1800s |
| Client service commands | PASS (unit/integration) | `service_me`, `service_status`, idempotent dictation, quota/auth/unreachable error mapping, Checkout/Portal URL checks, `service delete --confirm` cleanup and token revocation, CLI confirmation guards; hidden token entry not exercised interactively |
| Fresh-home Ubuntu installation smoke test | PASS | Install scripts, CLI wrapper, config discovery, language list, `devices`/`doctor` with mocked `pw-dump`/`wpctl` |
| Simulated full dictation workflow | PASS | Toggle start -> fake WAV recording -> stop -> mocked Groq transcription -> grammar -> clipboard -> eitype dispatch -> success audio cleanup -> `last` / `last --raw` / `clear-last` |
| Live PipeWire device enumeration | PASS | This host: `scribiva devices` parsed `pw-dump` nodes, filtered sinks, marked the virtual source and resolved the WirePlumber default source |
| Live `doctor` on the host session | PASS | Wayland/GNOME detection, PipeWire socket, devices, key mode `0600`, eitype path, writable state; no API call or audio capture |
| Package/Tauri JSON and Rust formatting | PASS (syntax only) | JSON parsed for package, TS config, Tauri config and capabilities; `rustfmt --check` passes. TypeScript and Rust were **not compiled** (no `node_modules` or Tauri system dependencies in this workspace). |
| Dashboard HTTP GET /api/status, /api/config, /api/devices, /api/last, /api/service; POST /api/service/checkout, /api/service/portal | PASS | Loopback server responds with JSON; host/token/origin checks reject invalid requests; billing routes validate configuration/plan/body and do not leak tokens |
| Dashboard live browser render | PASS | 0.2.0-alpha.1 page loaded from the loopback server in headless Chrome at desktop and 390px mobile widths; status, device list, service-address field, quota card, last-result and all `doctor` checks populated from live APIs. No control that starts recording was clicked. |

## Not verified in this environment

- A real microphone capture, Obsidian integration, or live Groq calls.
- Actual Windows or Linux X11 / KDE Wayland behavior.
- Tauri/Rust compilation (Rust toolchain is present but Tauri system dependencies and `node_modules` are not installed in this workspace).
- Twenty-language ASR and grammar quality benchmarks.
- Windows installer signing, production billing, account lifecycle, deployed quota enforcement or commercial readiness.
- Live Groq/Stripe calls and real Stripe Checkout/Portal redirects. Stripe tests use locally generated signatures and subscribe-event fixtures only.
- Multi-instance SQLite behaviour, distributed rate limiting, TLS reverse-proxy configuration, backup/restore or account deletion SLA.

## Previously verified by the user in their own Ubuntu environment

- eitype + wl-copy GNOME Wayland paste to Obsidian Live Preview and Source Mode.
- Repeated global shortcut input and Undo.
- `pw-record` with `alsa_input.pci-0000_0b_00.4.analog-stereo` microphone node.
- Groq Whisper transcription via interactive curl using valid key.

Important: previous user verification was performed on earlier Gate scripts, not this newly integrated alpha bundle. This new bundle should not be described as host-verified until it runs there.
