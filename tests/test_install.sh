#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin" "$TMP/home/.cargo/bin"
for name in pw-record wl-copy; do
  printf '#!/bin/sh\nexit 0\n' > "$TMP/bin/$name"
  chmod +x "$TMP/bin/$name"
done
cat > "$TMP/bin/wpctl" <<'MOCK'
#!/bin/sh
if [ "$1" = "inspect" ]; then echo 'node.name = "alsa_input.mock"'; fi
MOCK
chmod +x "$TMP/bin/wpctl"
cat > "$TMP/bin/pw-dump" <<'MOCK'
#!/usr/bin/env python3
print('''[
 {"id":36,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"default"},
  "metadata":[{"key":"default.audio.source","value":{"name":"alsa_input.mock"}}]},
 {"id":54,"type":"PipeWire:Interface:Node",
  "info":{"props":{"media.class":"Audio/Source","node.name":"alsa_input.mock",
                   "node.description":"Mock Microphone","priority.session":"100"}}}
]''')
MOCK
chmod +x "$TMP/bin/pw-dump"
printf '#!/bin/sh\nexit 0\n' > "$TMP/home/.cargo/bin/eitype"
chmod +x "$TMP/home/.cargo/bin/eitype"
HOME="$TMP/home" PATH="$TMP/bin:$PATH" bash "$ROOT/scripts/install-ubuntu.sh" >/dev/null
HOME="$TMP/home" PATH="$TMP/bin:$PATH" "$TMP/home/.local/bin/scribiva" status | grep 'idle' >/dev/null
HOME="$TMP/home" PATH="$TMP/bin:$PATH" "$TMP/home/.local/bin/scribiva" doctor | grep 'alsa_input.mock' >/dev/null
HOME="$TMP/home" PATH="$TMP/bin:$PATH" "$TMP/home/.local/bin/scribiva" devices --json | grep 'alsa_input.mock' >/dev/null
HOME="$TMP/home" PATH="$TMP/bin:$PATH" "$TMP/home/.local/bin/scribiva" languages | grep '^zh' >/dev/null
printf 'Installer smoke test: PASS\n'
