#!/usr/bin/env bash
# Iterate on the Scribiva GNOME extension without logging out.
#
#   bash dev.sh reload    # load the current impl.js into your running GNOME Shell
#   bash dev.sh install   # first time only: copy this checkout's loader to
#                         # ~/.local/share/gnome-shell/extensions (takes effect
#                         # at your next login; nothing is interrupted now)
#   bash dev.sh nested    # isolated GNOME Shell in a window, for loader changes
#
# Only impl.js changes are hot-reloadable. Changes to extension.js (the
# loader) need `nested` to test, and reach your real session at next login.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UUID="scribiva@scribiva.app"
SRC="$HERE/$UUID"
USER_DIR="$HOME/.local/share/gnome-shell/extensions/$UUID"

reload() {
  if ! busctl --user get-property app.scribiva.Shell /app/scribiva/Loader \
      app.scribiva.Loader1 LoaderVersion >/dev/null 2>&1; then
    echo "The running Scribiva extension has no hot-reload loader yet."
    echo "Run 'bash dev.sh install' once; it takes over at your next login."
    exit 1
  fi
  echo "Now running: $(busctl --user call app.scribiva.Shell /app/scribiva/Loader \
    app.scribiva.Loader1 Reload | sed 's/^s //')"
}

case "${1:-reload}" in
  reload)
    reload
    ;;
  install)
    # A per-user copy overrides the packaged one. Without impl-version.txt the
    # loader imports impl.js (a fresh copy on every reload).
    rm -rf "$USER_DIR"
    mkdir -p "$USER_DIR"
    cp "$SRC/extension.js" "$SRC/metadata.json" "$SRC/stylesheet.css" "$USER_DIR/"
    ln -sf "$SRC/impl.js" "$USER_DIR/impl.js"
    echo "Installed a development loader at $USER_DIR (impl.js links to this checkout)."
    echo "It takes over at your next login. After that: edit impl.js, then 'bash dev.sh reload'."
    ;;
  nested)
    WORK="$(mktemp -d)"
    trap 'rm -rf "$WORK"' EXIT
    export XDG_DATA_HOME="$WORK/share"
    mkdir -p "$XDG_DATA_HOME/gnome-shell/extensions"
    cp -R "$SRC" "$XDG_DATA_HOME/gnome-shell/extensions/"
    echo "Starting a nested GNOME Shell window. Close it to stop."
    dbus-run-session -- bash -c "
      gsettings set org.gnome.shell enabled-extensions \"['$UUID']\"
      exec gnome-shell --nested --wayland"
    ;;
  *)
    echo "usage: bash dev.sh [reload|install|nested]" >&2
    exit 2
    ;;
esac
