"""Builds the track program from named blocks and checks closure with geom.py.

Layout: serpentine out-and-back in the y band north of the station, returning
down an east corridor that never touches the passes (no crossings at all, so
no clearance guesswork).
"""
import json
import pathlib
import sys

import geom

START = {"x": 58, "y": 62, "dir": 0}


def chain(n_pieces):
    return [{"t": p, "chain": True} for p in n_pieces]


# --- blocks -----------------------------------------------------------------
def lift(n):
    """Chain lift: rises 8 + 16n + 8."""
    return chain(["flat_to_up_25"] + ["up_25"] * n + ["up_25_to_flat"])


def steep_drop(n60):
    """Steep drop: falls 8 + 32 + 48n + 32 + 8."""
    return ["flat_to_down_25", "down_25_to_down_60"] + ["down_60"] * n60 + [
        "down_60_to_down_25", "down_25_to_flat"]


def hill(n_up, n_down=None, flat_top=0):
    """Airtime hill: up 8+16n+8, flat crest, back down. Net 0 if n_up == n_down."""
    n_down = n_up if n_down is None else n_down
    return (["flat_to_up_25"] + ["up_25"] * n_up + ["up_25_to_flat"]
            + ["flat"] * flat_top
            + ["flat_to_down_25"] + ["down_25"] * n_down + ["down_25_to_flat"])


def steep_hill(n60_up, n60_down):
    return (["flat_to_up_25", "up_25_to_up_60"] + ["up_60"] * n60_up
            + ["up_60_to_up_25", "up_25_to_flat", "flat_to_down_25", "down_25_to_down_60"]
            + ["down_60"] * n60_down + ["down_60_to_down_25", "down_25_to_flat"])


def turn180(side, banked, mid=0):
    """180 degrees. side in {l, r}. Banked version needs the transitions."""
    t = "left" if side == "l" else "right"
    if banked:
        return ([f"flat_to_{t}_bank"] + [f"banked_{t}_turn_5"] + [f"{t}_bank"] * mid
                + [f"banked_{t}_turn_5", f"{t}_bank_to_flat"])
    return [f"{t}_turn_5"] + ["flat"] * mid + [f"{t}_turn_5"]


def turn90(side, banked):
    t = "left" if side == "l" else "right"
    if banked:
        return [f"flat_to_{t}_bank", f"banked_{t}_turn_5", f"{t}_bank_to_flat"]
    return [f"{t}_turn_5"]


def helix(side, updown, size):
    t = "left" if side == "l" else "right"
    return [f"flat_to_{t}_bank", f"{t}_helix_{updown}_{size}", f"{t}_bank_to_flat"]


def report(pieces, start=START):
    r = geom.simulate(pieces, (start["x"], start["y"], start["dir"]), 0)
    closed = (r["x"], r["y"], r["z"], r["dir"], r["roll"], r["pitch"]) == (
        start["x"], start["y"], 0, start["dir"], "none", "none")
    cs = geom.collisions(r["foot"])
    xs = [f[0] for f in r["foot"]]
    ys = [f[1] for f in r["foot"]]
    print(f"pieces={len(pieces)} end=({r['x']},{r['y']},z={r['z']},dir={r['dir']},"
          f"{r['roll']}/{r['pitch']}) {'CLOSED' if closed else 'OPEN'}")
    print(f"  bbox x {min(xs)}..{max(xs)} y {min(ys)}..{max(ys)} "
          f"z 0..{max(f[3] for f in r['foot'])} collisions={len(cs)}")
    for c in cs[:8]:
        print("   !", c)
    return r, closed, cs


def write(pieces, path):
    prog = {"ride_type": 52, "start": START, "pieces": pieces}
    pathlib.Path(path).write_text(json.dumps(prog, indent=1))
    print("wrote", path, len(pieces), "pieces")


if __name__ == "__main__":
    mod = __import__(sys.argv[1].replace(".py", ""))
    pieces = mod.build()
    report(pieces)
    if len(sys.argv) > 2:
        write(pieces, sys.argv[2])
