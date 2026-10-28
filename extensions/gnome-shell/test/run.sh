#!/usr/bin/env bash
# Headless GNOME Shell + Scribiva extension + GTK4 field; checks that
# InsertText commits Unicode text into the focused field via the input method.
# Usage: dbus-run-session -- bash run.sh   (private session bus)
#
# Known limitation (2026-10-28, GNOME 46 on the dev machine): the extension
# loads and its D-Bus API answers, but GTK clients stall during Wayland
# startup in this headless setup, so the field checks do not complete. Use
# manual-check.sh in a real GNOME session for the insertion itself.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT="$(cd "$HERE/.." && pwd)/scribiva@scribiva.app"
WORK="$(mktemp -d)"
trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$WORK"' EXIT
export HOME="$WORK/home" XDG_CONFIG_HOME="$WORK/home/.config" XDG_DATA_HOME="$WORK/home/.local/share"
export XDG_RUNTIME_DIR="$WORK/run" XDG_CACHE_HOME="$WORK/home/.cache"
mkdir -p "$XDG_DATA_HOME/gnome-shell/extensions" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
cp -R "$EXT" "$XDG_DATA_HOME/gnome-shell/extensions/"
export GSETTINGS_BACKEND=keyfile
gsettings set org.gnome.shell enabled-extensions "['scribiva@scribiva.app']"
gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.desktop.a11y.applications screen-keyboard-enabled false
export WAYLAND_DISPLAY=scribiva-test-0
gnome-shell --headless --virtual-monitor 1280x800 --wayland-display "$WAYLAND_DISPLAY" \
  --no-x11 >"$WORK/shell.log" 2>&1 &
for _ in $(seq 1 100); do
  gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner app.scribiva.Shell 2>/dev/null | grep -q true && break
  sleep 0.2
done
gdbus introspect --session --dest app.scribiva.Shell --object-path /app/scribiva/Shell \
  | grep -q InsertText || { tail -40 "$WORK/shell.log"; echo "FAIL extension not loaded" >&2; exit 1; }
echo "PASS extension loaded in headless shell"
# A fresh session starts in the overview (a modal grab); leave it so app
# windows can take keyboard focus.
gsettings set org.gnome.shell welcome-dialog-last-shown-version '999' 2>/dev/null || true
for _ in $(seq 1 20); do
  gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
    --method org.freedesktop.DBus.Properties.Set org.gnome.Shell OverviewActive '<false>' >/dev/null 2>&1 || true
  sleep 0.5
  gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
    --method org.freedesktop.DBus.Properties.Get org.gnome.Shell OverviewActive 2>/dev/null \
    | grep -q false && break
done

check() {
  local mode="$1" expected="$2"
  GTK_A11Y=none NO_AT_BRIDGE=1 GTK_USE_PORTAL=0 GDK_DEBUG=no-portals GDK_BACKEND=wayland python3 "$HERE/entry.py" "$mode" >"$WORK/$mode.out" 2>"$WORK/$mode.err" &
  local app=$!
  for _ in $(seq 1 75); do grep -q READY "$WORK/$mode.out" 2>/dev/null && break; sleep 0.2; done
  sleep 1
  local focus wid outcome
  focus=$(gdbus call --session --dest app.scribiva.Shell --object-path /app/scribiva/Shell \
    --method app.scribiva.Shell1.GetFocus)
  wid=$(echo "$focus" | sed -E 's/.*uint64 ([0-9]+).*/\1/')
  [ -n "$wid" ] && [ "$wid" != 0 ] || { echo "FAIL $mode: no focused window ($focus)" >&2
    echo "app stdout: $(head -c 300 "$WORK/$mode.out")" >&2
    echo "app stderr: $(grep -v AddMatch "$WORK/$mode.err" | head -c 600)" >&2; return 1; }
  outcome=$(gdbus call --session --dest app.scribiva.Shell --object-path /app/scribiva/Shell \
    --method app.scribiva.Shell1.InsertText "$3" "uint64 $wid" "{}")
  sleep 0.6
  kill "$app" 2>/dev/null || true
  wait "$app" 2>/dev/null || true
  grep -q "committed" <<<"$outcome" || { echo "FAIL $mode outcome $outcome" >&2; cat "$WORK/$mode.err" >&2; return 1; }
  tail -1 "$WORK/$mode.out" | grep -qF -- "$expected" \
    || { echo "FAIL $mode: field shows $(tail -1 "$WORK/$mode.out")" >&2; return 1; }
  echo "PASS $mode: $outcome -> $(tail -1 "$WORK/$mode.out")"
}
check entry  "before 你好 Scribiva 🎉 العربية"  "你好 Scribiva 🎉 العربية"
check textview "before 第一行\\n- 第二行" "第一行
- 第二行"

# Wrong expected window id: nothing is typed.
out=$(gdbus call --session --dest app.scribiva.Shell --object-path /app/scribiva/Shell \
  --method app.scribiva.Shell1.InsertText "x" "uint64 999999" "{}")
grep -qE "focus_changed|no_focus" <<<"$out" && echo "PASS stale window id rejected ($out)"
