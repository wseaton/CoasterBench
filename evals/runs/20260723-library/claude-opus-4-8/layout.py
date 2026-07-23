#!/usr/bin/env python3
"""Layout generator: a wooden out-and-back built from parallel columns.

Topology (net quarter-turns must sum to a multiple of 4):

    station (y=72, heading +x)
      -> R90 (+1)
      -> C1  x=36 heading north
      -> TA  banked left 180 (-2)   -> +5x, heading south
      -> C2  x=41 heading south
      -> TA  banked right 180 (+2)  -> +5x, heading north
      -> C3  x=46 heading north
      -> TA  banked left 180 (-2)   -> +5x, heading south
      -> C4  x=51 heading south
      -> L90 (-1) onto the y=77 return straight heading west
      -> L90, L90 (-2) U-turn back into the station
                                  net = 1-2+2-2-1-2 = -4  (closed)

Every valley sits on the ground (z=0): track elements touching the surface are
worth proximity points, and it keeps the whole descent budget available for
hills instead of spending it on the turnarounds.

Box x 22..66, y 22..78 is flat in this scenario (verified by probe).
"""
import sys

sys.path.insert(0, __file__.rsplit('/', 1)[0])
import build  # noqa: E402

START = (28, 72, 2)


def lift(t, dz):
    """Chain lift rising exactly dz over dz/16 + 1 tiles.

    Wooden coasters reject a 60 degree chain ("too steep for lift hill"), so
    25 degrees it is; the lift is therefore long, and lift time is dead time
    that drags the average-speed rating down. Height is not free.
    """
    m = (dz - 16) // 16
    assert m >= 0 and 16 + 16 * m == dz, dz
    t.add("flat_to_up_25", chain=True)
    t.add("up_25", chain=True, n=m)
    t.add("up_25_to_flat", chain=True)


def _steep_parts(dz):
    k = (dz - 80) // 64
    m = (dz - 80 - 64 * k) // 16
    assert k >= 0 and m >= 0 and 80 + 64 * k + 16 * m == dz, dz
    return k, m


def ascend(t, dz, steep=False):
    """Climb exactly dz, flat in to flat out."""
    if steep:
        k, m = _steep_parts(dz)
        t.add("flat_to_up_25")
        t.add("up_25_to_up_60")
        t.add("up_60", n=k)
        t.add("up_60_to_up_25")
        t.add("up_25", n=m)
        t.add("up_25_to_flat")
    else:
        m = (dz - 16) // 16
        assert m >= 0 and 16 + 16 * m == dz, dz
        t.add("flat_to_up_25")
        t.add("up_25", n=m)
        t.add("up_25_to_flat")


def descend(t, dz, steep=False):
    if steep:
        k, m = _steep_parts(dz)
        t.add("flat_to_down_25")
        t.add("down_25_to_down_60")
        t.add("down_60", n=k)
        t.add("down_60_to_down_25")
        t.add("down_25", n=m)
        t.add("down_25_to_flat")
    else:
        m = (dz - 16) // 16
        assert m >= 0 and 16 + 16 * m == dz, dz
        t.add("flat_to_down_25")
        t.add("down_25", n=m)
        t.add("down_25_to_flat")


def plunge(t, dz):
    """Sharp crest: off the flat top straight into 60 degrees. dz = 64 + 16m."""
    m = (dz - 64) // 16
    assert m >= 0 and 64 + 16 * m == dz, dz
    t.add("flat_to_down_60")
    t.add("down_60_to_down_25")
    t.add("down_25", n=m)
    t.add("down_25_to_flat")


def hill(t, dz):
    """Airtime hill: gentle climb, sharp crest, steep plunge back to ground."""
    ascend(t, dz)
    plunge(t, dz) if dz >= 64 else descend(t, dz)


def weave(t):
    """S-bend pair: 6 tiles forward, no net lateral shift."""
    t.add("s_bend_right")
    t.add("s_bend_left")


def bank_turn(t, side, radius=5, n=2):
    """Flat banked turn. Two radius-5 quarters make a 180: from north, +5x +1y."""
    t.add(f"flat_to_{side}_bank")
    t.add(f"banked_{side}_turn_{radius}", n=n)
    t.add(f"{side}_bank_to_flat")


def helix(t, side, direction="down"):
    """180 degree banked helix, +5 tiles across, -/+16 z. Counts as a helix."""
    t.add(f"flat_to_{side}_bank")
    t.add(f"{side}_helix_{direction}_large")
    t.add(f"{side}_bank_to_flat")


OPS = {
    "lift": lift,
    "hill": hill,
    "weave": lambda t, _: weave(t),
    "drop": lambda t, _: descend(t, t.z, steep=True),
    "flat": lambda t, n: t.add("flat", n=n),
    "brakes": lambda t, n: t.add("brakes", n=n),
    "helixL": lambda t, _: helix(t, "left"),
    "helixR": lambda t, _: helix(t, "right"),
    "bankL": lambda t, n: bank_turn(t, "left", n=n),
    "bankR": lambda t, n: bank_turn(t, "right", n=n),
}


def run_ops(t, ops):
    for op in ops:
        name, arg = (op, None) if isinstance(op, str) else op
        OPS[name](t, arg)


def build_track(p):
    t = build.Track(*START)
    t.add("begin_station")
    t.add("middle_station", n=p.get("stations", 3))
    t.add("end_station")
    t.add("flat")
    t.add("right_turn_5")                       # -> (36,69) heading north

    for i, (ops, end_y) in enumerate(p["columns"]):
        run_ops(t, ops)
        if end_y is not None:
            t.run_to("y", end_y)
        if i < len(p["columns"]) - 1:
            bank_turn(t, "left" if i % 2 == 0 else "right", p.get("radius", 5))

    t.add("left_turn_5")                        # -> (48,77) heading west
    run_ops(t, p.get("tail", []))
    t.run_to("x", p["brake_x"])
    t.run_to("x", 26, "brakes")
    t.add("left_turn_5")                        # -> (24,74) heading north
    t.add("left_turn_5")                        # -> (27,72) heading east
    t.add("flat")
    return t


PARAMS = {
    "columns": [
        ([("lift", 272), "drop", ("hill", 112)], 29),
        ([("hill", 96), "weave", ("hill", 80)], 64),
        ([("hill", 80), "weave", ("hill", 64)], 33),
        ([("hill", 64), "weave", ("hill", 48), ("hill", 32)], 75),
    ],
    "brake_x": 32,
}

if __name__ == "__main__":
    t = build_track(PARAMS)
    print("end:", t.at, "pieces:", len(t.pieces))
    t.dump(sys.argv[1] if len(sys.argv) > 1 else "/tmp/v1.json", START)
