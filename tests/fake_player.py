#!/usr/bin/env python3
"""A do-nothing MPRIS player for tests/live.sh: prints each call it gets, then flushes."""
import sys
from gi.repository import Gio, GLib

NAME = "org.mpris.MediaPlayer2.cranked_selftest"
XML = """<node><interface name="org.mpris.MediaPlayer2.Player">
<method name="PlayPause"/><method name="Next"/><method name="Previous"/>
<property name="PlaybackStatus" type="s" access="read"/></interface></node>"""

def call(_conn, _sender, _path, _iface, method, _args, invocation):
    print(method, flush=True)
    invocation.return_value(None)

def prop(*_):
    return GLib.Variant("s", "Playing")

def acquired(conn, _name):
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object("/org/mpris/MediaPlayer2", node.interfaces[0], call, prop, None)

Gio.bus_own_name(Gio.BusType.SESSION, NAME, Gio.BusNameOwnerFlags.NONE, acquired,
                 lambda *_: print("ready", flush=True), lambda *_: sys.exit(1))
GLib.MainLoop().run()
