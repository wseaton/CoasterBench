import json
from geom import run

def chain(*names):
    return [{"t": n, "chain": True} for n in names]

station = ["begin_station"] + ["middle_station"] * 5 + ["end_station"]  # 7 tiles

# Chained lift + chained ascending helix, all the way to the apex -- an
# unchained climb right off the lift exit is what stalled round 1: near-zero
# momentum at the top can't crest an extra unpowered rise, so the train just
# rocks in place forever and "tested" never flips.
climb = chain("flat_to_up_25", *["up_25"] * 12, "up_25_to_flat")  # +208z, 14 tiles
helix_up = chain(
    "flat_to_right_bank", "right_helix_up_large", "right_helix_up_large", "right_bank_to_flat"
)  # +32z, 2 tiles (bulges away from lane2, since it's a right helix on a left-turning loop)
LH = ["flat_to_left_bank", "banked_left_turn_5", "banked_left_turn_5", "left_bank_to_flat"]  # 90 deg banked corner

bigdrop = [
    "flat_to_down_25", "down_25_to_down_60", "down_60", "down_60", "down_60_to_down_25", "down_25_to_flat",
]  # -208z, 6 tiles


def hill():
    return ["flat_to_up_25", "up_25", "up_25_to_flat", "flat_to_down_25", "down_25", "down_25_to_flat"]  # 0z, 6 tiles


helix_down = [
    "flat_to_right_bank", "right_helix_down_large", "right_helix_down_large", "right_bank_to_flat",
]  # -32z, 2 tiles (coasting downhill, no chain needed)
sbend = ["s_bend_left", "s_bend_right"]  # 0z, 6 tiles
BRAKES = ["brakes"] * 3  # decelerate before the station so arrival/test-finish triggers

FILLER1 = 12

lane1 = ["flat"] * FILLER1 + climb + helix_up
lane2 = bigdrop + hill() + hill() + hill() + helix_down + sbend + BRAKES

pieces = station + lane1 + LH + lane2 + LH

start = {"x": 30, "y": 30, "dir": 2}
cur, hist = run(pieces, start)
print("end cursor:", cur)
print("start x,y*32:", start["x"] * 32, start["y"] * 32)
prog = {"ride_type": 52, "start": start, "pieces": pieces}
json.dump(prog, open("program.json", "w"), indent=2)
print("pieces total:", len(pieces))
