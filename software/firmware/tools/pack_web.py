#!/usr/bin/env python3
"""Pack a directory of web assets into an NVS image for the `web` partition.

Everything is gzipped unconditionally. Every browser accepts gzip, and
compressing everything means the firmware can answer with a fixed
`Content-Encoding: gzip` instead of recording which files happened to be worth
compressing. Assets are stored as blobs rather than strings because NVS caps a
string at 4000 bytes on a single page, while blobs span pages.

The assets get their own partition because flashing an image replaces the whole
partition; sharing `nvs` would erase every setting on each UI update. The
offset and size are read from partitions.csv so the two cannot drift apart.

The image is written by esp-nvs-partition-tool, a standalone Rust port of
Espressif's nvs_partition_gen.py, so no ESP-IDF install is needed:

    cargo install esp-nvs-partition-tool

The script prints the espflash command that writes the result.
"""
import argparse
import csv
import gzip
import pathlib
import shutil
import subprocess
import sys

TOOL = "esp-nvs-partition-tool"

# NVS keys are 15 bytes plus a null terminator.
MAX_KEY_LEN = 15

# Past roughly this much of the partition, NVS spends more time rewriting pages
# to reclaim space than writing new data.
COMFORTABLE_FILL = 0.70

PARTITIONS = pathlib.Path(__file__).resolve().parent.parent / "partitions.csv"


def find_partition(table, name):
    """Offset and size of `name` in an ESP-IDF partition CSV."""
    lines = [line for line in table.read_text().splitlines()
             if line.strip() and not line.lstrip().startswith("#")]
    for row in csv.reader(lines, skipinitialspace=True):
        if row[0].strip() == name:
            if row[1].strip() != "data" or row[2].strip() != "nvs":
                sys.exit(f"partition {name!r} in {table} is not an nvs data partition")
            return int(row[3], 0), int(row[4], 0)
    sys.exit(f"no partition named {name!r} in {table}")


def collect(source):
    """Top-level files only: NVS keys are flat and short, so a directory tree
    has nowhere to go."""
    files, skipped = [], []
    for path in sorted(source.iterdir()):
        if path.is_dir():
            skipped.append(path.name)
        elif path.is_file() and not path.name.startswith("."):
            files.append(path)
    return files, skipped


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", type=pathlib.Path,
                        help="directory of web assets to pack")
    parser.add_argument("--out", type=pathlib.Path, default=pathlib.Path("build/web.bin"),
                        help="image to write (default: build/web.bin)")
    parser.add_argument("--partition", default="web",
                        help="partition to pack for, from partitions.csv (default: web)")
    parser.add_argument("--namespace", default="web",
                        help="NVS namespace to store under (default: web)")
    args = parser.parse_args()

    offset, size = find_partition(PARTITIONS, args.partition)
    if not args.source.is_dir():
        sys.exit(f"{args.source} is not a directory")
    if len(args.namespace) > MAX_KEY_LEN:
        sys.exit(f"namespace {args.namespace!r} exceeds {MAX_KEY_LEN} bytes")

    files, skipped = collect(args.source)
    if not files:
        sys.exit(f"no files in {args.source}")
    for name in skipped:
        print(f"skipping subdirectory {name}/", file=sys.stderr)

    too_long = [f.name for f in files if len(f.name.encode()) > MAX_KEY_LEN]
    if too_long:
        sys.exit("NVS keys are limited to "
                 f"{MAX_KEY_LEN} bytes, rename: {', '.join(too_long)}")
    # The tool's input is CSV, so a comma would split the row.
    with_commas = [f.name for f in files if "," in f.name]
    if with_commas:
        sys.exit(f"file names cannot contain commas, rename: {', '.join(with_commas)}")

    staging = args.out.parent / "nvs"
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True, exist_ok=True)

    rows = [f"{args.namespace},namespace,,"]
    raw_total = packed_total = 0
    for path in files:
        raw = path.read_bytes()
        # mtime=0 so rebuilding identical inputs produces an identical image.
        packed = gzip.compress(raw, compresslevel=9, mtime=0)
        (staging / f"{path.name}.gz").write_bytes(packed)
        rows.append(f"{path.name},file,binary,nvs/{path.name}.gz")
        raw_total += len(raw)
        packed_total += len(packed)
        print(f"  {path.name:24s} {len(raw):7d} -> {len(packed):7d} B")

    print(f"  {'total':24s} {raw_total:7d} -> {packed_total:7d} B "
          f"({packed_total / size:.1%} of {args.partition})")
    if packed_total > size * COMFORTABLE_FILL:
        print(f"warning: assets fill more than {COMFORTABLE_FILL:.0%} of the "
              f"{args.partition} partition", file=sys.stderr)

    csv_path = args.out.parent / "nvs_data.csv"
    csv_path.write_text("key,type,encoding,value\n" + "\n".join(rows) + "\n")

    if shutil.which(TOOL) is None:
        sys.exit(f"{TOOL} not found on PATH; install it with "
                 f"'cargo install {TOOL}', then rerun")

    subprocess.run(
        [TOOL, "generate", str(csv_path), str(args.out), "--size", str(size)],
        check=True,
    )
    print(f"wrote {args.out} ({args.out.stat().st_size} B); flash it with:")
    print(f"  espflash write-bin {offset:#x} {args.out}")


if __name__ == "__main__":
    main()
