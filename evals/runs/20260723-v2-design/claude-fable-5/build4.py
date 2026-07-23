"""Round 4: turn-dense double serpentine.

Rounds 2 and 3 both landed on 6.63; length and duration are done paying
(BonusDuration caps at 150s, BonusLength is only 0.0133 excitement per metre).
The one big untapped term is BonusTurns, whose sub-rating tops out around 250
against the ~32 we were scoring.

turnCountBanked / turnCountDefault / turnCountSloped are three independent
counters, each with its own per-size cap (kTurnMask2Elements = 7). A 2-element
turn (two consecutive turn pieces, i.e. a 180 turnaround) is the best
excitement-per-tile entry in both the banked and the unbanked table, so this
layout is a tight zigzag of 14 180-degree turnarounds -- 7 banked, 7 unbanked --
using 3-tile turns to keep the footprint small. Two stacked serpentines: one
marching -y, one marching +y a bit further west.
"""

import itertools
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import geo  # noqa: E402


def chain(name):
    return {"t": name, "chain": True}


HOPAIR = ["flat_to_up_25", "up_25_to_up_60", "up_60_to_flat",
          "flat_to_down_60", "down_60_to_down_25", "down_25_to_flat"]
HOPBIG = ["flat_to_up_25", "up_25", "up_25_to_flat",
          "flat_to_down_25", "down_25", "down_25_to_flat"]
HOP = ["flat_to_up_25", "up_25_to_flat", "flat_to_down_25", "down_25_to_flat"]


def turnaround(kind, side):
    if kind == "b":
        return [f"flat_to_{side}_bank", f"banked_{side}_turn_3",
                f"banked_{side}_turn_3", f"{side}_bank_to_flat"]
    return [f"{side}_turn_3", f"{side}_turn_3", "flat"]


def serpentine(kinds, legs, march, heading):
    """Zigzag of legs along x joined by 180s; `march` is the y direction."""
    out = []
    for i, leg in enumerate(legs):
        out += leg
        if i == len(legs) - 1:
            break
        if march < 0:
            side = "left" if heading == 0 else "right"
        else:
            side = "right" if heading == 0 else "left"
        out += turnaround(kinds[i], side)
        heading = 2 if heading == 0 else 0
    return out, heading


def program(start, fc, pre, ret, runin, lift=14):
    fill = ["flat"] * fc
    p = []
    p += ["begin_station", "middle_station", "middle_station", "middle_station", "end_station"]
    p += ["flat", "flat"]
    p += [chain("flat_to_up_25")] + [chain("up_25")] * lift + [chain("up_25_to_flat")]
    p += ["flat"]
    # -- Plunge: one drop segment, with an intensity-free sloped turn inside it.
    p += ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60_to_down_25",
          "right_turn_5_down_25", "down_25", "down_25", "down_25_to_flat"]
    p += ["flat"] * pre
    p += ["right_turn_3"]                       # dir 3 -> 0, heading -x
    # -- Serpentine 1: marching -y. Fast half, so the airtime hops live here.
    legs1 = [HOPAIR, fill, HOPAIR, fill, HOPBIG, fill, HOPBIG, fill]
    kinds1 = ["b", "f", "b", "f", "b", "f", "b"]
    s1, h = serpentine(kinds1, legs1, -1, 0)
    p += s1
    # -- Cross over to the western stack.
    if h == 2:                                  # make sure we leave heading -x
        p += turnaround("f", "right")
        h = 0
    p += ["flat"] * fc
    # -- Serpentine 2: marching +y, back up the west side.
    legs2 = [HOPBIG, fill, HOP, fill, HOP, fill, HOP, fill]
    kinds2 = ["f", "b", "f", "b", "f", "b", "f"]
    s2, h = serpentine(kinds2, legs2, 1, h)
    p += s2
    # -- Home: west corridor up to station level, then east into the platform.
    if h == 2:
        p += turnaround("f", "left")
        h = 0
    p += ["flat"] * 1
    p += ["right_turn_3"]                       # -x -> +y
    p += ["flat"] * ret
    p += ["right_turn_3"]                       # +y -> +x
    p += ["brakes"] * 2 + ["flat"] * runin
    return {"ride_type": 52, "start": start, "pieces": p}


def solve(start, ranges):
    hits = []
    for fc, pre, ret, runin in itertools.product(*ranges):
        prog = program(start, fc, pre, ret, runin)
        errs, cur, tiles = geo.simulate(prog)
        if errs:
            continue
        if (cur.x, cur.y, cur.z, cur.d, cur.bank, cur.slope) != (
                start["x"] * 32, start["y"] * 32, 0, start["dir"], 0, 0):
            continue
        if any(max(z for _, _, z in v) - min(z for _, _, z in v) < 24
               for v in tiles.values() if len(v) > 1):
            continue
        hits.append((fc, pre, ret, runin))
    return hits


if __name__ == "__main__":
    start = {"x": 26, "y": 74, "dir": 2}
    hits = solve(start, [range(0, 12), range(0, 12), range(0, 44), range(0, 8)])
    print("solutions:", hits[:12])
    if not hits:
        errs, cur, _ = geo.simulate(program(start, 2, 2, 10, 2), verbose=True)
        print(errs, cur)
        sys.exit(1)
    fc, pre, ret, runin = hits[0]
    out = Path(__file__).parent / "round_4"
    out.mkdir(exist_ok=True)
    (out / "program.json").write_text(json.dumps(program(start, fc, pre, ret, runin), indent=1))
    geo.check(str(out / "program.json"))
