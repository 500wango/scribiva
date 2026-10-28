#!/usr/bin/env bash
# Real-session check of in-compositor text insertion (run inside your GNOME
# session after logging in with the v2 extension). It types a sample string
# into whatever text field has focus 5 seconds after you start it.
#
#   bash manual-check.sh            # then click into a text field and wait
set -euo pipefail
DEST=app.scribiva.Shell
OBJ=/app/scribiva/Shell
IFACE=app.scribiva.Shell1
version=$(busctl --user --json=short get-property "$DEST" "$OBJ" "$IFACE" Version 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"])' 2>/dev/null || echo 0)
if [ "$version" -lt 2 ] 2>/dev/null; then
  echo "Scribiva extension v2 is not active (found: $version)."
  echo "Run 'scribiva-cli setup extension', log out and back in, then retry."
  exit 1
fi
echo "Extension v$version active. Click into a text field now; typing in 5 seconds…"
sleep 5
wid=$(busctl --user --json=short call "$DEST" "$OBJ" "$IFACE" GetFocus \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"][2])')
sample='Scribiva 测试：你好 🎉 العربية — 第二部分'
outcome=$(busctl --user --json=short -- call "$DEST" "$OBJ" "$IFACE" InsertText 'sta{sv}' \
  "$sample" "$wid" 0 | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"][0])')
echo "Outcome: $outcome"
case "$outcome" in
  committed) echo "Inserted through the input method. Check the text and press Ctrl+Z once: it should undo the whole insertion." ;;
  pasted)    echo "This app has no Wayland text-input support; Scribiva used its paste fallback." ;;
  *)         echo "Nothing was typed ($outcome)." ;;
esac
