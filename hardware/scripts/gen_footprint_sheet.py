#!/usr/bin/env python3
"""Build a printable parts list and a 1:1 footprint check sheet.

Each line of omnimotion_BOM.csv names the footprint its part needs, and is
matched, reference by reference, to the footprints placed on the KiCad PCB
(kicad/omnimotion.kicad_pcb). Every distinct footprint is drawn once, at true
scale, so the real part can be laid on the printout to check its land
pattern. Where the PCB carries the right footprint the drawing is the PCB's
own copy, compared pad by pad with the library; where it does not, the
drawing comes from the library and the card is flagged in red. Libraries are
the KiCad project's .pretty folders or the official KiCad library
(downloaded once into build/footprints/, skipped when offline).

Outputs omnimotion_footprints.html and, when Google Chrome is installed,
omnimotion_footprints.pdf. Print at 100 % / "Actual size" and check the
50 mm ruler on page 1 with a real ruler before trusting any footprint.

Run:  python3 scripts/gen_footprint_sheet.py
"""
import collections
import csv
import html
import math
import os
import re
import shutil
import subprocess
import urllib.error
import urllib.parse
import urllib.request

from kicadlib import child, children, isnum, nums, props, sexpr

HW = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BOM = os.path.join(HW, "generated", "omnimotion_BOM.csv")
KICAD = os.path.join(HW, "kicad")
PCB = os.path.join(KICAD, "omnimotion.kicad_pcb")
CACHE = os.path.join(HW, "build", "footprints")
OUT_HTML = os.path.join(HW, "generated", "omnimotion_footprints.html")
OUT_PDF = os.path.join(HW, "generated", "omnimotion_footprints.pdf")
LIB_URL = ("https://gitlab.com/kicad/libraries/kicad-footprints/-/raw/master/"
           "{lib}.pretty/{name}.kicad_mod")
SYSTEM_FOOTPRINTS = ("/usr/share/kicad/footprints",
                     "/usr/local/share/kicad/footprints")
CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

# BOM lines that are not placed on the PCB.
NO_FOOTPRINT = {"PCB", "HS1-HS3"}

# Things to look at on paper, keyed by manufacturer P/N. "!" notes are
# problems, the rest are reminders.
NOTES = {
    "KLDX-0202-AC": "Rated 3.5 A; use J2 above that.",
    "ESP32-S3-WROOM-1-N16R8": "U4 has twelve 0.30 mm thermal drills in "
        "the original 0.60 mm pads under its exposed ground land.",
    "USB4105-GF-A": "Project copy of the KiCad footprint: the four ground "
        "lands are shortened from 1.15 to 1.00 mm, centres unchanged.",
    "EEE-FK1H221P": "KiCad has no Panasonic-specific 10 mm land; this is the "
        "Vishay 1010 one. Lay the 10 × 10.2 mm can on it.",
    "1206L050/15YR": "Generic 1206 land; Littelfuse 1206L pads are similar.",
    "1206L010/60WR": "Generic 1206 land; Littelfuse 1206L pads are similar.",
}


def expand(refs):
    """'R1 R3' -> [R1, R3]; 'MH1-MH4' -> [MH1, MH2, MH3, MH4]."""
    out = []
    for r in refs.split():
        m = re.fullmatch(r"([A-Z]+)(\d+)-\1?(\d+)", r)
        if m:
            out += [f"{m.group(1)}{i}"
                    for i in range(int(m.group(2)), int(m.group(3)) + 1)]
        else:
            out.append(r)
    return out


def placed_footprints():
    """{ref: footprint node} from the PCB, empty when there is no PCB."""
    if not os.path.exists(PCB):
        return {}
    with open(PCB, encoding="utf-8") as f:
        board = sexpr(f.read())
    return {props(fp).get("Reference"): fp
            for fp in children(board, "footprint")}


def capacitor_rating(row):
    """'50 V X7R' for a capacitor line, '' for anything else."""
    if not row["References"].startswith("C"):
        return ""
    desc = row["Description"]
    kind = re.search(r"\b(X[57][RS]|C0G|NP0)\b", desc)
    kind = (kind.group(1) if kind else "tantalum" if "Tantalum" in desc
            else "electrolytic" if "electrolytic" in desc else "")
    return " ".join(filter(None, (row["Rating verified against datasheet"],
                                  kind)))


def bom_lines(placed):
    """One entry per (BOM line, footprint): a line whose references need or
    sit on different footprints, like the breakaway header strip, is split.
    `fp` is the footprint the part needs, `pcb` the one placed on the PCB."""
    with open(BOM, newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    lines = []
    for row in rows:
        base = dict(item=row["Item"], value=row["Value"],
                    rating=capacitor_rating(row),
                    desc=row["Description"], mfr=row["Manufacturer"],
                    mpn=row["Manufacturer P/N"], pkg=row["Package"],
                    note=NOTES.get(row["Manufacturer P/N"], ""))
        if row["References"] in NO_FOOTPRINT:
            lines.append(dict(base, refs=row["References"],
                              qty=int(row["Qty"]), fp=None, pcb=None,
                              missing=False))
            continue
        wanted = row["Footprint"].split(" / ") if row["Footprint"] else []
        groups = collections.OrderedDict()
        for ref in expand(row["References"]):
            pcb = placed[ref][1] if ref in placed else None
            want = pcb if pcb in wanted or not wanted else wanted[0]
            groups.setdefault((want, pcb), []).append(ref)
        for (want, pcb), refs in groups.items():
            lines.append(dict(base, refs=" ".join(refs), qty=len(refs),
                              fp=want, pcb=pcb, missing=pcb is None))
    return lines


# ---------------------------------------------------------------------------
# library copies
# ---------------------------------------------------------------------------
def library_copy(fp):
    """The library footprint `fp` names, or None when it cannot be had."""
    lib, name = fp.split(":")
    filename = name + ".kicad_mod"
    paths = [os.path.join(base, lib + ".pretty", filename)
             for base in (KICAD, *SYSTEM_FOOTPRINTS)]
    for path in (*paths, os.path.join(CACHE, lib, filename)):
        if os.path.exists(path):
            break
    if not os.path.exists(path):
        url = LIB_URL.format(lib=urllib.parse.quote(lib),
                             name=urllib.parse.quote(name))
        try:
            with urllib.request.urlopen(url, timeout=30) as r:
                data = r.read()
        except (urllib.error.URLError, OSError):
            return None
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)
    with open(path, encoding="utf-8") as f:
        return sexpr(f.read())


def footprint_angle(mod):
    at = nums(mod, "at")
    return at[2] if at and len(at) > 2 else 0.0


def pad_signature(mod):
    """Pads in the footprint's own frame, ignoring nets and placement."""
    turn = footprint_angle(mod)
    out = collections.defaultdict(list)
    for p in children(mod, "pad"):
        at = nums(p, "at")
        rr = nums(p, "roundrect_rratio")
        out[p[1]].append((
            p[2], p[3], round(at[0], 3), round(at[1], 3),
            round(((at[2] if len(at) > 2 else 0) - turn) % 180, 1),
            tuple(round(v, 3) for v in nums(p, "size")),
            str(child(p, "drill")), rr and round(rr[0], 3)))
    return {k: sorted(v) for k, v in out.items()}


def library_difference(mod, fp):
    """Pad numbers whose geometry differs from the library, or None when the
    library copy is unavailable."""
    lib = library_copy(fp)
    if lib is None:
        return None
    a, b = pad_signature(mod), pad_signature(lib)
    if fp == "RF_Module:ESP32-S3-WROOM-1":
        ground_land = b.get("41", [])
        thermal_vias = [p for p in ground_land
                        if p[0] == "thru_hole" and p[5] == (0.6, 0.6)]
        if len(thermal_vias) == 12:
            # Keep checking the pad size, position and hole count. Only the
            # specified board-level thermal drill enlargement is intentional.
            enlarged = [p[:6] + ("['drill', '0.3']",) + p[7:]
                        if p in thermal_vias else p for p in ground_land]
            b["41"] = sorted(enlarged)
    return sorted((n for n in set(a) | set(b) if a.get(n) != b.get(n)),
                  key=lambda n: (len(n), n))


# ---------------------------------------------------------------------------
# SVG drawing, all coordinates in mm
# ---------------------------------------------------------------------------
COPPER = "#4a4a4a"
LAYER_STYLE = {
    "F.Fab": 'stroke="#1f66c9" stroke-width="0.12"',
    "F.SilkS": 'stroke="#000" stroke-width="0.12"',
    "F.CrtYd": 'stroke="#999" stroke-width="0.05" stroke-dasharray="0.3 0.2"',
}


class Box:
    def __init__(self):
        self.x0 = self.y0 = math.inf
        self.x1 = self.y1 = -math.inf

    def add(self, x, y):
        self.x0, self.x1 = min(self.x0, x), max(self.x1, x)
        self.y0, self.y1 = min(self.y0, y), max(self.y1, y)


def f(v):
    return f"{v:.3f}".rstrip("0").rstrip(".")


def rot(x, y, deg):
    a = math.radians(-deg)
    return x * math.cos(a) - y * math.sin(a), x * math.sin(a) + y * math.cos(a)


def stroke_width(node, default):
    s = child(node, "stroke")
    w = nums(s, "width") if s else nums(node, "width")
    return w[0] if w and w[0] > 0 else default


def filled(node):
    fl = child(node, "fill")
    return bool(fl) and fl[1] in ("solid", "yes")


def arc_path(start, mid, end):
    (ax, ay), (bx, by), (cx, cy) = start, mid, end
    d = 2 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by))
    if abs(d) < 1e-9:
        return f"M{f(ax)},{f(ay)} L{f(cx)},{f(cy)}"
    ux = ((ax**2 + ay**2) * (by - cy) + (bx**2 + by**2) * (cy - ay)
          + (cx**2 + cy**2) * (ay - by)) / d
    uy = ((ax**2 + ay**2) * (cx - bx) + (bx**2 + by**2) * (ax - cx)
          + (cx**2 + cy**2) * (bx - ax)) / d
    r = math.hypot(ax - ux, ay - uy)
    cross = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
    sweep = 1 if cross > 0 else 0
    a0, a2 = math.atan2(ay - uy, ax - ux), math.atan2(cy - uy, cx - ux)
    span = (a2 - a0) % (2 * math.pi) if sweep else (a0 - a2) % (2 * math.pi)
    large = 1 if span > math.pi else 0
    return (f"M{f(ax)},{f(ay)} A{f(r)},{f(r)} 0 {large} {sweep} "
            f"{f(cx)},{f(cy)}")


def graphic(node, box, prefix="fp_"):
    """Footprint graphics take their stroke from the enclosing layer group;
    pad primitives (prefix "gr_") are drawn in copper."""
    kind = node[0][len(prefix):]
    if prefix == "gr_":
        fill = COPPER if filled(node) or kind == "poly" else "none"
        attrs = (f'stroke="{COPPER}" stroke-width="{f(stroke_width(node, 0))}"'
                 f' fill="{fill}"')
    else:
        attrs = 'fill="#ccc"' if filled(node) else ""
    if kind == "line":
        (x0, y0), (x1, y1) = nums(node, "start"), nums(node, "end")
        box.add(x0, y0), box.add(x1, y1)
        return (f'<line x1="{f(x0)}" y1="{f(y0)}" x2="{f(x1)}" y2="{f(y1)}" '
                f'{attrs} stroke-linecap="round"/>')
    if kind == "rect":
        (x0, y0), (x1, y1) = nums(node, "start"), nums(node, "end")
        box.add(x0, y0), box.add(x1, y1)
        return (f'<rect x="{f(min(x0, x1))}" y="{f(min(y0, y1))}" '
                f'width="{f(abs(x1 - x0))}" height="{f(abs(y1 - y0))}" '
                f'{attrs}/>')
    if kind == "circle":
        (cx, cy), (ex, ey) = nums(node, "center"), nums(node, "end")
        r = math.hypot(ex - cx, ey - cy)
        box.add(cx - r, cy - r), box.add(cx + r, cy + r)
        return f'<circle cx="{f(cx)}" cy="{f(cy)}" r="{f(r)}" {attrs}/>'
    if kind == "arc":
        s, m, e = nums(node, "start"), nums(node, "mid"), nums(node, "end")
        for p in (s, m, e):
            box.add(*p)
        copper = (f'stroke="{COPPER}" stroke-width="'
                  f'{f(stroke_width(node, 0.1))}" fill="none" '
                  if prefix == "gr_" else "")
        return (f'<path d="{arc_path(s, m, e)}" {copper}'
                f'stroke-linecap="round"/>')
    if kind == "poly":
        pts = [(float(p[1]), float(p[2])) for p in child(node, "pts")
               if isinstance(p, list) and p[0] == "xy"]
        for p in pts:
            box.add(*p)
        return (f'<polygon points="{" ".join(f"{f(x)},{f(y)}" for x, y in pts)}"'
                f' {attrs}/>')
    return ""


def pad_svg(pad, box, labels, turn):
    """`turn` is the footprint's placement angle, which a PCB copy folds
    into every pad angle while keeping pad positions in footprint frame."""
    num, kind, shape = pad[1], pad[2], pad[3]
    layers = child(pad, "layers")[1:]
    on_top = any(lay in ("F.Cu", "*.Cu") for lay in layers)
    if kind != "np_thru_hole" and not on_top:
        return ""
    at = nums(pad, "at")
    x, y, a = at[0], at[1], (at[2] if len(at) > 2 else 0.0) - turn
    w, h = nums(pad, "size")
    for cx, cy in ((-w / 2, -h / 2), (w / 2, -h / 2),
                   (-w / 2, h / 2), (w / 2, h / 2)):
        dx, dy = rot(cx, cy, a)
        box.add(x + dx, y + dy)
    body = []
    if kind == "np_thru_hole":
        body.append(f'<circle r="{f(w / 2)}" fill="#fff" stroke="{COPPER}" '
                    f'stroke-width="0.1"/>')
    else:
        if shape == "custom":
            opts = child(pad, "options")
            anchor = child(opts, "anchor")[1] if opts else "rect"
            shape = "circle" if anchor == "circle" else "rect"
            for prim in child(pad, "primitives")[1:]:
                body.append(graphic(prim, Box(), prefix="gr_"))
        if shape == "circle":
            body.insert(0, f'<circle r="{f(w / 2)}" fill="{COPPER}"/>')
        else:
            rr = 0.0
            if shape == "oval":
                rr = min(w, h) / 2
            elif shape == "roundrect":
                rr = (nums(pad, "roundrect_rratio") or [0.25])[0] * min(w, h)
            body.insert(0, f'<rect x="{f(-w / 2)}" y="{f(-h / 2)}" '
                           f'width="{f(w)}" height="{f(h)}" rx="{f(rr)}" '
                           f'fill="{COPPER}"/>')
        drill = child(pad, "drill")
        if drill:
            vals = [float(v) for v in drill[1:] if isnum(v)]
            off = nums(drill, "offset", [0.0, 0.0])
            if "oval" in drill and len(vals) >= 2:
                dw, dh = vals[0], vals[1]
                body.append(f'<rect x="{f(off[0] - dw / 2)}" '
                            f'y="{f(off[1] - dh / 2)}" width="{f(dw)}" '
                            f'height="{f(dh)}" rx="{f(min(dw, dh) / 2)}" '
                            f'fill="#fff"/>')
            elif vals:
                body.append(f'<circle cx="{f(off[0])}" cy="{f(off[1])}" '
                            f'r="{f(vals[0] / 2)}" fill="#fff"/>')
    if num and kind != "np_thru_hole":
        size = min(w, h)
        if size > labels.get(num, (0,))[0]:
            labels[num] = (size, x, y)
    return (f'<g transform="translate({f(x)},{f(y)}) rotate({f(-a)})">'
            + "".join(body) + "</g>")


def footprint_svg(mod):
    box, layers = Box(), collections.defaultdict(list)
    for node in mod:
        if (isinstance(node, list) and node and node[0].startswith("fp_")
                and node[0] != "fp_text"):
            layer = child(node, "layer")
            if layer and layer[1] in LAYER_STYLE:
                layers[layer[1]].append(graphic(node, box))
    labels, pads, turn = {}, [], footprint_angle(mod)
    for node in children(mod, "pad"):
        pads.append(pad_svg(node, box, labels, turn))
    text = []
    for num, (size, x, y) in labels.items():
        if size < 0.45:
            continue
        fs = max(0.35, min(size * 0.55, 1.2))
        text.append(f'<text x="{f(x)}" y="{f(y)}" font-size="{f(fs)}" '
                    f'fill="#fff">{html.escape(num)}</text>')
    m = 0.6
    x0, y0 = box.x0 - m, box.y0 - m
    w, h = box.x1 - box.x0 + 2 * m, box.y1 - box.y0 + 2 * m
    parts = [f'<g {LAYER_STYLE["F.CrtYd"]} fill="none">',
             *layers["F.CrtYd"], "</g>",
             *pads,
             f'<g {LAYER_STYLE["F.Fab"]} fill="none">', *layers["F.Fab"],
             "</g>",
             f'<g {LAYER_STYLE["F.SilkS"]} fill="none">', *layers["F.SilkS"],
             "</g>",
             '<g font-family="Helvetica, Arial, sans-serif" '
             'text-anchor="middle" dominant-baseline="central" '
             'font-weight="bold">', *text, "</g>"]
    body = "\n".join(parts)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{f(w)}mm" '
            f'height="{f(h)}mm" viewBox="{f(x0)} {f(y0)} {f(w)} {f(h)}">\n'
            f"{body}\n</svg>"), (w, h)


# ---------------------------------------------------------------------------
# HTML
# ---------------------------------------------------------------------------
# Footprint cards sit on a grid of COLUMNS equal columns across the 190 mm
# printable width; a drawing too wide for one column spans several.
COLUMNS, GAP, PAGE_W, CARD_PAD = 4, 3.0, 190.0, 4.6
COL_W = (PAGE_W - (COLUMNS - 1) * GAP) / COLUMNS


def grid_span(width):
    """Columns a card needs so that a `width` mm drawing stays at 1:1."""
    for n in range(1, COLUMNS + 1):
        if n * COL_W + (n - 1) * GAP >= width + CARD_PAD:
            return n
    return COLUMNS


CSS = """
@page { size: A4; margin: 10mm; }
* { box-sizing: border-box; }
body { font-family: Helvetica, Arial, sans-serif; font-size: 7.5pt; margin: 0;
       color: #000; -webkit-print-color-adjust: exact; print-color-adjust: exact; }
h1 { font-size: 13pt; margin: 0 0 1mm; }
h2 { font-size: 11pt; margin: 0 0 1mm; break-before: page; }
p.lead { margin: 0 0 1mm; }
table { border-collapse: collapse; width: 100%; table-layout: fixed;
        font-size: 8pt; }
th, td { border-top: 0.2mm solid #888; padding: 0.6mm 1mm;
         vertical-align: top; text-align: left; }
thead th { background: #eee; border-bottom: 0.3mm solid #000; }
tbody { break-inside: avoid; }
tr.note td { border-top: none; padding-top: 0; }
td.box, th.box { text-align: center; }
td.qty { text-align: right; }
td.fp { font-family: Menlo, Consolas, monospace; font-size: 6.5pt;
        overflow-wrap: anywhere; }
td small { color: #555; }
.warn { color: #b00000; font-weight: bold; }
.new { color: #555; font-style: italic; }
.ruler { margin: 1mm 0 2mm; }
.cards { display: grid; grid-template-columns: repeat(COLUMNS, 1fr);
         gap: GAPmm; font-size: 7pt; line-height: 1.15; }
.card { border: 0.3mm solid #000; padding: 1.5mm; break-inside: avoid;
        display: flex; flex-direction: column; min-width: 0; }
.card .head { display: flex; justify-content: space-between; gap: 1mm; }
.card .title { min-width: 0; }
.card .item { font-weight: bold; font-size: 8pt; text-align: right;
              flex: none; max-width: 45%; }
.card .name { font-family: Menlo, Consolas, monospace; font-size: 7pt;
              font-weight: bold; word-break: break-all; }
.card .lib { font-family: Menlo, Consolas, monospace; font-size: 6pt;
             color: #555; }
.card .draw { margin: 1mm 0; display: flex; justify-content: center; }
.card .draw svg { display: block; flex: none; }
.card ul { margin: 0; padding-left: 3mm; }
.card .check { margin-top: auto; padding-top: 0.5mm; }
.card.flag { border-color: #b00000; border-width: 0.6mm; }
"""


def ruler(length_mm):
    ticks = "".join(
        f'<line x1="{i}" y1="0" x2="{i}" y2="{3 if i % 10 == 0 else 1.5}" '
        f'stroke="#000" stroke-width="0.15"/>'
        + (f'<text x="{i}" y="5.2" font-size="2" text-anchor="middle">'
           f'{i}</text>' if i % 10 == 0 else "")
        for i in range(length_mm + 1))
    return (f'<svg class="ruler" xmlns="http://www.w3.org/2000/svg" '
            f'width="{length_mm + 4}mm" height="7mm" '
            f'viewBox="-2 -0.5 {length_mm + 4} 7">'
            f'<line x1="0" y1="0" x2="{length_mm}" y2="0" stroke="#000" '
            f'stroke-width="0.2"/>{ticks}</svg>')


def esc(s):
    return html.escape(s)


def note_html(note):
    if note.startswith("!"):
        return f'<span class="warn">{esc(note[1:].strip())}</span>'
    return esc(note)


def build():
    placed = placed_footprints()
    lines = bom_lines(placed)

    rows = []
    for ln in lines:
        fp = ln["fp"]
        if fp is None:
            fp_cell = '<span class="new">no footprint</span>'
        else:
            lib, name = fp.split(":")
            fp_cell = f"{esc(name)}<br><small>{esc(lib)}</small>"
            if ln["missing"]:
                fp_cell += '<br><span class="warn">not on the PCB</span>'
            elif ln["pcb"] != fp:
                fp_cell += ('<br><span class="warn">PCB has '
                            f'{esc(ln["pcb"].split(":")[1])}</span>')
        value = esc(ln["value"]) + (f"<br><b>{esc(ln['rating'])}</b>"
                                    if ln["rating"] else "")
        part = esc(ln["mpn"]) + (f"<br><small>{esc(ln['mfr'])}</small>"
                                 if ln["mfr"] else "")
        note = (f'<tr class="note"><td></td><td></td><td colspan="6">'
                f'{note_html(ln["note"])}</td></tr>' if ln["note"] else "")
        rows.append(
            f'<tbody><tr><td class="box">☐</td><td>{esc(ln["item"])}</td>'
            f'<td>{esc(ln["refs"])}</td>'
            f'<td class="qty">{ln["qty"]}</td><td>{value}</td>'
            f'<td>{part}</td><td>{esc(ln["pkg"])}</td>'
            f'<td class="fp">{fp_cell}</td></tr>{note}</tbody>')

    by_fp = collections.OrderedDict()
    for ln in lines:
        if ln["fp"]:
            by_fp.setdefault(ln["fp"], []).append(ln)
    drawn = []
    for fp, uses in by_fp.items():
        on_pcb = [u for u in uses if u["pcb"] == fp]
        wrong = [u for u in uses if u["pcb"] != fp]
        if on_pcb:
            mod = placed[expand(on_pcb[0]["refs"])[0]]
            differs = library_difference(mod, fp)
        else:
            mod, differs = library_copy(fp), None
        svg, (w, _h) = footprint_svg(mod) if mod else ("", (0, 0))
        drawn.append((fp, uses, svg, w, differs, bool(on_pcb), wrong))

    cards = []
    for fp, uses, svg, w, differs, from_pcb, wrong in drawn:
        lib, name = fp.split(":")
        if not svg:
            libnote = '<div class="warn">footprint not available</div>'
        elif not from_pcb:
            libnote = (f'<div class="lib">drawn from the {esc(lib)} '
                       f"library</div>")
        elif differs is None:
            libnote = '<div class="new">library copy not available</div>'
        elif differs:
            libnote = (f'<div class="warn">Pads {esc(", ".join(differs))} '
                       f"differ from the {esc(lib)} library footprint.</div>")
        else:
            libnote = (f'<div class="lib">pads identical to the {esc(lib)} '
                       f"library</div>")
        for u in wrong:
            where = ("not on the PCB" if u["missing"] else
                     f"the PCB has {u['pcb'].split(':')[1]}")
            libnote += (f'<div class="warn">{esc(u["refs"])}: {esc(where)}'
                        " - change it in KiCad.</div>")
        flag = (bool(differs) or bool(wrong)
                or any(u["note"].startswith("!") for u in uses))
        items = "".join(
            f'<li><b>{esc(u["value"])}</b>'
            + (f' <b>{esc(u["rating"])}</b>' if u["rating"] else "")
            + f' ×{u["qty"]} '
            f'<small>{esc(u["refs"]) if u["qty"] <= 8 else ""}</small>'
            + (f'<br><small>{esc(u["mpn"])}</small>' if u["mpn"] else "")
            + "</li>" for u in uses)
        warn = "".join(
            f'<div class="warn">{esc(n[1:].strip())}</div>'
            for n in dict.fromkeys(u["note"] for u in uses)
            if n.startswith("!"))
        numbers = ", ".join(dict.fromkeys(f"#{u['item']}" for u in uses))
        cards.append(
            f'<div class="card{" flag" if flag else ""}" '
            f'style="grid-column: span {grid_span(w)}">'
            f'<div class="head"><div class="title">'
            f'<div class="name">{esc(name)}</div>'
            f'<div class="lib">{esc(lib)}</div></div>'
            f'<div class="item">{esc(numbers)}</div></div>'
            f'<div class="draw">{svg}</div><ul>{items}</ul>{warn}{libnote}'
            f'<div class="check">☐ part fits &nbsp; ☐ pin 1 / polarity OK'
            f"</div></div>")

    n_parts = sum(ln["qty"] for ln in lines if ln["fp"])
    missing = sum(ln["qty"] for ln in lines if ln["missing"])
    wrong = sum(ln["qty"] for ln in lines
                if ln["fp"] and not ln["missing"] and ln["pcb"] != ln["fp"])
    gap = "".join(
        f' <span class="warn">{n} BOM references {what}.</span>'
        for n, what in ((missing, "are not on the PCB"),
                        (wrong, "still have another footprint on the PCB"))
        if n)
    doc = f"""<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8">
<title>omnimotion - parts list and footprint check</title>
<style>{CSS.replace("COLUMNS", str(COLUMNS)).replace("GAP", f(GAP))}</style></head><body>
<h1>omnimotion Rev A.1 - parts list and footprint check</h1>
<p class="lead">{len(rows)} part lines, {n_parts} footprints (parts, test
pads and mounting holes), {len(by_fp)} distinct footprints. Generated by
gen_footprint_sheet.py from omnimotion_BOM.csv, which names the footprint
each part needs, and kicad/omnimotion.kicad_pcb.
Drawings are the PCB's own copy where it is right, the library footprint
where it is not.{gap}
<b>Print at 100 % ("Actual size"), then check this ruler with a real
one:</b></p>
{ruler(50)}
<table><colgroup><col style="width:5mm"><col style="width:6mm">
<col style="width:36mm"><col style="width:7mm"><col style="width:24mm">
<col style="width:36mm"><col style="width:24mm"><col style="width:52mm">
</colgroup>
<thead><tr><th class="box">✓</th><th>#</th><th>Refs</th>
<th>Qty</th><th>Value</th><th>Part</th><th>Package (BOM)</th>
<th>Footprint</th></tr></thead>
{chr(10).join(rows)}
</table>
<h2>Footprints at 1:1 - lay each part on its land pattern</h2>
<p class="lead">Grey = top copper (pad numbers in white), blue = component
body (F.Fab), black = silkscreen, dashed = courtyard. Holes are white.
Cards follow the parts list; the number in the corner is the list's #, and a
footprint shared by several lines appears once, at its first line. Red frames
need attention before the PCB is ordered.</p>
{ruler(50)}
<div class="cards">
{chr(10).join(cards)}
</div>
</body></html>
"""
    return doc


def main():
    doc = build()
    with open(OUT_HTML, "w", encoding="utf-8") as f:
        f.write(doc)
    print("wrote", os.path.relpath(OUT_HTML, HW))
    chrome = CHROME if os.path.exists(CHROME) else shutil.which("chromium")
    if not chrome:
        print("no Chrome found: print the HTML from a browser instead")
        return
    subprocess.run([chrome, "--headless=new", "--disable-gpu",
                    "--no-pdf-header-footer", f"--print-to-pdf={OUT_PDF}",
                    "file://" + OUT_HTML], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print("wrote", os.path.relpath(OUT_PDF, HW))


if __name__ == "__main__":
    main()
