#!/usr/bin/env python3
"""Round 4: farm the BonusTurns / helix terms.

BonusTurns buckets each maximal run of turning elements by length and scores
2- and 3-element runs at 3.75 rating points each (runs of 4+ score NOTHING for
banked turns), and GetSpecialTrackElementsRating pays 3.89 per helix element up
to 9. A pair of large half-helices is therefore the best element in the game
here: one 2-element banked turn (3.75) + two helices (7.8) in two tiles of
travel, with ZERO net rotation so it never disturbs circuit closure.

So: a helix staircase on a high deck. Everything that turns lives at z >= 96;
with a 192z apex that keeps lateral G near 1.35, under the 1.50 derail line.
Ground level is strictly straight airtime hills, where speed is highest.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from build import hill, banked_turn, main

PAD = int(sys.argv[1]) if len(sys.argv) > 1 else 0

spiral_lift = ([{"t": "flat_to_up_25", "chain": True}]
               + [{"t": "left_turn_3_up_25", "chain": True}] * 4
               + [{"t": "up_25", "chain": True}] * 3
               + [{"t": "up_25_to_flat", "chain": True}])            # +192z, 360 left


def dhelix(side, updown):
    """Full-circle double helix: a 2-element banked turn + 2 helices, net rot 0."""
    return [f"flat_to_{side}_bank", f"{side}_helix_{updown}_large",
            f"{side}_helix_{updown}_large", f"{side}_bank_to_flat"]


def plunge_128():
    return ["flat_to_down_60", "down_60", "down_60_to_down_25", "down_25_to_flat"]


def plunge_96():
    return ["flat_to_down_25", "down_25_to_down_60", "down_60_to_down_25",
            "down_25", "down_25_to_flat"]


def climb(n):
    """+16 + 16n z over n+2 tiles."""
    return ["flat_to_up_25"] + ["up_25"] * n + ["up_25_to_flat"]


pieces = (
    ["begin_station", "middle_station", "end_station"]
    + spiral_lift                                     # z=192  [sloped 4-elem turn]
    + dhelix("right", "down") + ["flat"] * 4          # z=160  [banked 2-elem #1]
    + dhelix("right", "down")                         # z=128  [banked 2-elem #2]
    + plunge_128()                                    # z=0    [drop 1, 16 units]
    + hill(1)                                         # [drop 2]
    + climb(5)                                        # z=96
    + banked_turn("left", 5, 2)                       # hairpin [banked 2-elem #3]
    # --- return lane, dir +x, deck at z>=96 ---
    + dhelix("right", "up") + ["flat"] * 4            # z=128  [banked 2-elem #4]
    + dhelix("right", "down")                         # z=96   [banked 2-elem #5]
    + plunge_96()                                     # z=0    [drop 3, 12 units]
    + hill(2)                                         # [drop 4]
    + hill(1)                                         # [drop 5]
    + ["s_bend_left", "s_bend_right"]
    + ["flat"] * PAD
    + climb(5)                                        # z=96
    + banked_turn("left", 5, 2)                       # hairpin [banked 2-elem #7]
    + ["flat_to_down_25"] + ["down_25"] * 5 + ["down_25_to_flat"]   # z=0 [drop 6]
)

main(pieces, {"x": 58, "y": 52, "dir": 0}, Path(__file__).parent / "round_5/program.json")
