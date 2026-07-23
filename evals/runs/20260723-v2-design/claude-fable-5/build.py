"""Round 1: out-and-back woodie with a big 60-degree drop, two airtime hills,
banked turns, s-bends and a double descending helix.

Leg fillers are solved by brute force so the circuit closes exactly.
"""

import itertools
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import geo  # noqa: E402

START = {"x": 38, "y": 60, "dir": 2}


def chain(name):
    return {"t": name, "chain": True}


def program(fill_b, fill_c, fill_d, runin):
    p = []
    # -- Leg A (+x): station, then the chain lift to z=144.
    p += ["begin_station", "middle_station", "middle_station", "middle_station", "end_station"]
    p += ["flat"]
    p += [chain("flat_to_up_25")] + [chain("up_25")] * 8 + [chain("up_25_to_flat")]
    p += ["flat"]
    # -- Turn 1: crest turn, dir 2 -> 3.
    p += ["right_turn_5"]
    # -- Leg B (-y): the 128-unit plunge through 60 degrees, then airtime hill 1.
    p += ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60_to_flat"]
    p += ["flat_to_up_25", "up_25", "up_25", "up_25_to_flat",
          "flat_to_down_25", "down_25", "down_25", "down_25_to_flat"]
    p += ["flat"] * fill_b
    # -- Turn 2: banked, dir 3 -> 0.
    p += ["flat_to_right_bank", "banked_right_turn_5", "right_bank_to_flat"]
    # -- Leg C (-x): s-bend wiggle + airtime hill 2, climbing to z=32.
    p += ["s_bend_left", "s_bend_right"]
    p += ["flat"] * fill_c
    p += ["flat_to_up_25", "up_25", "up_25_to_flat", "flat_to_down_25", "down_25_to_flat"]
    # -- Turn 3: banked, dir 0 -> 1.
    p += ["flat_to_right_bank", "banked_right_turn_5", "right_bank_to_flat"]
    # -- Leg D (+y): double descending helix back to ground.
    p += ["flat"] * fill_d
    p += ["flat_to_right_bank", "right_helix_down_small", "right_helix_down_small",
          "right_bank_to_flat"]
    # -- Turn 4: dir 1 -> 2, then brake run into the station.
    p += ["right_turn_5"]
    p += ["brakes"] * 2 + ["flat"] * runin
    return {"ride_type": 52, "start": START, "pieces": p}


def solve():
    hits = []
    for fb, fc, fd, ri in itertools.product(range(0, 8), range(0, 14), range(0, 14), range(0, 10)):
        prog = program(fb, fc, fd, ri)
        errs, cur, tiles = geo.simulate(prog)
        if errs:
            continue
        if (cur.x, cur.y, cur.z, cur.d, cur.bank, cur.slope) != (
                START["x"] * 32, START["y"] * 32, 0, START["dir"], 0, 0):
            continue
        # Reject origin-tile reuse where the two passes are too close vertically.
        if any(max(z for _, _, z in v) - min(z for _, _, z in v) < 24
               for v in tiles.values() if len(v) > 1):
            continue
        hits.append((fb, fc, fd, ri))
    return hits


if __name__ == "__main__":
    hits = solve()
    print("closing solutions:", hits[:10])
    if not hits:
        prog = program(0, 0, 0, 0)
        errs, cur, _ = geo.simulate(prog, verbose=True)
        print(errs, cur)
        sys.exit(1)
    fb, fc, fd, ri = hits[0]
    out = Path(__file__).parent / "round_1"
    out.mkdir(exist_ok=True)
    (out / "program.json").write_text(json.dumps(program(fb, fc, fd, ri), indent=1))
    geo.check(str(out / "program.json"))
