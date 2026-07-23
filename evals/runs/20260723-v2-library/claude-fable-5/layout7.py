"""Round 3: 'Corkscrew Timber II'.

Change from round 2: every 180 turnaround now sits on a +64 shelf reached by a
climb and left by a dive. Round 2 measured maxLateralG 2.76 and the penalty
cliff is 2.80 (+3.75 raw intensity, which at 8.78 would have blown past the
10.00 excitement-quartering bound). Turning where the train is slowest is the
cheap fix, and the climb/dive pairs simply replace hills, so numDrops stays at
the excitement cap of 9.

The passes deliberately stay at ground level: PROXIMITY_SURFACE_TOUCH pays up
to 120 proximity points (~+0.41 excitement) for track whose base sits on the
surface, which an elevated layout throws away.
"""
from build import chain, hill, turn180, turn90

A1 = 2
B1 = 0
C1 = 6
E1 = 4
CORR = 24
MID1 = 6
MID2 = 6
MID3 = 12
SHELF = 3


def spiral_lift():
    """Twin 360 chain spiral: +288 in ~8 tiles, and two 4+ element sloped turn
    runs (sloped turns are the only rating bonus with zero intensity cost)."""
    return chain(["flat_to_up_25"] + ["left_turn_3_up_25"] * 4
                 + ["right_turn_3_up_25"] * 4 + ["up_25", "up_25_to_flat"])


def first_drop():
    return ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60", "down_60",
            "down_60_to_down_25", "down_25", "down_25_to_flat"]


def climb(n):
    return ["flat_to_up_25"] + ["up_25"] * n + ["up_25_to_flat"]


def dive(n):
    return ["flat_to_down_25"] + ["down_25"] * n + ["down_25_to_flat"]


def bank_s(first, n=5):
    a = "left" if first == "l" else "right"
    b = "right" if first == "l" else "left"
    return [f"flat_to_{a}_bank", f"banked_{a}_turn_{n}", f"{a}_bank_to_flat",
            f"flat_to_{b}_bank", f"banked_{b}_turn_{n}", f"{b}_bank_to_flat"]


def slalom(first="l", n=5):
    other = "r" if first == "l" else "l"
    return bank_s(first, n) + bank_s(other, n)


def slalom_hill(first):
    a = "left" if first == "l" else "right"
    b = "right" if first == "l" else "left"
    return ["flat_to_up_25", f"{a}_turn_3_up_25", "up_25_to_flat",
            "flat_to_down_25", f"{b}_turn_3_down_25", "down_25_to_flat"]


def build():
    p = []
    # --- Pass A (dir 0, y=62)
    p += ["begin_station", "middle_station", "middle_station", "end_station"]
    p += spiral_lift()                       # +288
    p += first_drop()                        # -288, back to ground level
    p += ["flat"] * A1
    p += climb(SHELF)                        # onto the turnaround shelf
    p += turn180("l", banked=True, mid=MID1)
    # --- Pass B (dir 2)
    p += dive(SHELF)
    p += hill(3, 3)
    p += hill(2, 2)
    p += ["flat"] * B1
    p += climb(SHELF)
    p += turn180("r", banked=True, mid=MID2)
    # --- Pass C (dir 0)
    p += dive(SHELF)
    p += slalom_hill("r")
    p += slalom_hill("l")
    p += ["flat"] * C1
    p += climb(SHELF)
    p += turn90("r", banked=True)
    # --- West corridor south (dir 1)
    p += dive(SHELF)
    p += ["flat"] * CORR
    p += turn90("r", banked=True)
    # --- Pass D: finale south of the station (dir 2)
    p += hill(2, 2)
    p += slalom("r")
    p += ["flat"] * E1
    p += turn180("r", banked=True, mid=MID3)
    return p
