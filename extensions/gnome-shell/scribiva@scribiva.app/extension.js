// Scribiva GNOME Shell extension (ADR-012).
//
// Exposes the focused window's app id / WM class / window id / pid on the
// session bus so the local Scribiva client can verify the paste target, and
// shows a top-bar recording indicator that never takes keyboard focus.
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
const INTERFACE_VERSION = '1';

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

// D-Bus implementation object. Method/property names must match the XML.
class ScribivaService {
    constructor(onSetState) {
        this._onSetState = onSetState;
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
}

export default class ScribivaExtension extends Extension {
    enable() {
        this._indicator = new ScribivaIndicator();
        Main.panel.addToStatusArea(this.uuid, this._indicator, 0, 'right');
        this._indicator.hide();

        this._service = new ScribivaService(phase => this._setState(phase));
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
