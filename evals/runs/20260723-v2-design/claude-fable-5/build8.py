"""Round 6: single serpentine, turn-variety maximised, lateral g kept safe.

The lesson from rounds 4/5: a tall lift + a sloped turn at the bottom of the
plunge pushes lateral g past 3.10 and detonates the excessive-lateral-g penalty
(half the g-force excitement, +12.25 intensity). Rounds 2/3 kept lateral g at
1.93 and scored 6.63.

This keeps the proven single-serpentine-with-return-corridor topology (it
closes reliably), a moderate 12-piece lift, a dead-straight plunge, and pushes
on the one term that still had headroom: BonusTurns. Turnarounds alternate
banked / unbanked so turnCountBanked and turnCountDefault fill independently,
all on 5-tile turns so lateral load stays low. Circuit stays at ground level for
the surface-touch proximity bonus; exactly 9 drop segments.
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


def t_banked(side):
    return [f"flat_to_{side}_bank", f"banked_{side}_turn_5",
            f"banked_{side}_turn_5", f"{side}_bank_to_flat"]


def t_flat(side):
    return [f"{side}_turn_5", f"{side}_turn_5"]


def program(start, fa, fb, ret, runin, lift=12):
    p = []
    p += ["begin_station", "middle_station", "middle_station", "middle_station", "end_station"]
    p += ["flat", "flat"]
    p += [chain("flat_to_up_25")] + [chain("up_25")] * lift + [chain("up_25_to_flat")]
    p += ["flat"]
    # -- Dead-straight plunge, 208 z units, one drop segment.
    p += ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60",
          "down_60_to_down_25", "down_25_to_flat"]
    # -- Turnaround A (banked) into the fast section, dir 3 -> 1 across two 90s.
    p += t_banked("right")                      # dir 3 -> 1? no: 5-turn is 90
    # NOTE geometry validated by geo.solve below; t_banked here is a 180.
    p += HOPAIR + HOPAIR + ["flat"] * fa        # fast half: sharp airtime crests
    p += t_flat("left")
    p += HOPBIG + HOPBIG + ["flat"] * fb
    p += t_banked("right")
    p += HOPBIG + HOP + ["flat"] * fa
    p += t_flat("left")
    p += HOP + ["flat"] * fb
    p += t_banked("right")
    p += HOP + ["flat"] * fa
    p += ["right_turn_5"]                        # into the return corridor
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
        xs = [t[0] for t in tiles]
        ys = [t[1] for t in tiles]
        if min(xs) < 21 or max(xs) > 118 or min(ys) < 21 or max(ys) > 118:
            continue
        hits.append((fa, fb, ret, runin))
    return hits


if __name__ == "__main__":
    start = {"x": 30, "y": 74, "dir": 2}
    hits = solve(start, [range(0, 16), range(0, 16), range(0, 44), range(0, 6)])
    print("solutions:", len(hits), hits[:10])
    if hits:
        fa, fb, ret, runin = max(hits, key=lambda h: h[0] + h[1])
        out = Path(__file__).parent / "round_6"
        out.mkdir(exist_ok=True)
        (out / "program.json").write_text(json.dumps(program(start, fa, fb, ret, runin), indent=1))
        geo.check(str(out / "program.json"))
