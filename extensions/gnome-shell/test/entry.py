"""GTK4 window with one focused text field; prints its text when it changes."""
import sys
import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gio, GLib, Gtk

mode = sys.argv[1] if len(sys.argv) > 1 else 'entry'


def on_activate(app):
    win = Gtk.ApplicationWindow(application=app, title='scribiva-test')
    if mode == 'textview':
        view = Gtk.TextView()
        buf = view.get_buffer()
        buf.set_text('before ')
        buf.place_cursor(buf.get_end_iter())
        buf.connect('changed', lambda b: print('TEXT=' + repr(b.get_text(
            b.get_start_iter(), b.get_end_iter(), False)), flush=True))
        widget = view
    else:
        widget = Gtk.Entry()
        widget.set_text('before ')
        widget.connect('changed', lambda e: print('TEXT=' + repr(e.get_text()), flush=True))
    win.set_child(widget)
    win.present()

    def focus():
        widget.grab_focus()
        if mode == 'entry':
            widget.set_position(-1)
        print('READY', flush=True)
        return GLib.SOURCE_REMOVE
    GLib.timeout_add(800, focus)


# NON_UNIQUE: no bus name registration, so the test does not wait on D-Bus.
app = Gtk.Application(application_id='app.scribiva.TestEntry',
                      flags=Gio.ApplicationFlags.NON_UNIQUE)
app.connect('activate', on_activate)
app.run([])
