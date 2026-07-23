"""Round 2: 'Corkscrew Timber' - twin-spiral chain lift, slalom serpentine.

Ratings reasoning (RideRatings.cpp, wooden_rc RatingsData):
  * numDrops excitement caps at 9 while its intensity keeps scaling, so the
    layout is built to land on exactly 9 drops.
  * sloped turns are worth up to 97 sub-excitement for ZERO intensity, and
    turn runs are counted per consecutive same-direction piece: 4+ / 3 / 2 / 1
    element buckets. The twin-spiral lift alone books two 4+ sloped runs.
  * banked turns are the next best deal; unbanked hairpins are avoided at speed
    because maxLateralG > 2.80 detonates intensity.
"""
from build import chain, hill, turn180, turn90

A1 = 1
B1 = 0
C1 = 0
D1 = 0
E1 = 12
F1 = 1


def spiral_lift():
    """Twin 360 helix-style chain lift: +288 in a 7x5 footprint, ends dir 0.

    Four consecutive left sloped turns then four right ones = two 4+ element
    sloped turn runs, the single best excitement-per-intensity item there is.
    """
    return chain(["flat_to_up_25"] + ["left_turn_3_up_25"] * 4
                 + ["right_turn_3_up_25"] * 4 + ["up_25", "up_25_to_flat"])


def first_drop():
    """-288, matching the lift exactly. Steep for max speed."""
    return ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60", "down_60",
            "down_60_to_down_25", "down_25", "down_25_to_flat"]


def bank_s(first, n=5):
    """90 one way then 90 back: two 1-element banked turns, net dir unchanged."""
    a = "left" if first == "l" else "right"
    b = "right" if first == "l" else "left"
    return [f"flat_to_{a}_bank", f"banked_{a}_turn_{n}", f"{a}_bank_to_flat",
            f"flat_to_{b}_bank", f"banked_{b}_turn_{n}", f"{b}_bank_to_flat"]


def slalom_hill(first):
    """Airtime hill that climbs into one turn and drops out of the other:
    two 1-element sloped turns and a crest for negative Gs."""
    a = "left" if first == "l" else "right"
    b = "right" if first == "l" else "left"
    return ["flat_to_up_25", f"{a}_turn_3_up_25", "up_25_to_flat",
            "flat_to_down_25", f"{b}_turn_3_down_25", "down_25_to_flat"]


def build():
    p = []
    p += ["begin_station", "middle_station", "middle_station", "end_station"]
    p += ["flat"] * A1
    p += spiral_lift()                       # +288
    p += first_drop()                        # -288, still heading west
    # Pass A tail: airtime straight off the drop.
    p += hill(3, 3)
    p += ["flat"] * B1
    p += turn180("l", banked=True, mid=1)    # dir 2, 6 tiles north
    # Pass B
    p += hill(2, 2)
    p += bank_s("l")
    p += hill(2, 2)
    p += ["flat"] * C1
    p += turn180("r", banked=True, mid=1)    # dir 0
    # Pass C
    p += hill(2, 2)
    p += slalom_hill("l")
    p += hill(1, 1)
    p += ["flat"] * D1
    p += turn180("l", banked=True, mid=1)    # dir 2
    # Pass D
    p += hill(2, 2)
    p += bank_s("r")
    p += hill(1, 1)
    # Home
    p += turn90("l", banked=True)
    p += ["flat"] * E1
    p += turn90("l", banked=True)
    p += ["brakes", "brakes"]
    p += ["flat"] * F1
    return p
