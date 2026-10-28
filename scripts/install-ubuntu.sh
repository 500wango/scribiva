#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$HOME/.local/share/scribiva/app"
BIN="$HOME/.local/bin"
CONFIG="$HOME/.config/scribiva"
STATE="$HOME/.local/state/scribiva"

for cmd in python3 pw-record wpctl wl-copy curl; do
  command -v "$cmd" >/dev/null || { echo "缺少 $cmd" >&2; exit 1; }
done
if [ ! -x "$HOME/.cargo/bin/eitype" ] && ! command -v eitype >/dev/null; then
  echo "缺少 eitype：请先安装并完成 GNOME Wayland 授权" >&2
  exit 1
fi
umask 077
mkdir -p "$APP" "$BIN" "$CONFIG" "$STATE"
chmod 700 "$CONFIG" "$STATE"
python3 -m compileall -q "$ROOT/src"
if [ -e "$BIN/scribiva-insert-test" ]; then
  cp -p "$BIN/scribiva-insert-test" "$BIN/scribiva-insert-test.pre-alpha-$(date +%Y%m%d-%H%M%S)"
fi
cp -R "$ROOT/src" "$APP/"
cp -R "$ROOT/web" "$APP/"
# Preserve ~/.config/scribiva/groq-api-key and ~/.config/scribiva/mic-node.
cat > "$BIN/scribiva" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
APP="$HOME/.local/share/scribiva/app"
export PYTHONPATH="$APP/src${PYTHONPATH:+:$PYTHONPATH}"
exec python3 -m scribiva "$@"
WRAPPER
cat > "$BIN/scribiva-insert-test" <<'HOTKEY'
#!/usr/bin/env bash
exec "$HOME/.local/bin/scribiva" toggle
HOTKEY
chmod 700 "$BIN/scribiva" "$BIN/scribiva-insert-test"
if [ -e "$CONFIG/groq-api-key" ]; then chmod 600 "$CONFIG/groq-api-key"; fi
printf '\nScribiva Alpha installed.\n'
printf 'Shortcut target: %s\n' "$BIN/scribiva-insert-test"
printf 'Local dashboard: %s\n' "$BIN/scribiva ui"
printf 'Diagnostics: %s\n' "$BIN/scribiva doctor"
printf 'Key setup (if needed): %s\n' "$BIN/scribiva key"
printf 'If Gate 1 custom shortcut is still bound to scribiva-insert-test, Ctrl+Alt+V continues working.\n'
printf 'Backup copies of previous gate script are kept beside the executable.\n'
