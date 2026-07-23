"""Round 1: 'Splinterjack' - serpentine woodie, helix turnarounds, east corridor.

Turnaround inventory (all measured with geom.py):
  T1 flat 180 left   from dir0 : (+1, -6)      slow, top of the lift
  T2 right down-helix large     : (-1, -5, -16) from dir2, ends dir0
  T3 left  up-helix   large     : (+1, -5, +16) from dir0, ends dir2
  T4/T5 banked left 90          : (+3, +4) from dir2, (-4, +3) from dir1
"""
from build import lift, steep_drop, hill, turn180, turn90, helix

A1 = 3
B1 = 0
C1 = 1
D1 = 5
E1 = 12
F1 = 1


def build():
    p = []
    p += ["begin_station", "middle_station", "middle_station", "end_station"]
    p += ["flat"] * A1
    p += lift(16)                              # +272
    p += turn180("l", banked=False, mid=1)     # top turnaround, dir 2
    # Pass B: 272-unit steep first drop, then airtime.
    p += steep_drop(3)                         # -272 back to station level
    p += hill(3, 3)
    p += ["s_bend_right"]
    p += hill(1, 1)
    p += ["flat"] * B1
    p += helix("r", "up", "large")             # T2, +16 (pass C runs one level up)
    # Pass C (dir 0)
    p += hill(3, 3)
    p += ["s_bend_left"]
    p += hill(2, 2)
    p += hill(1, 1)
    p += ["flat"] * C1
    p += helix("l", "down", "large")           # T3, -16 back to station level
    # Pass D (dir 2)
    p += hill(2, 2)
    p += ["s_bend_right"]
    p += hill(2, 2)
    p += hill(1, 1)
    p += ["flat"] * D1
    # Return corridor and home straight.
    p += turn90("l", banked=True)
    p += ["flat"] * E1
    p += turn90("l", banked=True)
    p += ["brakes", "brakes"]
    p += ["flat"] * F1
    return p
