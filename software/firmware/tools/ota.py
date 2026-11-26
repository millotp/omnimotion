#!/usr/bin/env python3
"""Send a firmware build, or a web partition image, to a board over WiFi.

By default the ELF is converted to an app image with `espflash save-image`,
which is what the bootloader expects in an app slot, then POSTed to `/ota`. The
board writes it to its inactive slot, reboots into it, and keeps it only if the
new image gets back onto the network; otherwise it returns to the image it had.

With --web, the file is the `build/web.bin` written by `pack_web.py` and is
POSTed unchanged to `/web`. The board writes it over the web partition and
reboots. The application is untouched, and there is no trial.

The token is read from OMNIMOTION_OTA_TOKEN, the same variable the firmware
was built with. Build the new image with the WiFi variables set too, or it
cannot rejoin and will be rolled back.
"""
import argparse
import os
import pathlib
import subprocess
import sys
import urllib.error
import urllib.request

# Must match `network::OTA_PORT`.
PORT = 8080

# Every 4 kB sector erase stalls the board for tens of milliseconds.
TIMEOUT_S = 120


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("file", type=pathlib.Path,
                        help="firmware ELF, e.g. target/xtensa-esp32s3-none-elf/release/examples/ota; "
                             "with --web, the web.bin from pack_web.py")
    parser.add_argument("host", help="board IP address, printed by the firmware at boot")
    parser.add_argument("--web", action="store_true",
                        help="send a web partition image to /web instead of firmware")
    parser.add_argument("--out", type=pathlib.Path, default=pathlib.Path("build/ota.bin"),
                        help="app image to write (default: build/ota.bin)")
    args = parser.parse_args()

    token = os.environ.get("OMNIMOTION_OTA_TOKEN")
    if not token:
        sys.exit("set OMNIMOTION_OTA_TOKEN to the token the board was built with")
    if not args.file.is_file():
        sys.exit(f"{args.file} not found; build it first")

    if args.web:
        image = args.file.read_bytes()
        path = "/web"
    else:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["espflash", "save-image", "--chip", "esp32s3", str(args.file), str(args.out)],
            check=True,
        )
        image = args.out.read_bytes()
        path = "/ota"

    url = f"http://{args.host}:{PORT}{path}"
    print(f"sending {len(image)} B to {url}")
    request = urllib.request.Request(url, data=image, method="POST", headers={
        "X-API-Key": token,
        "Content-Type": "application/octet-stream",
    })
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT_S) as response:
            print(response.read().decode().strip())
    except urllib.error.HTTPError as error:
        sys.exit(f"{error.code}: {error.read().decode().strip()}")
    except urllib.error.URLError as error:
        sys.exit(f"could not reach {url}: {error.reason}")
    except ConnectionError as error:
        # The board answers and hangs up as soon as it refuses an upload.
        sys.exit(f"board closed the connection mid-upload: {error}")


if __name__ == "__main__":
    main()
