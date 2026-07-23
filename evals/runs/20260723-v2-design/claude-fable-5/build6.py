"""Round 5: same double zigzag, but on 5-tile turns.

Round 4 ran the turnarounds on 3-tile turns and hit 3.96 lateral g, which
trips ride_ratings_get_excessive_lateral_g_penalty above 3.10: it halves the
whole g-force excitement term and dumps 12.25 onto intensity. 5-tile turns
roughly halve the lateral load, and the banked ones all sit in the fast half
of the circuit while the unbanked ones wait until the train has slowed.

Original notes:

turnCountBanked and turnCountDefault are separate counters and each caps its
2-element bucket at 7, so seven banked and seven unbanked 180s is exactly the
point where both buckets saturate. Legs are all the same tile length so the
zigzag doesn't drift in x (a turnaround shifts x by one against the leg
direction, which cancels between an out leg and a back leg of equal length).

Stack 1 walks -y down the east side, a straight run carries the train west,
stack 2 walks +y back up, and the station is at the top of the west side.
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
    """180 degrees, 12 tiles, net (-1 along the leg, 5 across) either way."""
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
        if march < 0:
            side = "left" if heading == 0 else "right"
        else:
            side = "right" if heading == 0 else "left"
        out += turnaround(kinds[i], side)
        heading = 2 if heading == 0 else 0
    return out, heading


def program(start, pre, cross, ret, runin, lift=16):
    p = []
    p += ["begin_station", "middle_station", "middle_station", "middle_station", "end_station"]
    p += ["flat", "flat"]
    p += [chain("flat_to_up_25")] + [chain("up_25")] * lift + [chain("up_25_to_flat")]
    p += ["flat"]
    # -- One drop segment, 272 z units, with a sloped turn folded in (the only
    #    turn category that adds excitement at zero intensity).
    p += ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60",
          "down_60_to_down_25", "right_turn_5_down_25", "down_25_to_flat"]
    p += ["flat"] * pre
    p += ["right_turn_3"]                        # dir 3 -> 0, heading -x
    legs1 = [HOPAIR, [], HOPBIG, [], HOPBIG, [], HOPBIG, []]
    s1, h = stack(["b"] * 7, legs1, -1, 0)
    p += s1
    if h == 2:                                   # leave the east stack heading -x
        p += turnaround("f", "right")
        h = 0
    p += ["flat"] * cross                        # run west to the second stack
    legs2 = [HOPBIG, [], HOP, [], HOP, [], HOP, []]
    s2, h = stack(["f"] * 7, legs2, 1, h)
    p += s2
    if h == 2:
        p += turnaround("f", "left")
        h = 0
    p += ["flat"]
    p += ["right_turn_3"]                        # -x -> +y
    p += ["flat"] * ret
    p += ["right_turn_3"]                        # +y -> +x
    p += ["brakes"] * 2 + ["flat"] * runin
    return {"ride_type": 52, "start": start, "pieces": p}


def solve(start, ranges):
    hits = []
    for pre, cross, ret, runin in itertools.product(*ranges):
        prog = program(start, pre, cross, ret, runin)
        errs, cur, tiles = geo.simulate(prog)
        if errs:
            continue
        if (cur.x, cur.y, cur.z, cur.d, cur.bank, cur.slope) != (
                start["x"] * 32, start["y"] * 32, 0, start["dir"], 0, 0):
            continue
        if any(max(z for _, _, z in v) - min(z for _, _, z in v) < 24
               for v in tiles.values() if len(v) > 1):
            continue
        hits.append((pre, cross, ret, runin))
    return hits


if __name__ == "__main__":
    start = {"x": 30, "y": 74, "dir": 2}
    hits = solve(start, [range(0, 12), range(0, 46), range(0, 26), range(0, 10)])
    print("solutions:", hits[:10], len(hits))
    if hits:
        pre, cross, ret, runin = hits[0]
        out = Path(__file__).parent / "round_5"
        out.mkdir(exist_ok=True)
        (out / "program.json").write_text(
            json.dumps(program(start, pre, cross, ret, runin), indent=1))
        geo.check(str(out / "program.json"))
