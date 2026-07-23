"""Generate layout variants, close them geometrically, run the real eval."""
import json
import pathlib
import subprocess
import sys

import variants2 as V
from build import hill
from solve2 import solve2

HOME = pathlib.Path.home()
SCEN = HOME / "rct2-assets/Scenarios/Build your own Six Flags Park.SC6"
CLI = "/Users/weaton/git/OpenRCT2/build/openrct2-cli"


def sl5(first):
    a = "left" if first == "l" else "right"
    b = "right" if first == "l" else "left"
    return ["flat_to_up_25", f"{a}_turn_5_up_25", "up_25_to_flat",
            "flat_to_down_25", f"{b}_turn_5_down_25", "down_25_to_flat"]


def spike(n60=1):
    return (["flat_to_up_25", "up_25_to_up_60"] + ["up_60"] * n60
            + ["up_60_to_up_25", "up_25_to_flat", "flat_to_down_25", "down_25_to_down_60"]
            + ["down_60"] * n60 + ["down_60_to_down_25", "down_25_to_flat"])


def run(name):
    out = subprocess.run([CLI, "eval", str(SCEN), "--ticks", "25000", "--rct2-data-path",
                          str(HOME / "rct2-assets"), "--program", f"/tmp/{name}.json",
                          "--out", f"/tmp/{name}-report.json"], capture_output=True, text=True)
    try:
        d = json.load(open(f"/tmp/{name}-report.json"))["rides"][0]
    except (IndexError, FileNotFoundError):
        return None
    if d["excitement"] is None:
        return None
    return d


def trial(name, n60, n25, nrev, PB, PC, PD):
    if solve2(name, n60, n25, nrev, PB=PB, PC=PC, PD=PD, quiet=True) is None:
        print(f"{name:6s} no geometry")
        return
    d = run(name)
    if d is None:
        print(f"{name:6s} rejected")
        return
    print(f"{name:6s} E={d['excitement']:.2f} I={d['intensity']:.2f} "
          f"lat={d['max_lateral_g']:.2f} pos={d['max_positive_g']:.2f} neg={d['max_negative_g']:.2f} "
          f"air={d['total_air_time']} drops={d['num_drops']} avg={d['average_speed']}")
