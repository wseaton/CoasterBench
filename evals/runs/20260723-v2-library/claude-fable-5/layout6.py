"""Round 3: 'Corkscrew Timber II' - stepped-down serpentine.

Two changes over round 2, both aimed at the intensity budget rather than raw
excitement:
  * every 180 turnaround sits on a raised shelf, so the train is at its slowest
    where the turn is sharpest. Round 2 hit maxLateralG 2.76 and the cliff at
    2.80 costs +3.75 raw intensity, which would have wrecked the score.
  * the passes step DOWN 16z each, so the back half of the circuit still has
    potential energy to spend. Average speed is worth 5.6 excitement per unit
    and round 2's train was crawling home.
Shelf dives replace hills one-for-one, keeping numDrops at exactly 9 (the
excitement cap; intensity keeps scaling past it).

Height ledger: lift +352, first drop -288 -> pass A base 64; each shelf is
+64 and each dive -80, so the pass bases step 64 -> 48 -> 32, and the corridor
dive (-96) lands the finale at station level.
"""
from build import chain, hill, turn180, turn90

A1 = 4
B1 = 3
C1 = 5
E1 = 4
CORR = 30
MID1 = 6
MID2 = 6
MID3 = 13
SHELF = 3
STEP = 4


def spiral_lift(n_flat=5):
    return chain(["flat_to_up_25"] + ["left_turn_3_up_25"] * 4
                 + ["right_turn_3_up_25"] * 4 + ["up_25"] * n_flat + ["up_25_to_flat"])


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
    # --- Pass A (dir 0, y=62): station, twin-spiral lift, 288-unit first drop
    p += ["begin_station", "middle_station", "middle_station", "end_station"]
    p += spiral_lift()
    p += first_drop()
    p += ["flat"] * A1
    p += climb(SHELF)
    p += turn180("l", banked=True, mid=MID1)
    # --- Pass B (dir 2)
    p += dive(STEP)
    p += hill(3, 3)
    p += slalom("l")
    p += ["flat"] * B1
    p += climb(SHELF)
    p += turn180("r", banked=True, mid=MID2)
    # --- Pass C (dir 0)
    p += dive(STEP)
    p += slalom_hill("r")
    p += slalom_hill("l")
    p += hill(2, 2)
    p += ["flat"] * C1
    p += climb(SHELF)
    p += turn90("r", banked=True)
    # --- West corridor south (dir 1), diving back to station level
    p += dive(STEP + 1)
    p += ["flat"] * CORR
    p += turn90("r", banked=True)
    # --- Pass D: finale south of the station (dir 2)
    p += hill(2, 2)
    p += slalom("r")
    p += ["flat"] * E1
    p += turn180("r", banked=True, mid=MID3)
    return p
