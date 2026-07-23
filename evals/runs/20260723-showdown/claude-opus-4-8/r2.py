#!/usr/bin/env python3
"""Round 2: same out-and-back skeleton, but every turn happens ON A HILL.

Round 1 derailed: wooden coasters have no upstop wheels, so Vehicle.TrackMotion
derails them above 1.5 lateral G, and a 208z lift into a ground-level banked
turn hit 1.74. Lift trimmed to 160z and both turnarounds moved up to z=48, which
scales lateral G by sqrt(112/208) ~= 0.73 -> ~1.28.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from build import lift, hill, banked_turn, main

pieces = (
    ["begin_station", "middle_station", "end_station"]              # -> (53,60) z0
    + lift(9)                                                        # 11t, z=160
    + ["flat_to_down_25", "down_25_to_down_60", "down_60",
       "down_60_to_down_25", "down_25_to_flat"]                      # 5t, -144, z=16
    + hill(1)                                                        # 6t, z=16
    + ["flat_to_up_25", "up_25", "up_25_to_flat"]                    # 3t, +32, z=48
    + banked_turn("left", 5, 2)                                      # 180 left @ z48
    # --- return leg, dir +x, 32 tiles, net dz 0 ---
    + ["flat_to_right_bank", "right_helix_down_large",
       "right_helix_down_large", "right_bank_to_flat"]               # 2t, -32, z=16
    + ["flat_to_down_25", "down_25_to_flat"]                         # 2t, -16, z=0
    + hill(2)                                                        # 8t, peak 48
    + ["s_bend_left", "s_bend_right"]                                # 6t
    + hill(1)                                                        # 6t, peak 32
    + ["flat_to_up_25", "up_25", "up_25", "up_25_to_flat"]           # 4t, +48, z=48
    + ["flat"] * 4                                                   # 4t
    + banked_turn("left", 5, 2)                                      # 180 left @ z48
    # --- descent into the station ---
    + ["flat_to_down_25", "down_25", "down_25", "down_25_to_flat"]   # 4t, -48, z=0
)

main(pieces, {"x": 56, "y": 60, "dir": 0}, Path(__file__).parent / "round_2/program.json")
