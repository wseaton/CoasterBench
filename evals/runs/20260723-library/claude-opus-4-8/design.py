#!/usr/bin/env python3
"""The competition design: six-column out-and-back plus a station flyover lobe.

    station (28..32, y=72, heading +x)
      -> R90                       -> C1 x=36 north: the chain lift
      -> banked 180 at the top     -> C2 x=41 south: the big drop + airtime
      -> ... six columns, x = 36,41,46,51,56,61, turnarounds alternating
         left/right so the columns march east.  The turnarounds at the south
         end sit 64 units up and pass OVER the y=77 return straight, which is
         worth own-track proximity points and keeps lateral G off the turns.
      -> L90 onto the y=77 return straight heading west
      -> climb, L90, and fly over the station itself at x=29
      -> lobe: x=29 north, banked 180, x=24 south, brakes
      -> R90 back into the station

Rating notes that drove the shape (see RideRatings.cpp, WoodenRollerCoaster.h):
  * excitement clamps negative G at -2.50 and lateral G at +1.50, but intensity
    keeps charging for both, and intensity over 10.00 collapses excitement.  So
    every crest and turn is tuned to sit just under those clamps.
  * the drop bonus caps at 9 drops; extra drops are pure intensity.
  * length and duration are near-free excitement, hence the 2000m+ layout.
  * s-bends turned out to be the main source of lateral G, so there are none.
"""
import sys

sys.path.insert(0, __file__.rsplit('/', 1)[0])
import build  # noqa: E402
import layout  # noqa: E402

START = layout.START

layout.OPS["ghill"] = lambda t, dz: (layout.ascend(t, dz), layout.descend(t, dz))
layout.OPS["up"] = lambda t, dz: layout.ascend(t, dz)
layout.OPS["dn"] = lambda t, dz: layout.plunge(t, dz) if dz >= 64 else layout.descend(t, dz)
layout.OPS["gdn"] = lambda t, dz: layout.descend(t, dz)
# crest flavours, in order of how hard they throw you out of the seat
layout.OPS["mhill"] = lambda t, dz: (layout.ascend(t, dz), layout.descend(t, dz, steep=True))


def build_track(p):
    t = build.Track(*START)
    t.add("begin_station")
    t.add("middle_station", n=p.get("stations", 3))
    t.add("end_station")
    t.add("flat", n=4 - p.get("stations", 3))
    t.add("right_turn_5")                          # -> (36,69) heading north

    for i, (ops, end_y) in enumerate(p["columns"]):
        layout.run_ops(t, ops)
        if end_y is not None:
            t.run_to("y", end_y)
        if i < len(p["columns"]) - 1:
            layout.bank_turn(t, "left" if i % 2 == 0 else "right")

    t.add("left_turn_5")                           # -> (58,77) heading west
    t.run_to("x", p.get("climb_x", 35))
    layout.ascend(t, p["fly"])                     # up for the station flyover
    if p.get("bank_tail"):
        t.run_to("x", 32)
        layout.bank_turn(t, "left", n=1)           # -> (28,73) heading north
    else:
        t.run_to("x", 31)
        t.add("left_turn_5")                       # -> (29,74) heading north
    t.run_to("y", 71)                              # x=28/29 passes over the station
    layout.descend(t, p["fly"])
    layout.run_ops(t, p.get("lobe_up", []))
    t.run_to("y", p["lobe_top"])
    layout.bank_turn(t, "right")                   # -> (24,...) heading south
    layout.run_ops(t, p.get("lobe_dn", []))
    if p.get("bank_tail"):
        t.run_to("y", p["brake_y"])
        t.run_to("y", 69, "brakes")
        layout.bank_turn(t, "right", n=1)          # -> (28,72), back at the station
    else:
        t.run_to("y", p["brake_y"])
        t.run_to("y", 70, "brakes")
        t.add("right_turn_5")                      # -> (27,72) heading east
        t.add("flat")
    return t


COLUMNS = [
    [("lift", 640)],                                   # C1: the chain lift
    ["drop", ("ghill", 128), ("up", 64)],              # C2: 640-unit drop
    [("gdn", 64), ("ghill", 112), ("ghill", 96)],      # C3
    [("ghill", 80), ("up", 64)],                       # C4
    [("gdn", 64), ("mhill", 80)],                      # C5
    [("mhill", 80)],                                   # C6
]
ENDS = (28, 75, 27, 75, 27, 75)

# Round 5: excitement 7.90, intensity 9.95, similarity 0.24 (nearest: Titan).
# brake_y == the final run_to target means no brake run at all: braking early
# halved the average speed, which is worth ~0.06 excitement per unit.
PARAMS = {
    "columns": list(zip(COLUMNS, ENDS)),
    "stations": 5,
    "fly": 48,
    "lobe_up": [],
    "lobe_dn": [],
    "lobe_top": 26,
    "brake_y": 70,
}

if __name__ == "__main__":
    t = build_track(PARAMS)
    print("end:", t.at, "pieces:", len(t.pieces))
    t.dump(sys.argv[1] if len(sys.argv) > 1 else "/tmp/design.json", START)
