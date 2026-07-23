"""Round 1 layout: serpentine woodie with an east return corridor.

Geometry notes measured off the game's own tables (geom.py):
  up_25 = 16z, up_60 = 64z, flat_to_up_25 = 8z, up_25_to_up_60 = 32z
  flat 180 (turn5 + flat + turn5)      : (+1, -6) in the dir-0 frame
  banked 180 (mid=1)                   : (+2, -6)
  left turn decrements dir, right increments; dir 0=-x, 1=+y, 2=+x, 3=-y
"""
from build import lift, steep_drop, hill, turn180, turn90, helix

# tuned so the circuit closes exactly
A1 = 2    # flats between station and lift
B1 = 1    # tail flats on pass B
C1 = 1    # tail flats on pass C
D1 = 2    # tail flats on pass D
E1 = 7    # corridor flats
F1 = 1    # flats before the station


def build():
    p = []
    p += ["begin_station", "middle_station", "middle_station", "end_station"]
    p += ["flat"] * A1
    p += lift(12)                       # +208
    p += turn180("l", banked=False, mid=1)
    # Pass B: the money shot, 208-unit steep drop then two airtime hills.
    p += steep_drop(2)                  # -208, lands at station level
    p += hill(3, 3)
    p += ["s_bend_right"]
    p += hill(2, 2)
    p += ["flat"] * B1
    p += turn180("r", banked=True, mid=1)
    # Pass C
    p += hill(3, 3)
    p += ["s_bend_left"]
    p += hill(2, 2)
    p += ["flat"] * C1
    p += turn180("l", banked=True, mid=1)
    # Pass D
    p += hill(2, 2)
    p += hill(1, 1)
    p += ["flat"] * D1
    # Into the return corridor (dir 1, +y) and home.
    p += turn90("l", banked=True)
    p += ["flat"] * E1
    p += turn90("l", banked=True)
    p += ["brakes", "brakes"]
    p += ["flat"] * F1
    return p
