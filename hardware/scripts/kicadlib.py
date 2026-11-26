"""Read-only helpers for KiCad files: an s-expression parser and a
schematic connectivity extractor.

`schematic(root)` walks a hierarchical .kicad_sch project and returns every
placed symbol and every net, without needing KiCad installed. Connectivity
follows KiCad's rules: wire ends, pin ends, label anchors and junctions join
when they coincide, and anything sitting on a wire joins that wire; local
labels join within a sheet, global labels and power symbols join everywhere.
"""
import math
import os
import re

TOKEN = re.compile(r'\s*(?:(\()|(\))|"((?:[^"\\]|\\.)*)"|([^\s()"]+))')


def sexpr(text):
    stack, out = [], []
    for m in TOKEN.finditer(text):
        if m.group(1):
            stack.append(out)
            out = []
        elif m.group(2):
            done, out = out, stack.pop()
            out.append(done)
        elif m.group(3) is not None:
            out.append(m.group(3).replace('\\"', '"'))
        elif m.group(4):
            out.append(m.group(4))
    return out[0]


def child(node, name):
    for n in node:
        if isinstance(n, list) and n and n[0] == name:
            return n
    return None


def children(node, name):
    return [n for n in node if isinstance(n, list) and n and n[0] == name]


def isnum(v):
    return isinstance(v, str) and re.fullmatch(r"-?\d+(\.\d+)?(e-?\d+)?", v)


def nums(node, name, default=None):
    n = child(node, name)
    return [float(v) for v in n[1:] if isnum(v)] if n else default


def props(node):
    return {p[1]: p[2] for p in children(node, "property")}


def pcb(path):
    """{ref: (footprint id, {pad number: net name or None})} of a board."""
    board = sexpr(open(path, encoding="utf-8").read())
    out = {}
    for fp in children(board, "footprint"):
        pads = {}
        for p in children(fp, "pad"):
            if p[1]:
                net = child(p, "net")
                pads[p[1]] = net[-1] if net and len(net) > 1 else None
        out[props(fp).get("Reference")] = (fp[1], pads)
    return out


# ---------------------------------------------------------------------------
# schematic connectivity
# ---------------------------------------------------------------------------
def _key(x, y):
    return (round(x * 100), round(y * 100))


def _pin_xy(at, mirror, px, py):
    """Library pin (y up) -> sheet position (y down)."""
    x, y = px, -py
    a = math.radians(at[2] if len(at) > 2 else 0)
    x, y = (round(x * math.cos(a) + y * math.sin(a), 4),
            round(-x * math.sin(a) + y * math.cos(a), 4))
    if mirror == "x":
        y = -y
    elif mirror == "y":
        x = -x
    return at[0] + x, at[1] + y


def _lib_pins(lib, unit, style):
    """[(number, name, type, hidden, x, y)] of a library symbol's unit."""
    out = []
    for sub in children(lib, "symbol"):
        m = re.search(r"_(\d+)_(\d+)$", sub[1])
        u, s = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
        if u not in (0, unit) or s not in (0, style):
            continue
        for p in children(sub, "pin"):
            at = nums(p, "at")
            hidden = "hide" in p or (child(p, "hide") or [0, "no"])[1] == "yes"
            out.append((child(p, "number")[1], child(p, "name")[1], p[1],
                        hidden, at[0], at[1]))
    return out


class _Union:
    def __init__(self):
        self.parent = {}

    def find(self, a):
        self.parent.setdefault(a, a)
        while self.parent[a] != a:
            self.parent[a] = self.parent[self.parent[a]]
            a = self.parent[a]
        return a

    def join(self, a, b):
        self.parent[self.find(a)] = self.find(b)


def _on_segment(p, a, b):
    (x, y), (x1, y1), (x2, y2) = p, a, b
    if (x2 - x1) * (y - y1) != (y2 - y1) * (x - x1):
        return False
    return (min(x1, x2) <= x <= max(x1, x2) and
            min(y1, y2) <= y <= max(y1, y2))


def schematic(root):
    """Parse a hierarchical schematic.

    Returns (parts, nets, notes):
      parts  {ref: dict(sheet, lib_id, value, footprint, props, pins)} where
             pins is {number: name}
      nets   {net name: {(ref, pin number)}}
      notes  connectivity oddities worth a human look
    """
    folder = os.path.dirname(os.path.abspath(root))
    files, todo = [], [os.path.basename(root)]
    while todo:
        f = todo.pop(0)
        if f in files:
            continue
        files.append(f)
        doc = sexpr(open(os.path.join(folder, f), encoding="utf-8").read())
        for sh in children(doc, "sheet"):
            sub = props(sh).get("Sheetfile") or props(sh).get("Sheet file")
            if sub:
                todo.append(sub)

    uf, parts, notes = _Union(), {}, []
    names = {}            # union node -> set of net names
    pin_nodes = {}        # (ref, number) -> union node
    for f in files:
        doc = sexpr(open(os.path.join(folder, f), encoding="utf-8").read())
        libs = {L[1]: L for L in children(child(doc, "lib_symbols") or [], "symbol")}
        sheet = os.path.splitext(f)[0]
        points = {}       # point key -> union node on this sheet

        def node(x, y):
            k = _key(x, y)
            return points.setdefault(k, (sheet, k))

        wires = []
        for w in children(doc, "wire"):
            (a, b) = [(float(p[1]), float(p[2]))
                      for p in children(child(w, "pts"), "xy")]
            uf.join(node(*a), node(*b))
            wires.append((_key(*a), _key(*b)))
        junctions = {_key(*nums(j, "at")[:2]) for j in children(doc, "junction")}
        for k in junctions:
            node(k[0] / 100, k[1] / 100)
        for kind in ("label", "global_label", "hierarchical_label"):
            for lab in children(doc, kind):
                at = nums(lab, "at")
                n = node(at[0], at[1])
                scope = sheet + "/" if kind == "label" else ""
                names.setdefault(n, set()).add(scope + lab[1])
        no_connect = {_key(*nums(nc, "at")[:2])
                      for nc in children(doc, "no_connect")}

        for sym in children(doc, "symbol"):
            lib_id = child(sym, "lib_id")
            if not lib_id:
                continue
            pr = props(sym)
            ref = pr.get("Reference", "?")
            for inst in children(child(sym, "instances") or [], "project"):
                for path in children(inst, "path"):
                    r = child(path, "reference")
                    if r:
                        ref = r[1]
            at = nums(sym, "at")
            mirror = (child(sym, "mirror") or [None, None])[1]
            unit = int((child(sym, "unit") or [0, "1"])[1])
            style = int((child(sym, "convert") or child(sym, "body_style")
                         or [0, "1"])[1])
            lib = libs[lib_id[1]]
            pins = _lib_pins(lib, unit, style)
            power = "power" in lib or child(lib, "power") is not None
            for num, name, ptype, hidden, px, py in pins:
                x, y = _pin_xy(at, mirror, px, py)
                n = node(x, y)
                if power or ref.startswith("#"):
                    # Only a power_in pin makes a global net (not PWR_FLAG).
                    if power and ptype == "power_in":
                        names.setdefault(n, set()).add(pr.get("Value") or name)
                    continue
                if hidden and ptype.startswith("power"):
                    notes.append(f"{ref} pin {num} ({name}) is a hidden "
                                 f"power pin")
                if _key(x, y) in no_connect:
                    continue
                pin_nodes[(ref, num)] = n
            if not power and not ref.startswith("#"):
                parts.setdefault(ref, dict(
                    sheet=sheet, lib_id=lib_id[1], value=pr.get("Value"),
                    footprint=pr.get("Footprint"), props=pr, pins={}))
                parts[ref]["pins"].update({p[0]: p[1] for p in pins})

        for k, n in points.items():
            for a, b in wires:
                if k not in (a, b) and _on_segment(k, a, b):
                    uf.join(n, points[a])
                    if k not in junctions:
                        notes.append(f"{sheet}: {k[0] / 100}, {k[1] / 100} "
                                     f"touches a wire mid-span with no "
                                     f"junction")

    first = {}
    for n, ns in names.items():
        for name in ns:
            uf.join(n, first.setdefault(name, n))
    by_root = {}
    for n, ns in names.items():
        by_root.setdefault(uf.find(n), set()).update(ns)
    members = {}
    for pin, n in pin_nodes.items():
        members.setdefault(uf.find(n), set()).add(pin)
    nets = {}
    for r, pins in members.items():
        ns = by_root.get(r)
        if ns and len(ns) > 1:
            notes.append(f"one net carries several names: {sorted(ns)}")
        name = " = ".join(sorted(ns)) if ns else \
            "Net-({}-{})".format(*min(pins))
        nets[name] = pins
    return parts, nets, sorted(set(notes))
