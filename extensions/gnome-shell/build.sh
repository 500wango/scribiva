#!/usr/bin/env bash
# Pack the Scribiva GNOME Shell extension into dist/ (gitignored).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
uuid="scribiva@scribiva.app"

mkdir -p "$here/dist"
gnome-extensions pack "$here/$uuid" \
    --force \
    --out-dir="$here/dist"

echo "Built: $here/dist/$uuid.shell-extension.zip"
