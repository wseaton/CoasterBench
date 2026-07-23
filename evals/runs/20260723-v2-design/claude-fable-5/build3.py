"""Round 3: longer serpentine, mixed turn types, gentler valleys.

Changes vs round 2, each aimed at a specific term in RideRatings.cpp:
  * two more turnarounds, and half of them unbanked, because turnCountBanked
    and turnCountDefault are separate counters with separate caps
  * airtime hops keep the sharp 60-degree crest (negative Gs pay excitement up
    to -2.50) but pull out through 25 degrees, which cuts max positive G -- that
    term costs 0.42 intensity per 0.01 g and pays only 0.05 excitement
  * longer circuit: BonusDuration pays up to 150s and BonusLength up to 6000m,
    both with zero intensity
  * still exactly 9 drops, still ground level for the surface-touch proximity
"""

import itertools
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import geo  # noqa: E402


def chain(name):
    return {"t": name, "chain": True}


# Sharp 60-degree crest for airtime, gentle 25-degree pullout to keep +G down.
HOPAIR = ["flat_to_up_25", "up_25_to_up_60", "up_60_to_flat",
          "flat_to_down_60", "down_60_to_down_25", "down_25_to_flat"]
HOPBIG = ["flat_to_up_25", "up_25", "up_25_to_flat",
          "flat_to_down_25", "down_25", "down_25_to_flat"]
HOP = ["flat_to_up_25", "up_25_to_flat", "flat_to_down_25", "down_25_to_flat"]


def t_banked(side):
    """180 degrees of banked turn -> one 2-element entry in turnCountBanked."""
    return [f"flat_to_{side}_bank", f"banked_{side}_turn_5", f"banked_{side}_turn_5",
            f"{side}_bank_to_flat"]


def t_flat(side):
    """180 degrees unbanked -> one 2-element entry in turnCountDefault."""
    return [f"{side}_turn_5", f"{side}_turn_5"]


def program(start, fa, fb, ret, runin, lift=12):
    p = []
    p += ["begin_station", "middle_station", "middle_station", "middle_station", "end_station"]
    p += ["flat", "flat"]
    p += [chain("flat_to_up_25")] + [chain("up_25")] * lift + [chain("up_25_to_flat")]
    p += ["flat"]
    # -- The plunge, all one drop segment, with a sloped turn folded in (sloped
    #    turns are the only turn category that costs zero intensity).
    p += ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60_to_down_25",
          "right_turn_5_down_25", "down_25_to_flat"]
    p += ["flat_to_right_bank", "banked_right_turn_5", "right_bank_to_flat"]  # -> -x
    p += HOPAIR + HOPAIR + ["flat"] * fa
    p += t_banked("left")                      # -> +x
    p += HOPBIG + HOPBIG + ["flat"] * fb
    p += t_flat("right")                       # -> -x
    p += HOPBIG + HOP + ["flat"] * fa
    p += t_banked("left")                      # -> +x
    p += HOP + ["flat"] * fb
    p += t_flat("right")                       # -> -x
    p += HOP + ["flat"] * fa
    p += ["right_turn_5"]                      # -> +y
    p += ["flat"] * ret
    p += ["right_turn_5"]                      # -> +x
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
    start = {"x": 30, "y": 74, "dir": 2}
    hits = solve(start, [range(0, 14), range(0, 14), range(0, 40), range(0, 6)])
    print("solutions:", hits[:12])
    if not hits:
        errs, cur, _ = geo.simulate(program(start, 0, 0, 0, 0), verbose=True)
        print(errs, cur)
        sys.exit(1)
    fa, fb, ret, runin = hits[0]
    out = Path(__file__).parent / "round_3"
    out.mkdir(exist_ok=True)
    (out / "program.json").write_text(json.dumps(program(start, fa, fb, ret, runin), indent=1))
    geo.check(str(out / "program.json"))
