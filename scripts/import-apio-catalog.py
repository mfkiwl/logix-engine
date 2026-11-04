#!/usr/bin/env python3
"""Converts Apio's board and FPGA definitions into the engine's catalog,
catalog/boards.json and catalog/fpgas.json.

The catalog in this repo is now its own source and may have local edits.
This script rewrites both files, so use it only to bring in more boards from
Apio: then keep the new entries and restore any local edits it overwrote.

Usage: scripts/import-apio-catalog.py [APIO_PACKAGES_FOLDER]

APIO_PACKAGES_FOLDER defaults to ~/.apio/packages. It must hold Apio's
definitions package (boards.jsonc, programmers.jsonc), oss-cad-suite
package (FPGAS.jsonc) and openxc7 package (FPGAS.jsonc, the Xilinx FPGAs). Review the diff after running it.

What's imported:
- Every iCE40, ECP5, Gowin and Xilinx FPGA. ECP5 FPGAs also get nextpnr_speed.
  Gowin FPGAs get nextpnr_device, packer_device, and nextpnr_family and
  yosys_family when Apio gives them (not empty), and no nextpnr_package.
  Xilinx FPGAs get nextpnr_device, yosys_arch, and prjxray_family (Apio's
  yosys-family, the Project X-Ray database folder).
- The iCE40, ECP5, Gowin and Xilinx boards whose programmer the engine supports:
  openfpgaloader, iceprog, dfu (named dfu-util in the engine) and icesprog.
  Each gets no dimensions and one configuration with Apio's full command
  arguments, and tested true. Apio's ${BIN_FILE} becomes ${BITSTREAM}, added at the
  end when Apio leaves it out. An openFPGALoader board without a USB filter
  in Apio, such as one programmed with an external cable, gets the default
  USB IDs of the cable its -c option names (CABLE_USB_IDS), so the engine
  can find the device.

Local edits to restore after an import: the boards merged into one board with
dimensions (FPGA size, connection or memory; see docs/requirements.md, Boards
and variants), unique descriptions, boards moved to another programmer
(the ULX3S boards to openFPGALoader, odt-icyblue-feather to iceprog, both
with tested false), serial_regex filters (the iCESugar boards), user_notes and maintainer_notes.
"""

import json
import os
import re
import sys

ARCHITECTURES = {"ice40", "ecp5", "gowin", "xilinx"}

# openFPGALoader's default USB IDs for the cables Apio's boards name with -c
# (openFPGALoader's src/cable.hpp).
CABLE_USB_IDS = {
    "ft2232": ("0403", "6010"),
    "ft232": ("0403", "6014"),
    "ft231X": ("0403", "6015"),
    "usb-blaster": ("09fb", "6001"),
    "cmsisdap": ("0d28", "0204"),
}
# Apio's programmer IDs the engine supports, and the engine's names for them.
PROGRAMMERS = {"openfpgaloader": "openfpgaloader", "iceprog": "iceprog", "dfu": "dfu-util", "icesprog": "icesprog"}


def load_jsonc(path):
    """Reads JSON with // and /* */ comments and trailing commas."""
    s = open(path, encoding="utf-8").read()
    out, i, n, in_string = [], 0, len(s), False
    while i < n:
        c = s[i]
        if in_string:
            out.append(c)
            if c == "\\":
                out.append(s[i + 1])
                i += 2
                continue
            if c == '"':
                in_string = False
            i += 1
        elif c == '"':
            in_string = True
            out.append(c)
            i += 1
        elif s.startswith("//", i):
            while i < n and s[i] != "\n":
                i += 1
        elif s.startswith("/*", i):
            i = s.index("*/", i) + 2
        else:
            out.append(c)
            i += 1
    return json.loads(re.sub(r",(\s*[}\]])", r"\1", "".join(out)))


def build_info(folder):
    try:
        with open(os.path.join(folder, "BUILD-INFO.json"), encoding="utf-8") as f:
            info = json.load(f)
        return f"{info.get('package-name')} {info.get('release-tag')}"
    except OSError:
        return os.path.basename(folder)


def main():
    packages = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.apio/packages")
    definitions = os.path.join(packages, "definitions")
    oss = os.path.join(packages, "oss-cad-suite")
    apio_fpgas = load_jsonc(os.path.join(oss, "FPGAS.jsonc"))
    apio_fpgas.update(load_jsonc(os.path.join(packages, "openxc7", "FPGAS.jsonc")))
    apio_boards = load_jsonc(os.path.join(definitions, "boards.jsonc"))
    apio_programmers = load_jsonc(os.path.join(definitions, "programmers.jsonc"))

    fpgas = {}
    for fid, f in sorted(apio_fpgas.items()):
        if f["arch"] not in ARCHITECTURES:
            continue
        params = f[f"{f['arch']}-params"]
        fpgas[fid] = {
            "architecture": f["arch"],
            "part_number": f["vendor-part"],
            "size": f["size"],
        }
        if f["arch"] == "gowin":
            fpgas[fid]["nextpnr_device"] = params["nextpnr-device"]
            for apio, ours in [
                ("nextpnr-family", "nextpnr_family"),
                ("packer-device", "packer_device"),
                ("yosys-family", "yosys_family"),
            ]:
                if params.get(apio):
                    fpgas[fid][ours] = params[apio]
        elif f["arch"] == "xilinx":
            fpgas[fid]["nextpnr_device"] = params["nextpnr-device"]
            fpgas[fid]["yosys_arch"] = params["yosys-arch"]
            fpgas[fid]["prjxray_family"] = params["yosys-family"]
        else:
            fpgas[fid]["nextpnr_device"] = params["nextpnr-type"]
            fpgas[fid]["nextpnr_package"] = params["nextpnr-package"]
        if "nextpnr-speed" in params:
            fpgas[fid]["nextpnr_speed"] = params["nextpnr-speed"]

    boards, skipped = {}, []
    for bid, b in sorted(apio_boards.items()):
        if b["fpga-id"] not in fpgas:
            continue
        pid = b["programmer"]["id"]
        args = " ".join(
            a for a in [apio_programmers[pid].get("args", ""), b["programmer"].get("extra-args", "")] if a
        )
        if "usb" not in b and pid == "openfpgaloader":
            cable = re.search(r"-c (\S+)", args)
            if cable and cable.group(1) in CABLE_USB_IDS:
                vid, upid = CABLE_USB_IDS[cable.group(1)]
                b = dict(b, usb={"vid": vid, "pid": upid})
        if pid not in PROGRAMMERS or "usb" not in b:
            skipped.append(f"{bid} ({pid})")
            continue
        args = re.sub(r"\s+", " ", args).replace("${BIN_FILE}", "${BITSTREAM}")
        if "${BITSTREAM}" not in args:
            args += " ${BITSTREAM}"
        usb = {"vid": b["usb"]["vid"].lower(), "pid": b["usb"]["pid"].lower()}
        if "product-regex" in b["usb"]:
            usb["product_regex"] = b["usb"]["product-regex"]
        boards[bid] = {
            "description": b["description"],
            "fpga_id": b["fpga-id"],
            "variants": {"configurations": [
                {"programmer": PROGRAMMERS[pid], "args": args.strip(), "usb": usb, "tested": True}
            ]},
        }

    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "catalog")
    source = f"Imported from Apio's {build_info(definitions)} and {build_info(oss)} packages by scripts/import-apio-catalog.py."
    for name, data in [("fpgas", fpgas), ("boards", boards)]:
        with open(os.path.join(root, f"{name}.json"), "w", encoding="utf-8") as f:
            json.dump({"source": source, name: data}, f, indent=2, ensure_ascii=False)
            f.write("\n")
    print(f"{len(fpgas)} FPGAs, {len(boards)} boards. Skipped {len(skipped)} boards with unsupported programmers:")
    for s in skipped:
        print(f"  {s}")


if __name__ == "__main__":
    main()
