#!/usr/bin/env bash
set -euo pipefail
rm -f "$HOME/.local/bin/scribiva" "$HOME/.local/bin/scribiva-insert-test"
rm -rf "$HOME/.local/share/scribiva/app"
echo "Scribiva program files removed. User config, API key, audio, logs and shortcut settings were preserved."
echo "Optional manual cleanup only when desired: ~/.config/scribiva and ~/.local/state/scribiva"
