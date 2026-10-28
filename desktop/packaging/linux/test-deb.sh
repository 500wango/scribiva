#!/usr/bin/env bash
# Install the built .deb into a clean Ubuntu container and check that the
# package works without the source tree: dependencies resolve, the core runs
# as a normal user, eitype and the GNOME extension are found, and removal is
# clean. Desktop behaviour (hotkey, paste, extension loading) still needs a
# real GNOME session.
#
#   bash packaging/linux/test-deb.sh [path/to/scribiva_*.deb] [ubuntu:24.04]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEB="${1:-$(ls -t "$HERE"/../../src-tauri/target/release/bundle/deb/*.deb | head -1)}"
IMAGE="${2:-ubuntu:24.04}"
DEB="$(cd "$(dirname "$DEB")" && pwd)/$(basename "$DEB")"
echo "Testing $(basename "$DEB") on $IMAGE"

docker run --rm -v "$DEB:/tmp/scribiva.deb:ro" "$IMAGE" bash -euo pipefail -c '
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq /tmp/scribiva.deb >/tmp/apt.log 2>&1 || { tail -30 /tmp/apt.log; exit 1; }
echo "PASS install: $(dpkg-query -W -f="\${Version}" scribiva)"
useradd -m tester
as_user() { su tester -c "cd ~ && XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=GNOME $*"; }

test ! -e /usr/lib/scribiva/src && echo "PASS no Python core shipped"
dpkg-query -W -f="\${Depends}" scribiva | grep -qv python3 && echo "PASS no python3 dependency"
grep -q -- '--toggle' /usr/bin/scribiva-cli && echo "PASS legacy shortcut shim forwards to the app"
/usr/lib/scribiva/bin/eitype --version | grep -q "0.2.2" && echo "PASS eitype runs"
EXT=/usr/share/gnome-shell/extensions/scribiva@scribiva.app
IMPL="$EXT/impl-$(cat "$EXT/impl-version.txt").js"
test -f "$EXT/metadata.json" && grep -q Loader1 "$EXT/extension.js" \
  && grep -q InsertText "$IMPL" && echo "PASS extension loader + versioned implementation installed"
missing=$(ldd /usr/bin/scribiva /usr/lib/scribiva/bin/eitype | grep -c "not found" || true)
[ "$missing" = 0 ] && echo "PASS shared libraries resolve"
for tool in pw-record pw-dump wl-copy wl-paste; do command -v $tool >/dev/null || { echo "FAIL missing $tool"; exit 1; }; done
echo "PASS runtime tools installed by Depends"
grep -q '^Exec=scribiva$' /usr/share/applications/Scribiva.desktop && echo "PASS desktop entry"
test -f /usr/share/doc/scribiva/LICENSE.eitype && echo "PASS third-party license shipped"

apt-get remove -y -qq scribiva >/dev/null 2>&1
test ! -e /usr/bin/scribiva-cli && test ! -e /usr/lib/scribiva \
  && test ! -e /usr/share/gnome-shell/extensions/scribiva@scribiva.app && echo "PASS clean removal"
'
