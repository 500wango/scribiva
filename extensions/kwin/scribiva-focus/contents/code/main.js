// Scribiva focus script for KWin (Plasma 6, ADR-012).
//
// KWin scripts cannot own a D-Bus name, so this script pushes instead of
// answering: on load and on every window activation it calls
//   app.scribiva.Desktop /app/scribiva/Focus app.scribiva.Focus1.Activated(
//       resource_class, desktop_file, window_uuid, pid)
// with four strings. Numbers are sent as text because callDBus converts JS
// numbers to int32/double, never to the uint32 a typed signature would need.
// Only normal windows and dialogs are reported; panels, the desktop, popups
// and "no active window" are sent as four empty strings (focus unknown, so
// Scribiva only copies). Window titles and content are never read.
//
// When Scribiva is not running the call fails asynchronously and is ignored.

var SERVICE = "app.scribiva.Desktop";
var PATH = "/app/scribiva/Focus";
var IFACE = "app.scribiva.Focus1";
var MAX_LEN = 200;

function text(value) {
    if (value === undefined || value === null)
        return "";
    var s = String(value);
    return s.length > MAX_LEN ? s.substring(0, MAX_LEN) : s;
}

var UUID_RE = /[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/;

// internalId is a QUuid; depending on the Qt version it reaches JS as a
// string or as a wrapper whose text contains the UUID. Extract just that.
function uuidOf(id) {
    if (id === undefined || id === null)
        return "";
    var m = String(id).match(UUID_RE);
    return m ? m[0] : "";
}

function describe(win) {
    var empty = ["", "", "", ""];
    if (!win)
        return empty;
    try {
        if (!(win.normalWindow || win.dialog))
            return empty;
        var uuid = uuidOf(win.internalId);
        if (uuid === "")
            return empty;
        var pid = Number(win.pid);
        return [
            text(win.resourceClass),
            text(win.desktopFileName),
            uuid,
            pid > 0 ? String(Math.floor(pid)) : "0",
        ];
    } catch (e) {
        return empty;
    }
}

function send(win) {
    var f = describe(win);
    try {
        callDBus(SERVICE, PATH, IFACE, "Activated", f[0], f[1], f[2], f[3]);
    } catch (e) {
        // Never let a D-Bus problem break the script.
    }
}

workspace.windowActivated.connect(send);
send(workspace.activeWindow);
