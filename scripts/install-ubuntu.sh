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
  echo "提示：未找到 eitype，自动粘贴不可用，Scribiva 将只复制到剪贴板。" >&2
fi
umask 077
mkdir -p "$APP" "$BIN" "$CONFIG" "$STATE"
chmod 700 "$CONFIG" "$STATE"
python3 -m compileall -q "$ROOT/src"
if [ -e "$BIN/scribiva-insert-test" ]; then
  cp -p "$BIN/scribiva-insert-test" "$BIN/scribiva-insert-test.pre-alpha-$(date +%Y%m%d-%H%M%S)"
fi
rm -rf "$APP/src" "$APP/web"
cp -R "$ROOT/src" "$APP/"
cp -R "$ROOT/web" "$APP/"
find "$APP" -name '__pycache__' -prune -exec rm -rf {} +
# GNOME Shell extension: focus guard + recording indicator (ADR-012).
EXT_UUID="scribiva@scribiva.app"
EXT_SRC="$ROOT/extensions/gnome-shell/$EXT_UUID"
EXT_DST="$HOME/.local/share/gnome-shell/extensions/$EXT_UUID"
EXT_INSTALLED=0
if [ -f "$EXT_SRC/metadata.json" ]; then
  rm -rf "$EXT_DST"
  mkdir -p "$(dirname "$EXT_DST")"
  cp -R "$EXT_SRC" "$EXT_DST"
  chmod -R u=rwX,go=rX "$EXT_DST"
  EXT_INSTALLED=1
fi
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
printf '\nScribiva installed.\n'
printf 'Shortcut target: %s\n' "$BIN/scribiva-insert-test"
printf 'Local dashboard: %s\n' "$BIN/scribiva ui"
printf 'Diagnostics: %s\n' "$BIN/scribiva doctor"
printf 'Key setup (if needed): %s\n' "$BIN/scribiva key"
printf 'Sign in: %s\n' "$BIN/scribiva service login --url https://api.<domain>"
if [ "$EXT_INSTALLED" = 1 ]; then
  printf '\nGNOME extension copied to %s\n' "$EXT_DST"
  printf 'Log out and back in once, then run: gnome-extensions enable %s\n' "$EXT_UUID"
  printf 'Without it, Scribiva copies text instead of pasting (it cannot verify the target window).\n'
fi
