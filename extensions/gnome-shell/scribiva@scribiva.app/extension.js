// Scribiva GNOME Shell extension (ADR-012).
//
// - GetFocus: the focused window's app id / WM class / window id / pid, so the
//   Scribiva client can remember the target when recording stops.
// - InsertText: types the result into that window. When the focused text
//   field speaks the Wayland text-input protocol (GTK, Qt, Firefox, …) the text
//   is committed through the input method, exactly like GNOME's on-screen
//   keyboard: no clipboard, full Unicode, one undo step. Otherwise it falls back
//   to clipboard + a single Ctrl+V from a virtual keyboard. The focus check and
//   the insertion happen in one step inside the compositor, so the text cannot
//   land in a window the user switched to in the meantime.
// - A top-bar recording indicator that never takes keyboard focus.
// Window titles and window content are never read or exposed.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';

const BUS_NAME = 'app.scribiva.Shell';
const OBJECT_PATH = '/app/scribiva/Shell';
const INTERFACE_VERSION = '2';
const PASTE_SETTLE_MS = 60;
const RESTORE_DELAY_MS = 800;
const MAX_TEXT = 100000;

// Non-idle states fall back to idle if the client stops updating us
// (e.g. it crashed mid-recording), so the indicator cannot get stuck.
const STATE_TIMEOUT_SECONDS = 10 * 60;

const INTERFACE_XML = `
<node>
  <interface name="app.scribiva.Shell1">
    <method name="GetFocus">
      <arg type="s" name="app_id" direction="out"/>
      <arg type="s" name="wm_class" direction="out"/>
      <arg type="t" name="window_id" direction="out"/>
      <arg type="u" name="pid" direction="out"/>
    </method>
    <method name="InsertText">
      <arg type="s" name="text" direction="in"/>
      <arg type="t" name="expected_window_id" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="s" name="outcome" direction="out"/>
    </method>
    <method name="SetState">
      <arg type="s" name="phase" direction="in"/>
    </method>
    <property name="Version" type="s" access="read"/>
  </interface>
</node>`;

// Icon + text per phase, so states are distinguishable without color.
const PHASES = {
    recording: {icon: 'media-record-symbolic', label: 'REC', accessible: 'Scribiva: recording'},
    processing: {icon: 'content-loading-symbolic', label: '…', accessible: 'Scribiva: processing'},
    error: {icon: 'dialog-error-symbolic', label: '!', accessible: 'Scribiva: error'},
};

const ScribivaIndicator = GObject.registerClass(
class ScribivaIndicator extends PanelMenu.Button {
    _init() {
        // No menu; not reactive and not focusable, so it never grabs focus
        // away from the window the user is dictating into.
        super._init(0.0, 'Scribiva', true);
        this.reactive = false;
        this.can_focus = false;
        this.track_hover = false;

        const box = new St.BoxLayout({style_class: 'panel-status-menu-box'});
        this._icon = new St.Icon({
            icon_name: 'audio-input-microphone-symbolic',
            style_class: 'system-status-icon',
        });
        this._label = new St.Label({
            style_class: 'scribiva-label',
            y_align: Clutter.ActorAlign.CENTER,
        });
        box.add_child(this._icon);
        box.add_child(this._label);
        this.add_child(box);
    }

    setPhase(phase) {
        const spec = PHASES[phase];
        if (!spec) {
            this.hide();
            return;
        }
        this._icon.icon_name = spec.icon;
        this._label.text = spec.label;
        this.accessible_name = spec.accessible;
        for (const name of Object.keys(PHASES))
            this.remove_style_class_name(`scribiva-${name}`);
        this.add_style_class_name(`scribiva-${phase}`);
        this.show();
    }
});

function sleep(ms) {
    return new Promise(resolve => {
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => {
            resolve();
            return GLib.SOURCE_REMOVE;
        });
    });
}

function clipboardText() {
    return new Promise(resolve => {
        St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD,
            (_clipboard, text) => resolve(text));
    });
}

// Dictated text is data: drop control characters except newline and tab, and
// never produce a carriage return (no Enter is ever sent).
function sanitize(text, singleLine) {
    let out = text.replace(/\r\n?/g, '\n')
        .replace(/[\u0000-\u0008\u000b-\u001f\u007f\u0080-\u009f]/g, '');
    if (singleLine)
        out = out.replace(/[\n\t]+/g, ' ');
    return out;
}

function boolOption(options, name, fallback) {
    const value = options[name];
    if (value === undefined)
        return fallback;
    const unpacked = value instanceof GLib.Variant ? value.deepUnpack() : value;
    return unpacked === true;
}

function stringOption(options, name, fallback) {
    const value = options[name];
    if (value === undefined)
        return fallback;
    const unpacked = value instanceof GLib.Variant ? value.deepUnpack() : value;
    return typeof unpacked === 'string' ? unpacked : fallback;
}

// D-Bus implementation object. Method/property names must match the XML.
class ScribivaService {
    constructor(onSetState, keyboard) {
        this._onSetState = onSetState;
        this._keyboard = keyboard;
        this._busy = false;
    }

    get Version() {
        return INTERFACE_VERSION;
    }

    GetFocus() {
        const empty = new GLib.Variant('(sstu)', ['', '', 0, 0]);

        // While the shell itself holds keyboard focus (overview, run dialog,
        // top-bar menus, lock screen), a paste would not reach any app window,
        // so report "no focused window".
        if (Main.modalCount > 0)
            return empty;

        const win = global.display.get_focus_window();
        if (!win)
            return empty;

        const app = Shell.WindowTracker.get_default().get_window_app(win);
        const appId = app?.get_id() ?? '';
        const wmClass = win.get_wm_class() ?? '';
        const windowId = win.get_id();
        const pid = win.get_pid();

        return new GLib.Variant('(sstu)', [
            appId,
            wmClass,
            windowId,
            pid > 0 ? pid : 0,
        ]);
    }

    SetState(phase) {
        this._onSetState(phase);
    }

    InsertTextAsync(params, invocation) {
        const [text, expectedId, options] = params;
        this._insert(text, expectedId, options)
            .then(outcome => invocation.return_value(new GLib.Variant('(s)', [outcome])))
            .catch(error => {
                console.error(`Scribiva InsertText failed: ${error}`);
                invocation.return_value(new GLib.Variant('(s)', ['failed']));
            });
    }

    async _insert(rawText, expectedId, options) {
        if (typeof rawText !== 'string' || rawText.length === 0 || rawText.length > MAX_TEXT)
            return 'invalid';
        if (this._busy)
            return 'busy';
        if (Main.modalCount > 0)
            return 'no_focus';
        const win = global.display.get_focus_window();
        if (!win)
            return 'no_focus';
        // GJS maps the D-Bus uint64 to a Number; window ids fit in 53 bits.
        if (Number(win.get_id()) !== Number(expectedId))
            return 'focus_changed';

        const text = sanitize(rawText, boolOption(options, 'single_line', false));
        if (text.length === 0)
            return 'invalid';

        // 1. Input-method commit into the focused text field.
        if (Main.inputMethod.currentFocus) {
            Main.inputMethod.commit(text);
            return 'committed';
        }

        // 2. Clipboard + one Ctrl+V (apps without text-input, e.g. XWayland).
        if (!boolOption(options, 'paste_fallback', true))
            return 'no_text_input';
        this._busy = true;
        try {
            const restore = boolOption(options, 'restore_clipboard', true);
            const previous = restore ? await clipboardText() : null;
            const clipboard = St.Clipboard.get_default();
            clipboard.set_text(St.ClipboardType.CLIPBOARD, text);
            await sleep(PASTE_SETTLE_MS);
            if (global.display.get_focus_window() !== win)
                return 'focus_changed';
            const withShift = stringOption(options, 'paste_shortcut', 'ctrl+v') === 'ctrl+shift+v';
            this._keyboard.pressChord(withShift);
            if (restore && typeof previous === 'string') {
                await sleep(RESTORE_DELAY_MS);
                // Only if nobody changed the clipboard after us.
                if (await clipboardText() === text)
                    clipboard.set_text(St.ClipboardType.CLIPBOARD, previous);
            }
            return 'pasted';
        } finally {
            this._busy = false;
        }
    }
}

// Virtual keyboard owned by the compositor itself: no portal, no uinput.
class VirtualKeyboard {
    constructor() {
        const seat = Clutter.get_default_backend().get_default_seat();
        this._device = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
    }

    _key(keyval, pressed) {
        this._device.notify_keyval(GLib.get_monotonic_time(), keyval,
            pressed ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED);
    }

    pressChord(withShift) {
        this._key(Clutter.KEY_Control_L, true);
        if (withShift)
            this._key(Clutter.KEY_Shift_L, true);
        this._key(Clutter.KEY_v, true);
        this._key(Clutter.KEY_v, false);
        if (withShift)
            this._key(Clutter.KEY_Shift_L, false);
        this._key(Clutter.KEY_Control_L, false);
    }

    destroy() {
        this._device = null;
    }
}

export default class ScribivaExtension extends Extension {
    enable() {
        this._indicator = new ScribivaIndicator();
        Main.panel.addToStatusArea(this.uuid, this._indicator, 0, 'right');
        this._indicator.hide();

        this._keyboard = new VirtualKeyboard();
        this._service = new ScribivaService(phase => this._setState(phase), this._keyboard);
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE_XML, this._service);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);

        this._ownerId = Gio.bus_own_name_on_connection(
            Gio.DBus.session,
            BUS_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            null);
    }

    disable() {
        if (this._ownerId) {
            Gio.bus_unown_name(this._ownerId);
            this._ownerId = 0;
        }

        if (this._dbus) {
            this._dbus.flush();
            this._dbus.unexport();
            this._dbus = null;
        }
        this._service = null;
        this._keyboard?.destroy();
        this._keyboard = null;

        this._clearStateTimeout();

        this._indicator?.destroy();
        this._indicator = null;
    }

    _setState(phase) {
        if (typeof phase !== 'string')
            return;
        if (phase !== 'idle' && !(phase in PHASES))
            return; // unknown phase: ignore

        this._clearStateTimeout();
        this._indicator?.setPhase(phase);

        if (phase !== 'idle') {
            this._stateTimeoutId = GLib.timeout_add_seconds(
                GLib.PRIORITY_DEFAULT,
                STATE_TIMEOUT_SECONDS,
                () => {
                    this._stateTimeoutId = 0;
                    this._indicator?.setPhase('idle');
                    return GLib.SOURCE_REMOVE;
                });
        }
    }

    _clearStateTimeout() {
        if (this._stateTimeoutId) {
            GLib.source_remove(this._stateTimeoutId);
            this._stateTimeoutId = 0;
        }
    }
}
