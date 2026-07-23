#!/usr/bin/env python3
"""Round builder: assemble a track program from named segments, sim it, write JSON."""
import json
import sys
from pathlib import Path

import sim


def chain(*names):
    return [{"t": n, "chain": True} for n in names]


def lift(n):
    """Chain lift: flat -> n x up_25 -> flat. Gains 16 + 16*n z."""
    return chain("flat_to_up_25", *["up_25"] * n, "up_25_to_flat")


def hill(n=1):
    """Airtime hill: up 25 for n, back down. Net dz 0, length 4+2*(n-1)."""
    return (["flat_to_up_25"] + ["up_25"] * n + ["up_25_to_flat"]
            + ["flat_to_down_25"] + ["down_25"] * n + ["down_25_to_flat"])


def bigdrop():
    return ["flat_to_down_25", "down_25_to_down_60", "down_60",
            "down_60_to_down_25", "down_25_to_flat"]


def banked_turn(side, radius, count):
    b = f"flat_to_{side}_bank"
    e = f"{side}_bank_to_flat"
    t = f"banked_{side}_turn_{radius}"
    return [b] + [t] * count + [e]


def main(pieces, start, out):
    prog = {"ride_type": 52, "start": start, "pieces": pieces}
    Path(out).write_text(json.dumps(prog, indent=2) + "\n")
    sim.simulate(prog, verbose="-v" in sys.argv)
    print(f"{len(pieces)} pieces -> {out}")
