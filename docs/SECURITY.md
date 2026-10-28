# Security and privacy considerations

## Current development-only secret handling

- Reads key from `GROQ_API_KEY` environment or `~/.config/scribiva/groq-api-key` (mode 0600).
- Uses `curl --config -` with stdin for authorization rather than leaking keys through ordinary `ps` command arguments. The bearer token does still reside in process memory, and must not be distributed to end users.
- Local dashboard only listens on 127.0.0.1 using an unpredictable port, strict Host / Origin checks, and an anti-CSRF token for POST. It is not an internet-accessible admin console.
- No external JS/CDN. Strict minimum-permission directory and file modes are used.

## Service-mode token and server boundary

- `scribiva service login` stores only an opaque `scr_...` Bearer token in `~/.config/scribiva/service-token` (mode 0600); it never reads or stores the server's Groq key.
- The client sends the token with `curl --config -` via stdin, not in argv. The URL must be HTTPS unless it is loopback. Embedded credentials, query strings and fragments are rejected.
- The server stores token hashes (SHA-256), not plaintext tokens, and enforces expiry/revocation. It defaults to `127.0.0.1`; non-loopback binding requires an explicit `SCRIBIVA_ALLOW_REMOTE=1` and should sit behind a TLS reverse proxy.
- Quota is reserved transactionally before the provider call; only completed jobs create ledger debits. Failed, cancelled, expired and rejected requests are not charged.
- Stripe entitlement changes require a valid `Stripe-Signature` HMAC and deduplicated event ID. `checkout.session.completed` does not grant Pro; subscription events do. `invoice.paid` is not an entitlement source.
- `service billing` / `service portal` and the dashboard only accept HTTPS Stripe URLs; card data never passes through the client or the Scribiva server. Redirect completion is not treated as payment.
- `service delete --confirm` and dashboard/API settings never print the token. Server-side deletion revokes all tokens; the CLI clears local credentials after a successful request.
- The dashboard service card calls server `/v1/config/public` and `/v1/me` only; it does not receive the token from or expose it to the browser.
- Server logs omit Authorization headers, audio, transcripts and Stripe secrets. API responses expose request IDs and structured error codes only.
- `DELETE /v1/me/data` removes jobs, transcripts and tokens; usage/subscription metadata is retained for billing. This is a skeleton deletion flow, not a complete GDPR/CCPA workflow.

## Audio and text retention

- Creates fresh uniquely named `.wav` recordings. Successful job recordings are deleted by default; failed recordings remain in `~/.local/state/scribiva/audio/` for manual retry. Users must remove these files manually if desired.
- Last successful result remains in `~/.local/state/scribiva/last-result.txt`, mode 0600, until overwritten or manually deleted.
- The recording and edited text are sent to Groq. No automatic transcription text is written to normal logs; logs only include job identifiers, errors and length.
- Python CLI exceptions may include supplier-provided error messages; they should not include voice content, but users should review logs before publishing.

## Restrictions

- No blanket sudo or Linux input-group membership.
- No TLS verification bypass, browser fingerprint spoofing or retry loops for Cloudflare 1010.
- On Wayland, foreground application identity and caret location are not generically verifiable. Auto-insert is a convenience option with explicit risk; do not use it for sensitive content unless target has remained focused.
- No guarantee that simulated Ctrl+V succeeded or that undo forms one action in every target app.
- Clipboard rich content may be replaced by plain text. Restoration is only attempted if old content was accessible as plain text and the clipboard still contains Scribiva's result.

## Before shipping commercially

The architecture in `docs/07-SECURITY-PRIVACY.md` remains the target. The bundled server now provides an authenticated proxy, transactional quota, hashed tokens, signed webhook mapping and an initial deletion endpoint, but production still requires PostgreSQL/shared state, distributed rate limiting, TLS/account lifecycle, full Stripe test-mode validation, abuse controls, audit retention, consent/onboarding, and real Windows/Linux QA.
