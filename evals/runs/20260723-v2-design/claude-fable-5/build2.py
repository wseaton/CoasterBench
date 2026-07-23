"""Round 2: ground-hugging serpentine woodie.

Design driven by the actual rating maths in RideRatings.cpp:
  * the whole post-drop circuit sits at relative z=0 so every element scores
    PROXIMITY_SURFACE_TOUCH (worth up to ~0.4 excitement, and free of intensity)
  * exactly 9 drop segments: min(9, drops) caps the excitement bonus but the
    intensity term keeps growing, so drop #10 is a pure loss
  * turnarounds are pairs of banked turns (2-element banked turn = best
    excitement per piece), the first drop carries a sloped turn (intensity-free)
  * long-but-undulating, since Duration pays up to 150s with zero intensity
"""

import itertools
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import geo  # noqa: E402


def chain(name):
    return {"t": name, "chain": True}


# 4 tiles, peak z=16, one drop segment, two elements sitting on the ground.
HOP = ["flat_to_up_25", "up_25_to_flat", "flat_to_down_25", "down_25_to_flat"]
# Steep version for the fast section right after the plunge: real airtime.
HOP60 = ["flat_to_up_60", "up_60_to_flat", "flat_to_down_60", "down_60_to_flat"]


def turn180(side):
    """Two consecutive banked turns: one 2-element banked turn in the counters."""
    return [f"flat_to_{side}_bank", f"banked_{side}_turn_5", f"banked_{side}_turn_5",
            f"{side}_bank_to_flat"]


def program(start, fa, fb, ret, runin, lift=12):
    p = []
    p += ["begin_station", "middle_station", "middle_station", "middle_station", "end_station"]
    p += ["flat", "flat"]
    p += [chain("flat_to_up_25")] + [chain("up_25")] * lift + [chain("up_25_to_flat")]
    p += ["flat"]
    # -- The plunge: 60 degrees into a banked-free sloped turn, all one drop.
    p += ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60_to_down_25",
          "right_turn_5_down_25", "down_25_to_flat"]
    # -- Rest of turnaround A (dir 3 -> 0).
    p += ["flat_to_right_bank", "banked_right_turn_5", "right_bank_to_flat"]
    # -- Leg 2 (-x): the fast section, steep airtime hops.
    p += HOP60 + HOP60 + ["flat"] * fa
    p += turn180("left")                       # -> +x
    # -- Leg 3 (+x)
    p += HOP + HOP + ["flat"] * fb
    p += turn180("right")                      # -> -x
    # -- Leg 4 (-x)
    p += HOP + HOP + ["flat"] * fa
    p += turn180("left")                       # -> +x
    # -- Leg 5 (+x)
    p += HOP + ["flat"] * fb
    p += turn180("right")                      # -> -x
    # -- Leg 6 (-x)
    p += HOP + ["flat"] * fa
    # -- Return corridor up the west side, then into the station.
    p += ["right_turn_5"]                      # -x -> +y
    p += ["flat"] * ret
    p += ["right_turn_5"]
    p += ["brakes"] * 2 + ["flat"] * runin
    return {"ride_type": 52, "start": start, "pieces": p}


def solve(start, ranges):
    hits = []
    for fa, fb, ret, runin in itertools.product(*ranges):
        prog = program(start, fa, fb, ret, runin)
        errs, cur, tiles = geo.simulate(prog)
        if errs:
            continue
        if (cur.x, cur.y, cur.z, cur.d, cur.bank, cur.slope) != (
                start["x"] * 32, start["y"] * 32, 0, start["dir"], 0, 0):
            continue
        if any(max(z for _, _, z in v) - min(z for _, _, z in v) < 24
               for v in tiles.values() if len(v) > 1):
            continue
        hits.append((fa, fb, ret, runin))
    return hits


if __name__ == "__main__":
    start = {"x": 30, "y": 72, "dir": 2}
    hits = solve(start, [range(0, 16), range(0, 16), range(0, 40), range(0, 6)])
    print("solutions:", hits[:12])
    if not hits:
        errs, cur, _ = geo.simulate(program(start, 0, 0, 0, 0), verbose=True)
        print(errs, cur)
        sys.exit(1)
    fa, fb, ret, runin = max(hits, key=lambda h: h[0] + h[1])
    out = Path(__file__).parent / "round_2"
    out.mkdir(exist_ok=True)
    (out / "program.json").write_text(json.dumps(program(start, fa, fb, ret, runin), indent=1))
    geo.check(str(out / "program.json"))
