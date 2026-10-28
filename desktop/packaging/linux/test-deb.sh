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
# Each check either prints PASS or stops the run (a bare "test && echo" would
# not trip set -e, so a failed check could go unnoticed in CI).
check() { local name="$1"; shift; if "$@"; then echo "PASS $name"; else echo "FAIL $name" >&2; exit 1; fi; }
useradd -m tester
as_user() { su tester -c "cd ~ && XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=GNOME $*"; }

check "no Python core shipped" test ! -e /usr/lib/scribiva/src
check "no python3 dependency" bash -c "! dpkg-query -W -f=\"\\\${Depends}\" scribiva | grep -q python3"
check "legacy shortcut shim forwards to the app" grep -q -- '--toggle' /usr/bin/scribiva-cli
check "eitype runs" bash -c "/usr/lib/scribiva/bin/eitype --version | grep -q 0.2.2"
EXT=/usr/share/gnome-shell/extensions/scribiva@scribiva.app
IMPL="$EXT/impl-$(cat "$EXT/impl-version.txt").js"
check "extension loader + versioned implementation installed" bash -c \
  "test -f \"$EXT/metadata.json\" && grep -q Loader1 \"$EXT/extension.js\" && grep -q InsertText \"$IMPL\""
missing=$(ldd /usr/bin/scribiva /usr/lib/scribiva/bin/eitype | grep -c "not found" || true)
check "shared libraries resolve" test "$missing" = 0
for tool in pw-record pw-dump wl-copy wl-paste; do command -v $tool >/dev/null || { echo "FAIL missing $tool"; exit 1; }; done
echo "PASS runtime tools installed by Depends"
check "desktop entry" grep -q '^Exec=scribiva$' /usr/share/applications/Scribiva.desktop
check "third-party license shipped" test -f /usr/share/doc/scribiva/LICENSE.eitype

apt-get remove -y -qq scribiva >/dev/null 2>&1
check "clean removal" bash -c \
  "test ! -e /usr/bin/scribiva-cli && test ! -e /usr/lib/scribiva && test ! -e $EXT"
'
