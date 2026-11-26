"""Minimal schematic-symbol drawing library that emits SVG.

Coordinates are in SVG user units, one grid step = 10 units. Every symbol
function returns a dict mapping terminal name -> (x, y) so that nets can be
wired by referring to terminals instead of re-deriving geometry.

`line`, `wire` and `hv` draw electrical wires; symbols draw with `_line`.
Each sheet keeps the wires, symbol terminals and dots it drew, so
`Sheet.problems()` can check the drawing's connectivity, `Sheet.parts`
lists every (ref, val) placed, which is what the BOM is built from, and
`Doc.netlist()` turns the drawing into named nets of (ref, terminal) pairs.
"""

import functools
import inspect
from html import escape

STYLE = """
  .frame  { fill:none; stroke:#222; stroke-width:2; }
  .frame2 { fill:none; stroke:#222; stroke-width:1; }
  .w      { fill:none; stroke:#111; stroke-width:1.6; stroke-linecap:round; }
  .sym    { fill:#fff; stroke:#111; stroke-width:1.6; }
  .symf   { fill:#111; stroke:#111; stroke-width:1.2; }
  .box    { fill:#f2f7fc; stroke:#0a3d62; stroke-width:1.8; }
  .boxc   { fill:#fdf6e3; stroke:#8a6d1f; stroke-width:1.5; }
  .boxn   { fill:#fffdf0; stroke:#b8860b; stroke-width:1.2; }
  .boxw   { fill:#fdecea; stroke:#c0392b; stroke-width:1.5; }
  .boxz   { fill:none; stroke:#7f8c8d; stroke-width:1.2; stroke-dasharray:7 5; }
  .flag   { fill:#eaf4ff; stroke:#0a3d62; stroke-width:1.2; }
  .flagp  { fill:#fdecea; stroke:#c0392b; stroke-width:1.2; }
  text    { font-family:'Helvetica Neue',Helvetica,Arial,sans-serif; fill:#111; }
  .t      { font-size:11px; }
  .ts     { font-size:9.5px; }
  .tb     { font-size:12px; font-weight:600; }
  .th     { font-size:16px; font-weight:700; }
  .tt     { font-size:22px; font-weight:700; }
  .tms    { font-family:'SFMono-Regular',Menlo,Consolas,monospace; font-size:8.5px; }
  .tmb    { font-family:'SFMono-Regular',Menlo,Consolas,monospace; font-size:11px;
            font-weight:600; }
  .ref    { font-size:10.5px; font-weight:700; fill:#7d2d8b; }
  .val    { font-family:'SFMono-Regular',Menlo,Consolas,monospace; font-size:10px;
            fill:#0a5c36; }
  .pwr    { font-size:10.5px; font-weight:700; fill:#c0392b; }
  .pin    { font-family:'SFMono-Regular',Menlo,Consolas,monospace; font-size:9px;
            fill:#0a3d62; }
  .pnum   { font-family:'SFMono-Regular',Menlo,Consolas,monospace; font-size:8px;
            fill:#888; }
  .net    { font-family:'SFMono-Regular',Menlo,Consolas,monospace; font-size:9.5px;
            fill:#1a4f8a; }
  .arw    { fill:none; stroke:#111; stroke-width:1.7; marker-end:url(#ar); }
  .arwp   { fill:none; stroke:#c0392b; stroke-width:2.0; marker-end:url(#arp); }
  .blk    { fill:#eef5fb; stroke:#0a3d62; stroke-width:1.6; }
  .blkp   { fill:#fdecea; stroke:#c0392b; stroke-width:1.6; }
  .blkg   { fill:#eef9f1; stroke:#186a3b; stroke-width:1.6; }
  .blky   { fill:#fdf6e3; stroke:#8a6d1f; stroke-width:1.6; }
"""

DEFS = """<defs>
  <marker id="ar" markerWidth="9" markerHeight="7" refX="8.5" refY="3.5"
          orient="auto"><path d="M0,0 L9,3.5 L0,7 z" fill="#111"/></marker>
  <marker id="arp" markerWidth="9" markerHeight="7" refX="8.5" refY="3.5"
          orient="auto"><path d="M0,0 L9,3.5 L0,7 z" fill="#c0392b"/></marker>
</defs>"""

PAGE_W = 1840
PAGE_H = 1400

EPS = 0.5


def symbol(fn):
    """Register what a symbol method draws: its terminals, the segments that
    make up its body, and its (ref, val) if it has a reference designator."""
    sig = inspect.signature(fn)

    @functools.wraps(fn)
    def draw(self, *a, **k):
        args = sig.bind(self, *a, **k).arguments
        self._segs = []
        try:
            pins = fn(self, *a, **k)
        finally:
            segs, self._segs = self._segs, None
        if isinstance(pins, dict):
            keyed = {k: v for k, v in pins.items() if not k.startswith("_")}
        else:
            keyed = {"1": pins}
        terms = list(keyed.values())
        ref = args.get("ref")
        self.symbols.append((ref or fn.__name__, terms, segs))
        self.placed.append((ref, fn.__name__, keyed, args))
        if ref:
            self.parts.append((ref, args.get("val")))
        return pins
    return draw


def _same(p, q):
    return abs(p[0] - q[0]) < EPS and abs(p[1] - q[1]) < EPS


def _orthogonal(seg):
    (x1, y1), (x2, y2) = seg
    return abs(x1 - x2) < EPS or abs(y1 - y2) < EPS


def _on(p, seg, interior=False):
    """Whether p lies on the orthogonal segment, optionally excluding its ends."""
    (x1, y1), (x2, y2) = seg
    lo_x, hi_x = sorted((x1, x2))
    lo_y, hi_y = sorted((y1, y2))
    inside = (lo_x - EPS <= p[0] <= hi_x + EPS and
              lo_y - EPS <= p[1] <= hi_y + EPS)
    if not inside or not _orthogonal(seg):
        return False
    return not (interior and (_same(p, seg[0]) or _same(p, seg[1])))


def _meet(a, b):
    """Points where two orthogonal segments touch: one point for a crossing
    or a touch, both ends of the shared stretch for a colinear overlap."""
    if not (_orthogonal(a) and _orthogonal(b)):
        return []
    shared = [p for p in (*a, *b) if _on(p, a) and _on(p, b)]
    if shared:
        return shared
    va = abs(a[0][0] - a[1][0]) < EPS
    vb = abs(b[0][0] - b[1][0]) < EPS
    if va == vb:
        return []
    p = (a[0][0], b[0][1]) if va else (b[0][0], a[0][1])
    return [p] if _on(p, a) and _on(p, b) else []


class Sheet:
    """One schematic page."""

    def __init__(self, number, title, subtitle=""):
        self.number = number
        self.title = title
        self.subtitle = subtitle
        self.el = []
        self.wires = []
        self.symbols = []
        self.placed = []
        self.junctions = []
        self.parts = []
        self._segs = None

    # ---------------------------------------------------------------- raw SVG
    def raw(self, s):
        self.el.append(s)

    def _line(self, x1, y1, x2, y2, cls="w"):
        if self._segs is not None:
            self._segs.append(((x1, y1), (x2, y2)))
        self.raw(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" class="{cls}"/>')

    def _poly(self, pts, cls="w"):
        if self._segs is not None:
            self._segs.extend(zip(pts, pts[1:]))
        d = " ".join(f"{x},{y}" for x, y in pts)
        self.raw(f'<polyline points="{d}" class="{cls}"/>')

    def rect(self, x, y, w, h, cls="sym", rx=0):
        if self._segs is not None:
            c = [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]
            self._segs.extend(zip(c, c[1:]))
        self.raw(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" '
                 f'class="{cls}"/>')

    def circle(self, x, y, r, cls="sym"):
        self.raw(f'<circle cx="{x}" cy="{y}" r="{r}" class="{cls}"/>')

    def path(self, d, cls="w"):
        self.raw(f'<path d="{d}" class="{cls}"/>')

    def text(self, x, y, s, cls="t", anchor="start"):
        self.raw(f'<text x="{x}" y="{y}" class="{cls}" text-anchor="{anchor}">'
                 f'{escape(str(s))}</text>')

    # ------------------------------------------------------------------ wires
    def line(self, x1, y1, x2, y2):
        self.wire((x1, y1), (x2, y2))

    def wire(self, *pts):
        """Orthogonal wire through a list of (x, y) points."""
        assert self._segs is None, "symbols draw with _line, not wires"
        pts = [tuple(p) for p in pts]
        self.wires.extend(zip(pts, pts[1:]))
        self._poly(pts)

    def hv(self, p, q):
        """Route from p to q horizontally then vertically."""
        self.wire(p, (q[0], p[1]), q)

    def dot(self, x, y):
        self.junctions.append((x, y))
        self.raw(f'<circle cx="{x}" cy="{y}" r="3" class="symf"/>')

    def dots(self, *pts):
        for x, y in pts:
            self.dot(x, y)

    # ---------------------------------------------------- connection symbols
    @symbol
    def gnd(self, x, y):
        self._line(x, y, x, y + 12)
        self._line(x - 12, y + 12, x + 12, y + 12)
        self._line(x - 7, y + 17, x + 7, y + 17)
        self._line(x - 3, y + 22, x + 3, y + 22)
        return (x, y)

    @symbol
    def rail(self, x, y, label):
        """Power rail flag, stub pointing up."""
        self._line(x, y, x, y - 12)
        self._line(x - 13, y - 12, x + 13, y - 12)
        self.text(x, y - 17, label, "pwr", "middle")
        return (x, y)

    @symbol
    def netlabel(self, x, y, name, side="r", cls="flag"):
        """Net label flag attached to a wire end at (x, y)."""
        w = 7.0 * len(name) + 16
        if side == "r":
            self._line(x, y, x + 8, y)
            self.path(f"M{x+8},{y} l7,-8 h{w} v16 h{-w} z", cls)
            self.text(x + 19, y + 3.5, name, "net")
        else:
            self._line(x - 8, y, x, y)
            self.path(f"M{x-8},{y} l-7,-8 h{-w} v16 h{w} z", cls)
            self.text(x - 19, y + 3.5, name, "net", "end")
        return (x, y)

    @symbol
    def nc(self, x, y, note, side="l"):
        """No-connect: a short stub off a pin end, an X and a note."""
        s = -1 if side == "l" else 1
        ex = x + 14 * s
        self._line(x, y, ex, y)
        self._line(ex - 6, y - 6, ex + 6, y + 6)
        self._line(ex - 6, y + 6, ex + 6, y - 6)
        self.text(x + 30 * s, y + 3.5, note, "tms",
                  "end" if side == "l" else "start")
        return (x, y)

    @symbol
    def tp(self, x, y, ref, side="r"):
        self.circle(x, y, 4.5)
        self.circle(x, y, 1.8, "symf")
        if side == "r":
            self.text(x + 9, y + 3.5, ref, "tms")
        elif side == "l":
            self.text(x - 9, y + 3.5, ref, "tms", "end")
        else:
            self.text(x, y - 9, ref, "tms", "middle")
        return (x, y)

    # ---------------------------------------------------------------- passives
    @symbol
    def res_h(self, x, y, ref, val, L=64):
        lead, bh = 15, 15
        self._line(x, y, x + lead, y)
        self.rect(x + lead, y - bh / 2, L - 2 * lead, bh)
        self._line(x + L - lead, y, x + L, y)
        cx = x + L / 2
        self.text(cx, y - 13, ref, "ref", "middle")
        self.text(cx, y + 22, val, "val", "middle")
        return {"1": (x, y), "2": (x + L, y)}

    @symbol
    def res_v(self, x, y, ref, val, L=64, side="r"):
        lead, bw = 15, 15
        self._line(x, y, x, y + lead)
        self.rect(x - bw / 2, y + lead, bw, L - 2 * lead)
        self._line(x, y + L - lead, x, y + L)
        cy = y + L / 2
        if side == "r":
            self.text(x + 13, cy - 2, ref, "ref")
            self.text(x + 13, cy + 11, val, "val")
        else:
            self.text(x - 13, cy - 2, ref, "ref", "end")
            self.text(x - 13, cy + 11, val, "val", "end")
        return {"1": (x, y), "2": (x, y + L)}

    @symbol
    def cap_v(self, x, y, ref, val, L=52, pol=False, side="r"):
        g = 5
        my = y + L / 2
        self._line(x, y, x, my - g)
        self._line(x - 15, my - g, x + 15, my - g)
        if pol:
            self.path(f"M{x-15},{my+g+4} q15,-9 30,0", "sym")
            self.text(x - 20, my - g - 4, "+", "tb", "end")
        else:
            self._line(x - 15, my + g, x + 15, my + g)
        self._line(x, my + g + (4 if pol else 0), x, y + L)
        if side == "r":
            self.text(x + 20, my - 3, ref, "ref")
            self.text(x + 20, my + 10, val, "val")
        elif side == "l":
            self.text(x - 20, my - 3, ref, "ref", "end")
            self.text(x - 20, my + 10, val, "val", "end")
        else:  # below the ground symbol
            self.text(x, y + L + 36, ref, "ref", "middle")
            self.text(x, y + L + 48, val, "val", "middle")
        return {"1": (x, y), "2": (x, y + L)}

    @symbol
    def cap_h(self, x, y, ref, val, L=52):
        g = 5
        mx = x + L / 2
        self._line(x, y, mx - g, y)
        self._line(mx - g, y - 15, mx - g, y + 15)
        self._line(mx + g, y - 15, mx + g, y + 15)
        self._line(mx + g, y, x + L, y)
        self.text(mx, y + 30, ref, "ref", "middle")
        self.text(mx, y + 42, val, "val", "middle")
        return {"1": (x, y), "2": (x + L, y)}

    @symbol
    def ind_h(self, x, y, ref, val, L=72):
        lead = 8
        r = (L - 2 * lead) / 8
        self._line(x, y, x + lead, y)
        self.path(f"M{x+lead},{y}" + f" a{r},{r} 0 0 1 {2*r},0" * 4, "sym")
        self._line(x + L - lead, y, x + L, y)
        self.text(x + L / 2, y - 14, ref, "ref", "middle")
        self.text(x + L / 2, y + 20, val, "val", "middle")
        return {"1": (x, y), "2": (x + L, y)}

    # ----------------------------------------------------------------- diodes
    @symbol
    def diode_v(self, x, y, ref, val, L=52, kind="d", flip=False):
        """Vertical diode.

        flip=False -> anode at top pin '1', cathode at bottom pin '2'.
        flip=True  -> cathode at top pin '1', anode at bottom pin '2'.
        """
        my = y + L / 2
        base = my + (-11 if not flip else 11)
        apex = my + (5 if not flip else -5)
        self.path(f"M{x-14},{base} L{x+14},{base} L{x},{apex} z", "sym")
        self._line(x - 14, apex, x + 14, apex)
        if not flip:
            self._line(x, y, x, base)
            self._line(x, apex, x, y + L)
        else:
            self._line(x, y, x, apex)
            self._line(x, base, x, y + L)
        s = 7 if not flip else -7
        if kind == "z":
            self._line(x - 14, apex, x - 14, apex + s)
            self._line(x + 14, apex, x + 14, apex - s)
        elif kind == "s":
            self._poly([(x - 14, apex + s), (x - 14, apex), (x + 14, apex),
                        (x + 14, apex - s)], "sym")
        if kind == "led":
            self.path(f"M{x+17},{my-12} l11,-9 m0,0 l-4.5,0 m4.5,0 l0,4.5", "w")
            self.path(f"M{x+17},{my-1} l11,-9 m0,0 l-4.5,0 m4.5,0 l0,4.5", "w")
        tx = x + (34 if kind == "led" else 22)
        self.text(tx, my - 3, ref, "ref")
        self.text(tx, my + 10, val, "val")
        return {"1": (x, y), "2": (x, y + L)}

    @symbol
    def diode_h(self, x, y, ref, val, L=52, kind="d", below=True, flip=False):
        """Horizontal diode.

        flip=False -> anode at left pin '1'. flip=True -> cathode at left.
        """
        mx = x + L / 2
        base = mx + (-11 if not flip else 11)
        apex = mx + (5 if not flip else -5)
        self.path(f"M{base},{y-14} L{base},{y+14} L{apex},{y} z", "sym")
        self._line(apex, y - 14, apex, y + 14)
        if not flip:
            self._line(x, y, base, y)
            self._line(apex, y, x + L, y)
        else:
            self._line(x, y, apex, y)
            self._line(base, y, x + L, y)
        s = 7 if not flip else -7
        if kind == "s":
            self._poly([(apex + s, y - 14), (apex, y - 14), (apex, y + 14),
                        (apex - s, y + 14)], "sym")
        if kind == "led":
            self.path(f"M{mx-6},{y-17} l9,-11 m0,0 l0,4.5 m0,-4.5 l-4.5,0", "w")
            self.path(f"M{mx+5},{y-17} l9,-11 m0,0 l0,4.5 m0,-4.5 l-4.5,0", "w")
        if below:
            self.text(mx, y + 28, ref, "ref", "middle")
            self.text(mx, y + 40, val, "val", "middle")
        else:
            dy = -34 if kind == "led" else -22
            self.text(mx, y + dy, ref, "ref", "middle")
            self.text(mx, y + dy + 12, val, "val", "middle")
        return {"1": (x, y), "2": (x + L, y)}

    # --------------------------------------------------------------- switches
    @symbol
    def mosfet(self, x, y, ref, val):
        """N-channel MOSFET. Gate lead enters from the left at (x, y+34).

        Terminals: G, D (top right), S (bottom right).
        """
        gx, cx2, rx = x + 22, x + 32, x + 66
        self._line(x, y + 34, gx, y + 34)
        self._line(gx, y + 14, gx, y + 54)
        self._line(cx2, y + 6, cx2, y + 62)
        self._line(cx2, y + 14, rx, y + 14)
        self._line(rx, y + 14, rx, y)
        self._line(cx2, y + 54, rx, y + 54)
        self._line(rx, y + 54, rx, y + 68)
        self._line(cx2, y + 34, rx - 14, y + 34)
        self._line(rx - 14, y + 34, rx - 14, y + 54)
        ax = cx2 + 12
        self.path(f"M{ax+9},{y+28} L{ax-1},{y+34} L{ax+9},{y+40} z", "symf")
        self.text(rx + 12, y + 30, ref, "ref")
        self.text(rx + 12, y + 43, val, "val")
        self.text(rx + 5, y + 8, "D", "pin")
        self.text(rx + 5, y + 62, "S", "pin")
        self.text(x + 4, y + 30, "G", "pin")
        return {"G": (x, y + 34), "D": (rx, y), "S": (rx, y + 68)}

    @symbol
    def button(self, x, y, ref, val, L=56):
        """Momentary push button, horizontal, pins '1' and '2'."""
        self._line(x, y, x + 14, y)
        self._line(x + L - 14, y, x + L, y)
        self._line(x + 14, y, x + 14, y - 6)
        self._line(x + L - 14, y, x + L - 14, y - 6)
        self._line(x + 8, y - 12, x + L - 8, y - 12)
        self._line(x + L / 2, y - 12, x + L / 2, y - 20)
        self._line(x + L / 2 - 8, y - 20, x + L / 2 + 8, y - 20)
        self.text(x + L / 2, y + 16, ref, "ref", "middle")
        self.text(x + L / 2, y + 28, val, "val", "middle")
        return {"1": (x, y), "2": (x + L, y)}

    @symbol
    def jumper(self, x, y, ref, val, L=46):
        """Fitted solder jumper / link, horizontal."""
        self._line(x, y, x + 12, y)
        self._line(x + L - 12, y, x + L, y)
        self.rect(x + 12, y - 7, L - 24, 14, "sym", rx=3)
        self._line(x + 12, y, x + L - 12, y)
        self.text(x + L / 2, y - 14, ref, "ref", "middle")
        self.text(x + L / 2, y + 24, val, "val", "middle")
        return {"1": (x, y), "2": (x + L, y)}

    @symbol
    def fuse(self, x, y, ref, val, L=64):
        self._line(x, y, x + 12, y)
        self.rect(x + 12, y - 11, L - 24, 22, "sym", rx=4)
        self._line(x + 12, y, x + L - 12, y)
        self._line(x + L - 12, y, x + L, y)
        self.text(x + L / 2, y - 17, ref, "ref", "middle")
        self.text(x + L / 2, y + 26, val, "val", "middle")
        return {"1": (x, y), "2": (x + L, y)}

    # ------------------------------------------------------------- containers
    @symbol
    def ic(self, x, y, w, h, ref, val, left=(), right=(), top=(), bottom=()):
        """Rectangular IC body with named pins.

        Pin entries are (number, name) or (number, name, key) tuples. Pins are
        returned under their key, which defaults to the name; a repeated key
        raises, so give same-named pins (two GNDs) distinct keys.
        """
        self.rect(x, y, w, h, "box", rx=4)
        self.text(x + w / 2, y - 20, ref, "ref", "middle")
        self.text(x + w / 2, y - 7, val, "tmb", "middle")
        pins = {}
        lead = 20

        def add(entry, p):
            num, name, key = (entry + (entry[1],))[:3]
            if key in pins:
                raise ValueError(f"{ref}: duplicate pin key {key!r}")
            pins[key] = p
            return num, name

        for i, e in enumerate(left):
            py = y + h / (len(left) + 1) * (i + 1)
            num, name = add(e, (x - lead, py))
            self._line(x - lead, py, x, py)
            self.text(x + 6, py + 3.5, name, "pin")
            self.text(x - 4, py - 4, num, "pnum", "end")
        for i, e in enumerate(right):
            py = y + h / (len(right) + 1) * (i + 1)
            num, name = add(e, (x + w + lead, py))
            self._line(x + w, py, x + w + lead, py)
            self.text(x + w - 6, py + 3.5, name, "pin", "end")
            self.text(x + w + 4, py - 4, num, "pnum")
        for i, e in enumerate(top):
            px = x + w / (len(top) + 1) * (i + 1)
            num, name = add(e, (px, y - lead))
            self._line(px, y - lead, px, y)
            self.text(px, y + 14, name, "pin", "middle")
            self.text(px + 4, y - 6, num, "pnum")
        for i, e in enumerate(bottom):
            px = x + w / (len(bottom) + 1) * (i + 1)
            num, name = add(e, (px, y + h + lead))
            self._line(px, y + h, px, y + h + lead)
            self.text(px, y + h - 8, name, "pin", "middle")
            self.text(px + 4, y + h + 13, num, "pnum")
        return pins

    @symbol
    def conn(self, x, y, ref, val, pins, w=104, side="right", pitch=26):
        """Wire-to-board connector. Pin leads exit on `side`."""
        h = pitch * len(pins) + 18
        self.rect(x, y, w, h, "boxc", rx=3)
        self.text(x + w / 2, y - 20, ref, "ref", "middle")
        self.text(x + w / 2, y - 7, val, "tmb", "middle")
        out = {}
        lead = 20
        for i, name in enumerate(pins):
            py = y + 9 + pitch * i + pitch / 2
            self.rect(x + (w - 12 if side == "right" else 2), py - 5, 10, 10)
            if side == "right":
                self._line(x + w, py, x + w + lead, py)
                self.text(x + w - 18, py + 3.5, name, "pin", "end")
                self.text(x + 6, py + 3.5, i + 1, "pnum")
                out[name] = (x + w + lead, py)
            else:
                self._line(x - lead, py, x, py)
                self.text(x + 18, py + 3.5, name, "pin")
                self.text(x + w - 6, py + 3.5, i + 1, "pnum", "end")
                out[name] = (x - lead, py)
        return out

    # ------------------------------------------------------------ annotations
    def blk(self, x, y, w, h, title, lines=(), cls="blk", tcls="tb"):
        self.rect(x, y, w, h, cls, rx=4)
        self.text(x + w / 2, y + (h / 2 + 4 if not lines else 17), title, tcls,
                  "middle")
        for i, ln in enumerate(lines):
            self.text(x + w / 2, y + 31 + 12 * i, ln, "ts", "middle")
        return {"x": x, "y": y, "w": w, "h": h}

    def arrow(self, *pts, cls="arw"):
        self._poly(list(pts), cls)

    def zone(self, x, y, w, h, title):
        self.rect(x, y, w, h, "boxz", rx=6)
        self.text(x + 10, y + 16, title, "tb")

    def note(self, x, y, w, title, lines, cls="boxn"):
        h = 26 + 13 * len(lines)
        self.rect(x, y, w, h, cls, rx=4)
        self.text(x + 10, y + 17, title, "tb")
        for i, ln in enumerate(lines):
            self.text(x + 10, y + 33 + 13 * i, ln, "ts")
        return h

    def table(self, x, y, headers, rows, widths, title=None, rh=18):
        if title:
            self.text(x, y - 8, title, "tb")
        w = sum(widths)
        h = rh * (len(rows) + 1)
        self.rect(x, y, w, h, "boxc", rx=3)
        cx = x
        for wd in widths[:-1]:
            cx += wd
            self._line(cx, y, cx, y + h, "frame2")
        for i in range(len(rows)):
            self._line(x, y + rh * (i + 1), x + w, y + rh * (i + 1), "frame2")
        cx = x
        for i, hd in enumerate(headers):
            self.text(cx + 6, y + rh - 5, hd, "tb")
            cx += widths[i]
        for r, row in enumerate(rows):
            cx = x
            for i, cell in enumerate(row):
                self.text(cx + 6, y + rh * (r + 2) - 5, cell, "tms")
                cx += widths[i]
        return h

    # ------------------------------------------------------------ connectivity
    def problems(self):
        """Drawing errors that would make the picture lie about the netlist."""
        out = []
        terms = [(ref, p) for ref, ps, _ in self.symbols for p in ps]

        def on_wire(p, skip=None, interior=False):
            return any(_on(p, w, interior) for w in self.wires if w is not skip)

        def dotted(p):
            return any(_same(p, d) for d in self.junctions)

        for w in self.wires:
            if not _orthogonal(w):
                out.append(f"wire {w} is not orthogonal")
            for p in w:
                if not (any(_same(p, q) for _, q in terms) or on_wire(p, w)):
                    out.append(f"wire end {p} is not connected")
                if on_wire(p, w, interior=True) and not dotted(p):
                    out.append(f"T-junction at {p} has no dot")
        for ref, p in terms:
            if not (on_wire(p) or any(_same(p, q) for r, q in terms
                                      if r != ref)):
                out.append(f"{ref} terminal {p} is not connected")
            elif on_wire(p, interior=True) and not dotted(p):
                out.append(f"{ref} terminal {p} sits mid-wire with no dot")
        for ref, ps, segs in self.symbols:
            for w in self.wires:
                for seg in segs:
                    for p in _meet(w, seg):
                        if not any(_same(p, q) for q in ps):
                            out.append(f"wire {w} runs through {ref} at {p}")
        for d in self.junctions:
            joins = sum(2 if _on(d, w, interior=True) else 1
                        for w in self.wires if _on(d, w))
            joins += sum(1 for _, q in terms if _same(d, q))
            if joins < 3:
                out.append(f"dot at {d} joins only {joins} ends")
        return sorted(set(out))

    def nets(self):
        """Connected groups on this sheet as (names, members, no_connect).

        A terminal or wire end joins a wire anywhere along it; wires that
        merely cross do not join. `names` come from net labels, rails and
        ground symbols; `members` are (ref, terminal key) of real parts.
        """
        points = [p for w in self.wires for p in w]
        points += [p for _, _, pins, _ in self.placed for p in pins.values()]
        parent = list(range(len(points)))

        def find(i):
            while parent[i] != i:
                parent[i] = parent[parent[i]]
                i = parent[i]
            return i

        for i, p in enumerate(points):
            for j in range(i):
                if _same(p, points[j]):
                    parent[find(i)] = find(j)
        for w, seg in enumerate(self.wires):
            for i, p in enumerate(points):
                if _on(p, seg):
                    parent[find(i)] = find(2 * w)
        groups = {}
        index = {p: i for i, p in enumerate(points)}
        for ref, kind, pins, args in self.placed:
            for key, p in pins.items():
                names, members, nc = groups.setdefault(
                    find(index[p]), (set(), set(), [False]))
                if kind == "gnd":
                    names.add("GND")
                elif kind == "rail":
                    names.add(args["label"])
                elif kind == "netlabel":
                    names.add(args["name"])
                elif kind == "nc":
                    nc[0] = True
                elif ref:
                    members.add((ref, key))
        return [(n, m, nc[0]) for n, m, nc in groups.values()]

    # ------------------------------------------------------------------ frame
    def render(self, project, rev, total):
        f = []
        f.append(f'<rect x="14" y="14" width="{PAGE_W-28}" height="{PAGE_H-28}" '
                 f'class="frame"/>')
        f.append(f'<rect x="22" y="22" width="{PAGE_W-44}" height="{PAGE_H-44}" '
                 f'class="frame2"/>')
        tbx, tby, tbw, tbh = PAGE_W - 552, PAGE_H - 122, 530, 100
        f.append(f'<rect x="{tbx}" y="{tby}" width="{tbw}" height="{tbh}" '
                 f'class="frame"/>')
        f.append(f'<line x1="{tbx}" y1="{tby+34}" x2="{tbx+tbw}" y2="{tby+34}" '
                 f'class="frame2"/>')
        f.append(f'<line x1="{tbx}" y1="{tby+66}" x2="{tbx+tbw}" y2="{tby+66}" '
                 f'class="frame2"/>')
        f.append(f'<line x1="{tbx+400}" y1="{tby+66}" x2="{tbx+400}" '
                 f'y2="{tby+tbh}" class="frame2"/>')
        f.append(f'<text x="{tbx+10}" y="{tby+24}" class="th">{escape(project)}'
                 f'</text>')
        f.append(f'<text x="{tbx+10}" y="{tby+56}" class="tb">'
                 f'{escape(self.title)}</text>')
        f.append(f'<text x="{tbx+10}" y="{tby+88}" class="ts">'
                 f'{escape(self.subtitle)}</text>')
        f.append(f'<text x="{tbx+410}" y="{tby+88}" class="tb">Sheet '
                 f'{self.number}/{total} &#183; Rev {rev}</text>')
        f.append(f'<text x="40" y="52" class="tt">{escape(self.title)}</text>')
        return "\n".join(f) + "\n" + "\n".join(self.el)


class Doc:
    def __init__(self, project, rev):
        self.project = project
        self.rev = rev
        self.sheets = []

    def sheet(self, title, subtitle=""):
        s = Sheet(len(self.sheets) + 1, title, subtitle)
        self.sheets.append(s)
        return s

    def problems(self):
        return [f"sheet {s.number}: {p}" for s in self.sheets
                for p in s.problems()]

    def netlist(self):
        """{net name: {(ref, terminal key)}} over the whole document.

        Net names are global, so a label on one sheet joins the same label
        on every other; a net carrying several names is keyed "A = B".
        Unlabelled nets are named after their first member; no-connect stubs
        and lone labels are left out.
        """
        named, alias = {}, {}

        def root(n):
            while alias.get(n, n) != n:
                n = alias[n]
            return n

        unnamed = []
        for s in self.sheets:
            for names, members, nc in s.nets():
                if not names:
                    if members and not (nc and len(members) == 1):
                        unnamed.append(members)
                    continue
                first, *rest = sorted({root(n) for n in names})
                for n in rest:
                    alias[n] = first
                    named.setdefault(first, set()).update(named.pop(n, set()))
                named.setdefault(first, set()).update(members)
        spelled = {}
        for n in {*alias, *named}:
            spelled.setdefault(root(n), []).append(n)
        out = {" = ".join(sorted(spelled[n])): m for n, m in named.items() if m}
        for members in unnamed:
            ref, key = min(members)
            out[f"Net-({ref}-{key})"] = members
        return out

    def svg(self):
        n = len(self.sheets)
        total_h = PAGE_H * n
        out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{PAGE_W}" '
               f'height="{total_h}" viewBox="0 0 {PAGE_W} {total_h}">',
               f'<style>{STYLE}</style>', DEFS,
               f'<rect width="{PAGE_W}" height="{total_h}" fill="#ffffff"/>']
        for i, s in enumerate(self.sheets):
            out.append(f'<g transform="translate(0,{PAGE_H*i})">')
            out.append(s.render(self.project, self.rev, n))
            out.append('</g>')
        out.append('</svg>')
        return "\n".join(out)
