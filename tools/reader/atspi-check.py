#!/usr/bin/env python3
"""What a screen reader sees of a pleamar scene, over AT-SPI (docs/13-screen-readers.md).

Prints the tree of the application that shows WANTED (default: «Press me»), then
presses each button named in PRESS through AT-SPI's Action interface, as Orca
does, and waits for the words named in EXPECT to appear in the tree.

    python3 atspi-check.py [--want "Press me"] [--press "Press me" --press Quiet]
                           [--expect "pressed 1" --expect "quiet 1"]

Needs the AT-SPI bus (at-spi2-core) and PyGObject's Atspi; exits 1 if something
is missing from the tree or a press changed nothing."""
import argparse, sys, time
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

ap = argparse.ArgumentParser()
ap.add_argument("--want", default="Press me")
ap.add_argument("--press", action="append", default=[])
ap.add_argument("--expect", action="append", default=[])
ap.add_argument("--wait", type=float, default=10.0)
a = ap.parse_args()

def walk(node, depth=0, out=None):
    out = [] if out is None else out
    try:
        Atspi.Accessible.clear_cache(node)
        out.append((depth, node.get_role_name(), node.get_name() or "", node))
        for i in range(node.get_child_count()):
            child = node.get_child_at_index(i)
            if child is not None:
                walk(child, depth + 1, out)
    except Exception as e:  # a node gone while walking
        out.append((depth, "?", str(e), None))
    return out

def find_app():
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        if app is None:
            continue
        nodes = walk(app)
        if any(n[2] == a.want for n in nodes):
            return app, nodes
    return None, []

deadline = time.time() + a.wait
app, nodes = find_app()
while app is None and time.time() < deadline:
    time.sleep(0.3)
    app, nodes = find_app()
if app is None:
    print(f"no application shows «{a.want}»")
    sys.exit(1)
print(f"application «{app.get_name()}»:")
for depth, role, name, node in nodes:
    extra = ""
    if node is not None:
        try:
            v = node.get_value_iface()
            if v is not None:
                extra += f" value={v.get_current_value():g}"
        except Exception:
            pass
        try:
            states = node.get_state_set()
            for s in ("checked", "selected", "focusable", "sensitive"):
                if states.contains(getattr(Atspi.StateType, s.upper())):
                    extra += f" {s}"
        except Exception:
            pass
        try:
            act = node.get_action_iface()
            if act is not None and act.get_n_actions() > 0:
                extra += " actions=" + ",".join(act.get_action_name(i) for i in range(act.get_n_actions()))
        except Exception:
            pass
    print(f"{'  ' * depth}{role} «{name}»{extra}")
failed = False
for name in a.press:
    target = next((n for n in nodes if n[2] == name and n[3] is not None), None)
    if target is None:
        print(f"✗ nothing called «{name}» to press")
        failed = True
        continue
    act = target[3].get_action_iface()
    ok = act is not None and act.do_action(0)
    print(f"{'✓' if ok else '✗'} pressed «{name}» through AT-SPI")
    failed |= not ok
    time.sleep(0.5)
for words in a.expect:
    deadline = time.time() + a.wait
    seen = False
    while time.time() < deadline and not seen:
        seen = any(words in n[2] for n in walk(app))
        if not seen:
            time.sleep(0.3)
    print(f"{'✓' if seen else '✗'} the tree says «{words}»")
    failed |= not seen
sys.exit(1 if failed else 0)
