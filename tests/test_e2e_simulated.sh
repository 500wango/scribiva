#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export HOME="$TMP/home"
export PATH="$TMP/bin:$PATH"
mkdir -p "$TMP/bin" "$HOME/.cargo/bin" "$HOME/.config/scribiva"
printf 'gsk_test_key\n' > "$HOME/.config/scribiva/groq-api-key"
chmod 600 "$HOME/.config/scribiva/groq-api-key"
printf '#!/bin/sh\necho "node.name = \\"alsa_input.test\\""\n' > "$TMP/bin/wpctl"
cat > "$TMP/bin/pw-record" <<'MOCK'
#!/usr/bin/env python3
import signal, sys, time, wave, math, array
path=sys.argv[-1]
def save(_signal, _frame):
  with wave.open(path, 'wb') as file:
    file.setnchannels(1); file.setsampwidth(2); file.setframerate(16000)
    nums=array.array('h',(int(math.sin(i/40)*9000) for i in range(48000)))
    file.writeframes(nums.tobytes())
  sys.exit(0)
signal.signal(signal.SIGTERM,save)
while True: time.sleep(.1)
MOCK
cat > "$TMP/bin/curl" <<'MOCK'
#!/usr/bin/env python3
import json, sys
args=sys.argv
output=args[args.index('--output')+1]
if '/chat/completions' in args[-1]:
  data={'choices':[{'message':{'content':'你好，Scribiva 已成功完成语法修正。'}}]}
else:
  data={'text':'你好 Scribiva 已成功完成语法修正'}
open(output,'w').write(json.dumps(data))
print('200',end='')
MOCK
cat > "$TMP/bin/wl-copy" <<'MOCK'
#!/usr/bin/env python3
import sys, os
open(os.environ['HOME']+'/paste.txt','wb').write(sys.stdin.buffer.read())
MOCK
cat > "$TMP/bin/wl-paste" <<'MOCK'
#!/bin/sh
exit 1
MOCK
cat > "$HOME/.cargo/bin/eitype" <<'MOCK'
#!/bin/sh
printf '%s\n' 'paste-triggered' >> "$HOME/eitype.log"
MOCK
cat > "$TMP/bin/notify-send" <<'MOCK'
#!/bin/sh
exit 0
MOCK
chmod +x "$TMP/bin/"* "$HOME/.cargo/bin/eitype"
cat > "$TMP/bin/pw-dump" <<'MOCK'
#!/usr/bin/env python3
import json
print(json.dumps([
 {"id":36,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"default"},
  "metadata":[{"key":"default.audio.source","value":{"name":"alsa_input.test"}}]},
 {"id":54,"type":"PipeWire:Interface:Node",
  "info":{"props":{"media.class":"Audio/Source","node.name":"alsa_input.test",
                   "node.description":"Test Microphone","priority.session":"100"}}}
]))
MOCK
chmod +x "$TMP/bin/pw-dump"
bash "$ROOT/scripts/install-ubuntu.sh" >/dev/null
"$HOME/.local/bin/scribiva" devices --json | grep 'alsa_input.test' >/dev/null
"$HOME/.local/bin/scribiva" set language zh >/dev/null
"$HOME/.local/bin/scribiva" toggle | grep recording >/dev/null
sleep 1.3
"$HOME/.local/bin/scribiva" toggle | grep processing >/dev/null
for i in $(seq 1 60); do
  if [ -e "$HOME/eitype.log" ]; then break; fi
  sleep .2
done
[ -f "$HOME/eitype.log" ] || { cat "$HOME/.local/state/scribiva/scribiva.log"; echo 'No paste' >&2; exit 1; }
grep -q 'Scribiva' "$HOME/paste.txt"
"$HOME/.local/bin/scribiva" last | grep 'Scribiva' >/dev/null
"$HOME/.local/bin/scribiva" last --raw | grep '你好 Scribiva' >/dev/null
"$HOME/.local/bin/scribiva" clear-last >/dev/null
if "$HOME/.local/bin/scribiva" last >/dev/null 2>&1; then
  echo 'Last result still available after clear-last' >&2; exit 1
fi
"$HOME/.local/bin/scribiva" status | grep 'idle' >/dev/null
if find "$HOME/.local/state/scribiva/audio" -name 'record-*wav' | grep -q .; then
  echo 'Audio not deleted on success' >&2; exit 1
fi
echo 'Simulated full workflow (hotkey -> WAV -> ASR -> grammar -> paste): PASS'
