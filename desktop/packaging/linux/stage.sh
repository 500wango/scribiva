#!/usr/bin/env bash
# Stage everything the .deb ships besides the Tauri binary into
# packaging/linux/stage/ (gitignored). Run before `npm run tauri build`.
#
#   EITYPE_BIN=/path/to/eitype bash packaging/linux/stage.sh
#
# EITYPE_BIN defaults to a reproducible build of the pinned crate version.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
STAGE="$HERE/stage"
EITYPE_VERSION="0.2.2"

rm -rf "$STAGE"
mkdir -p "$STAGE/lib/bin" "$STAGE/doc"

# GNOME Shell extension (ADR-012). The loader (extension.js) is cached by the
# shell for the whole session; the implementation is shipped under a
# content-derived name so the app can load a new one without a logout.
EXT_SRC="$ROOT/extensions/gnome-shell/scribiva@scribiva.app"
mkdir -p "$STAGE/extension"
cp "$EXT_SRC/extension.js" "$EXT_SRC/metadata.json" "$EXT_SRC/stylesheet.css" "$STAGE/extension/"
IMPL_VERSION="$(sha256sum "$EXT_SRC/impl.js" | cut -c1-12)"
cp "$EXT_SRC/impl.js" "$STAGE/extension/impl-$IMPL_VERSION.js"
printf '%s\n' "$IMPL_VERSION" > "$STAGE/extension/impl-version.txt"

# eitype (Apache-2.0) for paste on GNOME/KDE Wayland via libei.
if [ -z "${EITYPE_BIN:-}" ]; then
  BUILD="$(mktemp -d)"
  trap 'rm -rf "$BUILD"' EXIT
  cargo install --locked --quiet --root "$BUILD" --version "=$EITYPE_VERSION" eitype
  EITYPE_BIN="$BUILD/bin/eitype"
fi
"$EITYPE_BIN" --version | grep -q "$EITYPE_VERSION" \
  || { echo "eitype $EITYPE_VERSION required" >&2; exit 1; }
install -m 0755 "$EITYPE_BIN" "$STAGE/lib/bin/eitype"
strip "$STAGE/lib/bin/eitype" 2>/dev/null || true

# Licenses: Scribiva and every bundled third-party component.
SRC="$(ls -d "$HOME"/.cargo/registry/src/*/eitype-"$EITYPE_VERSION" 2>/dev/null | head -1)"
[ -n "$SRC" ] && [ -f "$SRC/LICENSE" ] || { echo "eitype source/LICENSE not found" >&2; exit 1; }
{
  echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
  echo "Upstream-Name: Scribiva"
  echo "Source: https://github.com/500wango/scribiva"
  echo
  echo "Files: *"
  echo "License: AGPL-3.0-only"
  echo " See /usr/share/doc/scribiva/LICENSE"
  echo
  echo "Files: usr/lib/scribiva/bin/eitype"
  echo "Copyright: eitype contributors (https://github.com/Adam-D-Lewis/eitype)"
  echo "License: Apache-2.0"
  echo " See /usr/share/doc/scribiva/LICENSE.eitype and THIRD-PARTY-eitype.txt"
} > "$STAGE/doc/copyright"
cp "$ROOT/LICENSE" "$STAGE/doc/LICENSE"
cp "$SRC/LICENSE" "$STAGE/doc/LICENSE.eitype"
( cd "$SRC" && cargo metadata --format-version 1 --locked 2>/dev/null ) \
  | python3 -I "$HERE/third_party_licenses.py" > "$STAGE/doc/THIRD-PARTY-eitype.txt"
echo "Staged into $STAGE"
