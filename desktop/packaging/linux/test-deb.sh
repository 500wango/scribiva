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

as_user "scribiva-cli --version" | grep -qx "1.0.0" && echo "PASS core runs as user"
as_user "scribiva-cli languages" | grep -q "^zh " && echo "PASS languages"
as_user "scribiva-cli settings" | grep -q "whisper-large-v3-turbo" && echo "PASS default model turbo"
as_user "scribiva-cli doctor" > /tmp/doctor.json
python3 - <<EOF
import json
d = json.load(open("/tmp/doctor.json"))
assert d["eitype_path"] == "/usr/lib/scribiva/bin/eitype", d["eitype_path"]
checks = {c["id"]: c["status"] for c in d["checks"]}
assert checks["injection"] == "ok", checks
assert checks["focus"] == "warn", checks          # no GNOME session in the container
print("PASS bundled eitype found by doctor")
EOF
/usr/lib/scribiva/bin/eitype --version | grep -q "0.2.2" && echo "PASS eitype runs"
as_user "scribiva-cli setup status" | python3 -c "
import json, sys
s = json.load(sys.stdin)[\"status\"]
assert s[\"extension\"][\"installed\"] is True, s
print(\"PASS extension installed system-wide\")"
test ! -e /usr/lib/scribiva/src/scribiva/__pycache__ && echo "PASS no bytecode shipped"
as_user "scribiva-cli status" >/dev/null
test ! -e /usr/lib/scribiva/src/scribiva/__pycache__ && echo "PASS no bytecode written at runtime"
missing=$(ldd /usr/bin/scribiva /usr/lib/scribiva/bin/eitype | grep -c "not found" || true)
[ "$missing" = 0 ] && echo "PASS shared libraries resolve"
test -f /usr/share/doc/scribiva/LICENSE.eitype && echo "PASS third-party license shipped"

apt-get remove -y -qq scribiva >/dev/null 2>&1
test ! -e /usr/bin/scribiva-cli && test ! -e /usr/lib/scribiva \
  && test ! -e /usr/share/gnome-shell/extensions/scribiva@scribiva.app && echo "PASS clean removal"
'
