"""Round 6: the round-5 double zigzag with the lateral-g bomb defused.

Rounds 4 and 5 reported byte-identical max_speed / +g / -g / lateral g despite
completely different turnarounds, which pins the 3.96 lateral g on the one
element they shared: right_turn_5_down_25 sitting at the very bottom of the
plunge, taken at terminal speed. Above 3.10 lateral g,
ride_ratings_get_excessive_lateral_g_penalty halves the g-force excitement and
adds 12.25 intensity, which is what turned a ~7 into a 1.70.

So the plunge is dead straight now, and the first direction change waits until
a flat run has bled some speed off. Everything else that was working is kept:
ground-level circuit for the surface-touch proximity score, exactly 9 drops,
seven banked and seven unbanked 180s to saturate both 2-element turn buckets.
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

LEG_TILES = 6


def pad(leg):
    return leg + ["flat"] * (LEG_TILES - len(leg))


def turnaround(kind, side):
    """180 degrees on 5-tile turns: 12 tiles, net (-1 along, 5 across)."""
    if kind == "b":
        return [f"flat_to_{side}_bank", f"banked_{side}_turn_5",
                f"banked_{side}_turn_5", f"{side}_bank_to_flat"]
    return ["flat", f"{side}_turn_5", f"{side}_turn_5", "flat"]


def stack(kinds, legs, march, heading):
    out = []
    for i, leg in enumerate(legs):
        out += pad(leg)
        if i == len(legs) - 1:
            break
        side = ("left" if heading == 0 else "right") if march < 0 else \
               ("right" if heading == 0 else "left")
        out += turnaround(kinds[i], side)
        heading = 2 if heading == 0 else 0
    return out, heading


def program(start, run1, run2, cross, ret, runin, lift=15):
    p = []
    p += ["begin_station", "middle_station", "middle_station", "middle_station", "end_station"]
    p += ["flat", "flat"]
    p += [chain("flat_to_up_25")] + [chain("up_25")] * lift + [chain("up_25_to_flat")]
    p += ["flat"]
    # -- 256 z units, dead straight, one drop segment.
    p += ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60",
          "down_60_to_down_25", "down_25", "down_25", "down_25", "down_25_to_flat"]
    # -- Let the speed come off before the first direction change, then turn
    #    banked (a banked turn carries far less lateral load than a sloped one).
    p += ["flat"] * run1
    p += ["flat_to_right_bank", "banked_right_turn_5", "right_bank_to_flat"]   # -> -y
    p += ["flat"] * run2
    p += ["right_turn_5"]                                                      # -> -x
    legs1 = [HOPAIR, [], HOPBIG, [], HOPBIG, [], HOPBIG, []]
    s1, h = stack(["b"] * 7, legs1, -1, 0)
    p += s1
    if h == 2:
        p += turnaround("b", "right")
        h = 0
    p += ["flat"] * cross
    legs2 = [HOPBIG, [], HOP, [], HOP, [], HOP, []]
    s2, h = stack(["f"] * 7, legs2, 1, h)
    p += s2
    if h == 2:
        p += turnaround("f", "left")
        h = 0
    p += ["flat"]
    p += ["right_turn_5"]                                                      # -> +y
    p += ["flat"] * ret
    p += ["right_turn_5"]                                                      # -> +x
    p += ["brakes"] * 2 + ["flat"] * runin
    return {"ride_type": 52, "start": start, "pieces": p}


def solve(start, ranges):
    hits = []
    for run1, run2, cross, ret, runin in itertools.product(*ranges):
        prog = program(start, run1, run2, cross, ret, runin)
        errs, cur, tiles = geo.simulate(prog)
        if errs:
            continue
        if (cur.x, cur.y, cur.z, cur.d, cur.bank, cur.slope) != (
                start["x"] * 32, start["y"] * 32, 0, start["dir"], 0, 0):
            continue
        if any(max(z for _, _, z in v) - min(z for _, _, z in v) < 24
               for v in tiles.values() if len(v) > 1):
            continue
        xs = [t[0] for t in tiles]
        ys = [t[1] for t in tiles]
        if min(xs) < 21 or max(xs) > 66 or min(ys) < 21 or max(ys) > 100:
            continue
        hits.append((run1, run2, cross, ret, runin))
    return hits


if __name__ == "__main__":
    start = {"x": 28, "y": 76, "dir": 2}
    hits = solve(start, [range(4, 12), range(2, 10), range(0, 40),
                         range(0, 24), range(0, 8)])
    print("solutions:", len(hits), hits[:8])
    if hits:
        args = hits[0]
        out = Path(__file__).parent / "round_6"
        out.mkdir(exist_ok=True)
        (out / "program.json").write_text(json.dumps(program(start, *args), indent=1))
        geo.check(str(out / "program.json"))
