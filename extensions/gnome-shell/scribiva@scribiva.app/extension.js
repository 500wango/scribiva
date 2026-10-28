// Scribiva GNOME Shell extension — loader.
//
// GNOME Shell imports this file once per login session and caches it, so it
// is kept minimal and stable. All behaviour lives in a versioned
// implementation module (impl.js, installed as impl-<version>.js) that this
// loader imports by URI. Because every version has a different file name, a
// newer implementation can be loaded into the running shell without logging
// out: the Scribiva app calls Loader1.Reload after an update.
//
// Security: for an installed package, Reload only loads impl-<version>.js from
// this extension's own root-owned directory, with the version restricted to
// [A-Za-z0-9._-], so a session-bus caller cannot inject code. A source
// checkout (no impl-version.txt) copies impl.js from the same directory.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'app.scribiva.Shell';
const LOADER_PATH = '/app/scribiva/Loader';
const LOADER_VERSION = '1';
const VERSION_RE = /^[A-Za-z0-9._-]{1,32}$/;

const LOADER_XML = `
<node>
  <interface name="app.scribiva.Loader1">
    <method name="Reload">
      <arg type="s" name="impl_version" direction="out"/>
    </method>
    <property name="LoaderVersion" type="s" access="read"/>
    <property name="ImplVersion" type="s" access="read"/>
    <property name="AvailableImplVersion" type="s" access="read"/>
  </interface>
</node>`;

// D-Bus object for app.scribiva.Loader1; names must match LOADER_XML.
class LoaderService {
    constructor(extension) {
        this._extension = extension;
    }

    get LoaderVersion() {
        return LOADER_VERSION;
    }

    get ImplVersion() {
        return this._extension._implVersion;
    }

    get AvailableImplVersion() {
        return this._extension._availableVersion() ?? '';
    }

    ReloadAsync(_params, invocation) {
        this._extension._reload()
            .then(version => invocation.return_value(new GLib.Variant('(s)', [version])))
            .catch(e => invocation.return_dbus_error(
                'app.scribiva.Loader1.Error.ReloadFailed', String(e)));
    }
}

export default class ScribivaExtension extends Extension {
    enable() {
        this._impl = null;
        this._implVersion = '';
        this._reloading = null;

        this._loaderService = new LoaderService(this);
        this._loaderDbus = Gio.DBusExportedObject.wrapJSObject(LOADER_XML, this._loaderService);
        this._loaderDbus.export(Gio.DBus.session, LOADER_PATH);
        this._ownerId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);

        this._reload().catch(e => console.error(`Scribiva: could not load implementation: ${e}`));
    }

    disable() {
        this._unloadImpl();
        if (this._ownerId) {
            Gio.bus_unown_name(this._ownerId);
            this._ownerId = 0;
        }
        if (this._loaderDbus) {
            this._loaderDbus.flush();
            this._loaderDbus.unexport();
            this._loaderDbus = null;
        }
        this._loaderService = null;
    }

    // impl-version.txt names the current implementation file. The packaging
    // writes it together with impl-<version>.js.
    _availableVersion() {
        try {
            const file = this.dir.get_child('impl-version.txt');
            const [ok, bytes] = file.load_contents(null);
            if (!ok)
                return null;
            const version = new TextDecoder().decode(bytes).trim();
            return VERSION_RE.test(version) ? version : null;
        } catch {
            return null;
        }
    }

    _implFile(version) {
        // Source checkouts ship impl.js only; packages ship impl-<version>.js.
        const versioned = this.dir.get_child(`impl-${version}.js`);
        return versioned.query_exists(null) ? versioned : null;
    }

    _unloadImpl() {
        if (!this._impl)
            return;
        try {
            this._impl.disable();
        } catch (e) {
            console.error(`Scribiva: disabling implementation failed: ${e}`);
        }
        this._impl = null;
        this._implVersion = '';
    }

    async _reload() {
        // Serialise reloads: a second call waits for the first.
        if (this._reloading)
            await this._reloading;
        this._reloading = this._doReload();
        try {
            return await this._reloading;
        } finally {
            this._reloading = null;
        }
    }

    // Development (source checkout without impl-version.txt): GJS caches a
    // module per URI, so each reload imports a fresh copy of impl.js from the
    // user's runtime dir. The packaged install never takes this path.
    _devCopy() {
        const source = this.dir.get_child('impl.js');
        const folder = Gio.File.new_for_path(
            GLib.build_filenamev([GLib.get_user_runtime_dir(), 'scribiva-shell-dev']));
        try {
            folder.make_directory_with_parents(null);
        } catch (e) {
            if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.EXISTS))
                throw e;
        }
        const target = folder.get_child(`impl-${Date.now()}.js`);
        source.copy(target, Gio.FileCopyFlags.OVERWRITE, null, null);
        return target;
    }

    async _doReload() {
        const version = this._availableVersion();
        const file = version ? this._implFile(version) : null;
        if (version && !file)
            throw new Error(`impl-${version}.js is missing`);
        const label = file ? version : `dev-${Date.now()}`;
        if (file && this._impl && this._implVersion === label)
            return label;
        const uri = (file ?? this._devCopy()).get_uri();

        // Import first: a broken new version must not take down the old one.
        const module = await import(uri);
        if (typeof module.Implementation !== 'function')
            throw new Error(`${uri} does not export Implementation`);
        const next = new module.Implementation({uuid: this.uuid, implVersion: label});

        this._unloadImpl();
        next.enable(Gio.DBus.session);
        this._impl = next;
        this._implVersion = label;
        console.log(`Scribiva: implementation ${label} active`);
        return label;
    }
}
