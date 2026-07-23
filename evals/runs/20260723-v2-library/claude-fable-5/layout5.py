"""Round 2: 'Corkscrew Timber'.

Circuit: three serpentine passes marching north, a west corridor back south,
then a finale straight south of the station and a wide hairpin into it.
Rotation ledger: L180 (-2) R180 (+2) R90 (+1) R90 (+1) R180 (+2) = +4 = closed.

Design targets pulled straight out of RideRatings.cpp (wooden_rc RatingsData):
  * exactly 9 drops - excitement caps there, intensity keeps scaling
  * two 4+ element sloped turn runs from the twin-spiral lift: sloped turns are
    worth up to 97 sub-excitement at ZERO intensity cost
  * turn runs are counted per consecutive same-direction piece, so slaloms of
    single turns separated by straights farm the 1-element buckets
  * everything fast is banked; maxLateralG > 2.80 detonates intensity
"""
from build import chain, hill, turn180, turn90

MID1 = 6
MID2 = 6
MID3 = 8
CORR = 24
A1 = 1
B1 = 0
C1 = 0
D1 = 4
E1 = 0


def spiral_lift():
    return chain(["flat_to_up_25"] + ["left_turn_3_up_25"] * 4
                 + ["right_turn_3_up_25"] * 4 + ["up_25", "up_25_to_flat"])


def first_drop():
    return ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60", "down_60",
            "down_60_to_down_25", "down_25", "down_25_to_flat"]


def bank_s(first, n=5):
    """90 one way then 90 back: lateral shift, two 1-element banked turns."""
    a = "left" if first == "l" else "right"
    b = "right" if first == "l" else "left"
    return [f"flat_to_{a}_bank", f"banked_{a}_turn_{n}", f"{a}_bank_to_flat",
            f"flat_to_{b}_bank", f"banked_{b}_turn_{n}", f"{b}_bank_to_flat"]


def slalom(first="l", n=5):
    """Out and back: four 1-element banked turns, net lateral offset zero."""
    other = "r" if first == "l" else "l"
    return bank_s(first, n) + bank_s(other, n)


def slalom_hill(first):
    """Climbs into one turn, drops out of the other: two 1-element sloped
    turns and one drop."""
    a = "left" if first == "l" else "right"
    b = "right" if first == "l" else "left"
    return ["flat_to_up_25", f"{a}_turn_3_up_25", "up_25_to_flat",
            "flat_to_down_25", f"{b}_turn_3_down_25", "down_25_to_flat"]


def build():
    p = []
    # --- Pass A (dir 0, y=62): station, twin-spiral lift, 288-unit first drop
    p += ["begin_station", "middle_station", "middle_station", "end_station"]
    p += ["flat"] * A1
    p += spiral_lift()
    p += first_drop()
    p += hill(3, 3)
    p += ["flat"] * B1
    p += turn180("l", banked=True, mid=MID1)
    # --- Pass B (dir 2, y=51)
    p += hill(2, 2)
    p += slalom("l")
    p += hill(2, 2)
    p += ["flat"] * C1
    p += turn180("r", banked=True, mid=MID2)
    # --- Pass C (dir 0, y=40)
    p += hill(2, 2)
    p += slalom_hill("r")
    p += slalom_hill("l")
    p += hill(1, 1)
    p += ["flat"] * D1
    # --- West corridor south (dir 1)
    p += turn90("r", banked=True)
    p += ["flat"] * CORR
    p += turn90("r", banked=True)
    # --- Pass D: finale south of the station (dir 2)
    p += hill(1, 1)
    p += slalom("r")
    p += ["flat"] * E1
    p += turn180("r", banked=True, mid=MID3)
    return p
