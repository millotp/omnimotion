#!/usr/bin/env python3
"""Check a KiCad schematic against the generated design.

gen_schematic.py is the source of truth. This reads the KiCad project
read-only, extracts its netlist and compares it with the generator's, part by
part and net by net, and checks values (a KiCad value may also be the BOM
part number) and footprints against the BOM. Net names are ignored: only what
connects to what. It then checks that the PCB next to the schematic carries
the same parts, footprints and pad nets.

Run:  python3 scripts/check_kicad.py [path/to/root.kicad_sch]
Exits non-zero when anything differs.
"""
import collections
import os
import re
import sys

import gen_bom
import gen_schematic as g
import kicadlib

HW = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT = os.path.join(HW, "kicad", "omnimotion.kicad_sch")

# Terminal names the generator draws on the USB-C receptacle -> its contacts.
USB_C = {"VBUS": ["A4", "A9", "B4", "B9"], "GND": ["A1", "A12", "B1", "B12"],
         "SHLD": ["SH"], "D+": ["A6", "B6"], "D-": ["A7", "B7"],
         "CC1": ["A5"], "CC2": ["B5"]}
# Two-terminal parts whose pins are interchangeable.
SYMMETRIC = {"res_h", "res_v", "cap_h", "ind_h", "fuse", "jumper", "button"}


def by_name(kpins, name):
    nums = [n for n, nm in kpins.items() if nm == name]
    return nums or None


def kicad_pins(kind, args, key, kpins):
    """KiCad pin numbers for generator terminal `key`, or None if unknown."""
    if kind == "ic":
        for side in ("left", "right", "top", "bottom"):
            for e in args.get(side) or ():
                num, name, k = (tuple(e) + (e[1],))[:3]
                if k == key:
                    if num == "EP":  # KiCad numbers the pad after the pins
                        return [str(max(int(n) for n in kpins if n.isdigit()))]
                    return num.split(",")
        return None
    if kind == "conn":
        if args["ref"] == "J4":
            return USB_C.get(key)
        return [str(list(args["pins"]).index(key) + 1)]
    if kind in ("diode_v", "diode_h"):
        anode = (key == "1") != bool(args.get("flip"))
        return by_name(kpins, "A" if anode else "K") or \
            ["2" if anode else "1"]
    if kind == "mosfet":
        return by_name(kpins, key)
    return [key]


def symmetric(kind, args):
    return kind in SYMMETRIC or (kind == "cap_v" and not args.get("pol"))


def norm_value(v):
    v = (v or "").replace("\u00b5", "u").replace("\u03bc", "u")
    v = v.replace("\u2126", "").replace("\u03a9", "").replace("Ω", "")
    v = re.sub(r"\s+", "", v).lower()
    return v.replace("r", "") if re.fullmatch(r"[\d.]+r", v) else v


def bom_key(ref, value):
    return gen_bom.VALUE_MAP.get(gen_bom.split(ref)[0], {}).get(value)


def bom_mpn(ref, value):
    """The BOM part number for a design value; KiCad may carry it as Value."""
    key = bom_key(ref, value)
    return gen_bom.LIB[key]["mpn"] if key else ""


def compare(root):
    doc = g.build()
    gen_nets = doc.netlist()
    placed = {ref: (kind, args) for s in doc.sheets
              for ref, kind, _, args in s.placed if ref}
    gen_vals = dict(p for s in doc.sheets for p in s.parts)
    parts, kc_nets, notes = kicadlib.schematic(root)
    kc_of = {pin: n for n, pins in kc_nets.items() for pin in pins}
    out = collections.defaultdict(list)

    # ------------------------------------------------------------- parts
    for ref in sorted(set(placed) - set(parts)):
        out["missing from KiCad"].append(ref)
    extra = sorted(r for r in set(parts) - set(placed)
                   if not r.startswith("MH"))
    for ref in extra:
        out["in KiCad but not in the design"].append(
            f"{ref} {parts[ref]['value']}")
    for ref in sorted(set(placed) & set(parts)):
        if ref.startswith("TP"):
            continue
        a, b = norm_value(gen_vals.get(ref)), norm_value(parts[ref]["value"])
        if a != b and not (a.startswith(b) or b.startswith(a)) \
                and b != norm_value(bom_mpn(ref, gen_vals.get(ref))):
            out["values differ (design / KiCad)"].append(
                f"{ref}: {gen_vals.get(ref)!r} / {parts[ref]['value']!r}")

    # ---------------------------------------------------------------- nets
    fixed, loose = {}, {}          # gen net -> [(ref, kicad pins)]
    unmapped = []
    for net, members in gen_nets.items():
        for ref, key in members:
            if ref not in parts:
                continue
            kind, args = placed[ref]
            pins = kicad_pins(kind, args, key, parts[ref]["pins"])
            if not pins or any(p not in parts[ref]["pins"] for p in pins):
                unmapped.append(f"{ref}.{key} -> {pins}")
                continue
            dest = loose if symmetric(kind, args) else fixed
            dest.setdefault(net, []).append((ref, key, pins))
    out["generator terminals with no KiCad pin"] = unmapped

    # Pair each generator net with the KiCad net its fixed pins land in.
    match = {}
    for net in gen_nets:
        votes = collections.Counter(
            kc_of.get((ref, p)) for ref, _, pins in fixed.get(net, [])
            for p in pins)
        votes.pop(None, None)
        if votes:
            match[net] = votes.most_common(1)[0][0]
    for net in gen_nets:
        if net in match:
            continue
        votes = collections.Counter(
            kc_of.get((ref, p)) for ref, _, pins in loose.get(net, [])
            for p in pins)
        votes.pop(None, None)
        taken = set(match.values())
        for kn, _ in votes.most_common():
            if kn not in taken:
                match[net] = kn
                break

    for net, members in sorted(fixed.items()):
        for ref, key, pins in members:
            for p in pins:
                got = kc_of.get((ref, p))
                if got != match.get(net):
                    out["fixed pins on the wrong net"].append(
                        f"{ref}.{key} (pin {p}): on KiCad net {got}, but its "
                        f"design net {net} is KiCad net {match.get(net)}")
    gen_of = {}
    for net, members in loose.items():
        for ref, key, _ in members:
            gen_of.setdefault(ref, []).append(match.get(net, f"<{net}>"))
    for ref, want in sorted(gen_of.items()):
        got = [kc_of.get((ref, p)) for p in ("1", "2")]
        if sorted(want, key=str) != sorted(got, key=str):
            names = {v: k for k, v in match.items()}
            out["two-terminal parts on the wrong nets"].append(
                f"{ref}: design {sorted(names.get(w, w) for w in want)}, "
                f"KiCad {got}")

    shared = collections.defaultdict(list)
    for net, kn in match.items():
        shared[kn].append(net)
    for kn, nets in sorted(shared.items()):
        if len(nets) > 1:
            out["design nets shorted together in KiCad"].append(
                f"{kn}: {' + '.join(sorted(nets))}")
    for net in sorted(set(gen_nets) - set(match)):
        out["design nets with no KiCad counterpart"].append(
            f"{net}: " + " ".join(f"{r}.{k}" for r, k in
                                  sorted(gen_nets[net])))

    used = set(match.values())
    claimed = {(ref, p) for ms in (*fixed.values(), *loose.values())
               for ref, _, pins in ms for p in pins}
    # The BOM ties the mounting holes to GND; the generator does not draw them.
    claimed |= {(r, "1") for r in parts if r.startswith("MH")
                and kc_of.get((r, "1")) == match.get("GND")}
    for kn, pins in sorted(kc_nets.items()):
        stray = sorted(p for p in pins if p not in claimed)
        if stray and len(pins) > 1:
            out["KiCad pins the design leaves unconnected"].append(
                f"{kn}: " + " ".join(f"{r}.{n}" for r, n in stray))
        if kn not in used and len(pins) > 1 and not stray:
            out["KiCad nets with no design counterpart"].append(kn)
    out["KiCad connectivity notes"] = notes

    # ------------------------------------------------- PCB vs schematic
    board_path = os.path.splitext(root)[0] + ".kicad_pcb"
    wrong_fp = collections.defaultdict(list)
    for ref in sorted(set(placed) & set(parts), key=gen_bom.split):
        key = bom_key(ref, gen_vals.get(ref))
        have = parts[ref]["footprint"]
        if key and have not in gen_bom.footprints(key):
            wrong_fp[have, " / ".join(gen_bom.footprints(key))].append(ref)
    for (have, want), refs in wrong_fp.items():
        out["footprint differs from the BOM (KiCad / BOM)"].append(
            f"{' '.join(refs)}: {have} / {want}")

    if os.path.exists(board_path):
        board = kicadlib.pcb(board_path)
        for ref in sorted(set(parts) - set(board)):
            out["on the schematic but not the PCB"].append(ref)
        for ref in sorted(set(board) - set(parts)):
            out["on the PCB but not the schematic"].append(ref)
        for ref in sorted(set(parts) & set(board)):
            if parts[ref]["footprint"] != board[ref][0]:
                out["PCB footprint differs from the schematic"].append(
                    f"{ref}: schematic {parts[ref]['footprint']}, "
                    f"PCB {board[ref][0]}")
        pcb_nets = collections.defaultdict(set)
        for ref, (_, pads) in board.items():
            for pad, net in pads.items():
                if net:
                    pcb_nets[net].add((ref, pad))
        pcb_of = {p: n for n, ps in pcb_nets.items() for p in ps}
        for kn, pins in sorted(kc_nets.items()):
            split = {pcb_of.get(p) for p in pins}
            if len(split) > 1:
                out["PCB pad nets out of sync with the schematic"].append(
                    f"{kn} is split on the PCB into {sorted(map(str, split))}")
        for pn, pins in sorted(pcb_nets.items()):
            merged = {kc_of.get(p) for p in pins} - {None}
            if len(merged) > 1:
                out["PCB pad nets out of sync with the schematic"].append(
                    f"PCB net {pn} joins {sorted(merged)}")
    return {k: v for k, v in out.items() if v}, len(gen_nets), len(kc_nets)


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else DEFAULT
    problems, n_gen, n_kc = compare(root)
    print(f"{os.path.relpath(root, HW)}: {n_kc} nets, design: {n_gen} nets")
    for title, items in problems.items():
        print(f"\n{title} ({len(items)}):")
        for it in items:
            print("  " + it)
    if not problems:
        print("schematic matches the design")
    errors = set(problems) - {"KiCad connectivity notes"}
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
