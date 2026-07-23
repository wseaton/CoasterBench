#!/usr/bin/env python3
"""Round 1: out-and-back woodie. 208z lift, 60-degree plunge, airtime hills,
descending double helix on the return leg."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from build import lift, hill, banked_turn, main

pieces = (
    ["begin_station", "middle_station", "end_station"]
    + lift(12)                                    # +208z, 14 tiles
    # 60-degree plunge, -208z, 6 tiles
    + ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60",
       "down_60_to_down_25", "down_25_to_flat"]
    + hill(2)                                     # 8 tiles
    + banked_turn("left", 5, 2)                   # 180 left, return leg at y-5
    # --- return leg, dir +x, 31 tiles ---
    + ["flat_to_up_25", "up_25", "up_25", "up_25_to_flat"]          # +48z, 4
    + ["flat_to_left_bank", "left_helix_down_small", "left_helix_down_small",
       "left_bank_to_flat"]                                          # -32z, 2
    + ["flat_to_down_25", "down_25_to_flat"]                         # -16z, 2
    + ["s_bend_left", "s_bend_right"]                                # 6
    + hill(2)                                                        # 8
    + ["s_bend_right", "s_bend_left"]                                # 6
    + ["brakes", "flat", "flat"]                                     # 3
    + banked_turn("left", 5, 2)                   # 180 left back into station
)

main(pieces, {"x": 60, "y": 60, "dir": 0}, Path(__file__).parent / "round_1/program.json")
