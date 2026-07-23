#!/usr/bin/env python3
"""Round 3: rating-formula-driven rebuild.

Read RideRatings.cpp for the wooden RC modifier table and optimised against it:
  * BonusTurns counts maximal runs of turning elements, bucketed 1/2/3/4+, and
    scores banked and SLOPED runs separately. A chained 360-degree spiral lift
    (4x left_turn_3_up_25) is a free 4-element sloped turn: 7.5 rating points,
    zero intensity, and it climbs 128z in a 3x3 footprint.
  * BonusDrops is dominated by the drop COUNT (11.1 pts each up to 9), not drop
    height (0.24 pts per height unit). So: many hills, not one huge one.
  * The lateral-G excitement term saturates at exactly 1.50, which is also the
    derail threshold. Apex trimmed to 144z and every banked turn kept at z>=48
    so lateral lands ~1.35 instead of gambling on 1.49.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from build import hill, banked_turn, main

spiral_lift = [{"t": "flat_to_up_25", "chain": True}] + \
              [{"t": "left_turn_3_up_25", "chain": True}] * 4 + \
              [{"t": "up_25_to_flat", "chain": True}]          # +144z, 360 left

pieces = (
    ["begin_station", "middle_station", "end_station"]
    + spiral_lift
    # 60-degree plunge, -144z
    + ["flat_to_down_60", "down_60", "down_60_to_down_25", "down_25", "down_25_to_flat"]
    + hill(2)                                                   # 8t, peak 48
    + hill(1)                                                   # 6t, peak 32
    + ["flat_to_up_25", "up_25", "up_25", "up_25_to_flat"]      # 4t, +48
    + banked_turn("left", 5, 2)                                 # 180 left @ z48
    # --- return leg, dir +x ---
    + hill(1)                                                   # 6t, peak 80
    + ["flat_to_right_bank", "right_helix_down_large",
       "right_helix_down_large", "right_bank_to_flat"]          # 2t, -32
    + ["flat_to_down_25", "down_25_to_flat"]                    # 2t, -16 -> z0
    + hill(2)                                                   # 8t, peak 48
    + ["s_bend_left", "s_bend_right"]                           # 6t
    + hill(1)                                                   # 6t, peak 32
    + ["flat_to_up_60", "up_60_to_flat"]                        # 2t, +48
    + ["flat"] * int(sys.argv[1] if len(sys.argv) > 1 else 0)   # closure padding
    + banked_turn("left", 5, 2)                                 # 180 left @ z48
    + ["flat_to_down_25", "down_25", "down_25", "down_25_to_flat"]  # 4t, -48
)

main(pieces, {"x": 56, "y": 60, "dir": 0}, Path(__file__).parent / "round_3/program.json")
