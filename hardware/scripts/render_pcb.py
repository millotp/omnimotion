#!/usr/bin/env python3
"""Render the KiCad PCB in 3D: a top view and an angled view.

kicad-cli runs inside the official KiCad 10 Docker image, so neither KiCad
nor its 3D library has to be installed. The image ships without 3D models,
so the ones the PCB names are downloaded once from the official KiCad 3D
library into build/3dmodels/ and mounted into the container. The one part with
no model in the library (NO_MODEL) renders as bare pads. Some of the models
are the closest match in the library rather than the exact part, so connector
and package shapes are approximate.

`images` writes generated/omnimotion_pcb_{top,angle}.jpg. `models`
exports the board as a coloured 3D model and writes it where you say: the STEP
and GLB paths are both required, and nothing is written into the repository
by default because the files are large.

Needs Docker. The image is amd64 only, so on Apple Silicon it runs under
emulation and each view takes about a minute.

Run:  python3 scripts/render_pcb.py images
      python3 scripts/render_pcb.py models board.step board.glb
"""
import argparse
import os
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request

HW = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
KICAD = os.path.join(HW, "kicad")
PCB = os.path.join(KICAD, "omnimotion.kicad_pcb")
MODELS = os.path.join(HW, "build", "3dmodels")
OUT = os.path.join(HW, "generated")
IMAGE = "kicad/kicad:10.0"
CONTAINER = f"omnimotion-render-{os.getpid()}"  # per run, so two runs never kill each other
LIB_URL = "https://gitlab.com/kicad/libraries/kicad-packages3D/-/raw/master/{path}"

NO_MODEL = {"Fuse.3dshapes/Fuseholder_Littelfuse_Nano2_154x.step"}

RENDER = ["--quality", "high", "--floor", "--background", "opaque"]
VIEWS = {
    "top": ["--side", "top", "--width", "2000", "--height", "2000"],
    "angle": ["--perspective", "--rotate", "-40,0,-25", "--zoom", "0.8",
              "--width", "2400", "--height", "1600"],
}


# 3D exports of the same board: STEP for CAD (solids, colours kept) and GLB for
# viewers (PBR materials). Board body, component models and the soldermask; no
# copper pads, tracks, zones or silkscreen. The pads alone are about 60% of the
# triangles (155k of 250k) and are hidden under the parts or flush with the
# mask, so leaving them out keeps the model light (about 95k triangles, 3.7 MB
# GLB). Components marked do-not-populate are left out.
MODEL_FORMATS = ["step", "glb"]
MODEL_OPTIONS = ["--force", "--no-dnp", "--include-soldermask"]


def models():
    with open(PCB, encoding="utf-8") as f:
        text = f.read()
    return sorted(set(re.findall(r'\(model "\$\{KICAD10_3DMODEL_DIR\}/([^"]+)"', text)))


def fetch(path):
    dest = os.path.join(MODELS, path)
    if os.path.exists(dest) or path in NO_MODEL:
        return
    url = LIB_URL.format(path=urllib.parse.quote(path))
    try:
        with urllib.request.urlopen(url, timeout=60) as r:
            data = r.read()
    except (urllib.error.URLError, OSError) as e:
        print(f"no model for {path}: {e}", file=sys.stderr)
        return
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    with open(dest, "wb") as f:
        f.write(data)
    print(f"downloaded {path}")


def run(*args, check=True):
    quiet = {} if check else {"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}
    return subprocess.run(["docker", *args], check=check, **quiet)


def parse_args():
    parser = argparse.ArgumentParser(description="Render or export the KiCad PCB in 3D.")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("images", help="write the top and angle JPEGs to generated/")
    export = commands.add_parser("models", help="export the board as STEP and GLB")
    export.add_argument("step", help="where to write the STEP file")
    export.add_argument("glb", help="where to write the GLB file")
    return parser.parse_args()


def main():
    args = parse_args()
    for path in models():
        fetch(path)

    if run("image", "inspect", IMAGE, check=False).returncode != 0:
        run("pull", "--platform", "linux/amd64", IMAGE)

    # Bind-mounted output directories are not reliably writable by the
    # container's kicad user, so renders are copied out with `docker cp`.
    commands = ["cp -r /pcb /tmp/pcb", "mkdir -p /tmp/out"]
    outputs = {}  # file name in the container -> destination on the host
    if args.command == "images":
        for name, view in VIEWS.items():
            options = " ".join(f"'{a}'" for a in [*RENDER, *view])
            filename = f"omnimotion_pcb_{name}.jpg"
            commands.append(f"kicad-cli pcb render {options} "
                            f"-o /tmp/out/{filename} "
                            f"/tmp/pcb/omnimotion.kicad_pcb > /dev/null")
            outputs[filename] = os.path.join(OUT, filename)
    else:
        for fmt in MODEL_FORMATS:
            filename = f"omnimotion_pcb.{fmt}"
            commands.append(f"kicad-cli pcb export {fmt} {' '.join(MODEL_OPTIONS)} "
                            f"-o /tmp/out/{filename} "
                            f"/tmp/pcb/omnimotion.kicad_pcb > /dev/null")
            outputs[filename] = os.path.abspath(getattr(args, fmt))
    run("rm", "-f", CONTAINER, check=False)
    try:
        run("run", "--name", CONTAINER, "--platform", "linux/amd64",
            "-e", "KICAD10_3DMODEL_DIR=/models",
            "-v", f"{KICAD}:/pcb:ro", "-v", f"{MODELS}:/models:ro",
            IMAGE, "bash", "-ec", " && ".join(commands))
        for filename, destination in outputs.items():
            os.makedirs(os.path.dirname(destination), exist_ok=True)
            run("cp", f"{CONTAINER}:/tmp/out/{filename}", destination)
            print(destination)
    finally:
        run("rm", "-f", CONTAINER, check=False)


if __name__ == "__main__":
    main()
