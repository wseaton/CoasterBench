import json, sys
from geom import run

def chain(*names):
    return [{"t": n, "chain": True} for n in names]

station = ["begin_station"] + ["middle_station"] * 5 + ["end_station"]  # 7 tiles, extra length so a fast train fully brakes before the loop registers as arrived

climb = chain("flat_to_up_25", *["up_25"]*8, "up_25_to_flat")   # +144z, 10 tiles
# Chained (not coasting) -- an unchained climb straight off the lift's exit, with
# barely any momentum at the top, was the actual stall: the train crept into the
# helix and never had the kinetic energy to crest the extra +32z, so it rocked in
# place forever instead of ever completing a lap (hence "tested" never flips).
helix_up = chain("flat_to_right_bank", "right_helix_up_large", "right_helix_up_large", "right_bank_to_flat")  # +32z, 2 tiles (bulges away from lane2)
LH = ["flat_to_left_bank", "banked_left_turn_5", "banked_left_turn_5", "left_bank_to_flat"]

bigdrop = ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60_to_down_25", "down_25_to_flat"]  # -144z, 5 tiles
def hill():
    return ["flat_to_up_25", "up_25", "up_25_to_flat", "flat_to_down_25", "down_25", "down_25_to_flat"]  # 0z, 6 tiles
helix_down = ["flat_to_right_bank", "right_helix_down_large", "right_helix_down_large", "right_bank_to_flat"]  # -32z, 2 tiles (bulges away from lane1)
sbend = ["s_bend_left", "s_bend_right"]  # 0z, 6 tiles

FILLER1 = 9  # solve below (station grew from 3 to 7 tiles; +3 brakes in lane2 balanced here)

BRAKES = ["brakes"] * 3  # decelerate before re-entering the station so arrival/test-finish actually triggers

lane1 = ["flat"]*FILLER1 + climb + helix_up
lane2 = bigdrop + hill() + hill() + helix_down + sbend + BRAKES

pieces = station + lane1 + LH + lane2 + LH

start = {"x": 30, "y": 30, "dir": 2}
cur, hist = run(pieces, start)
print("end cursor:", cur)
print("start x,y*32:", start["x"]*32, start["y"]*32)
prog = {"ride_type": 52, "start": start, "pieces": pieces}
json.dump(prog, open("program.json", "w"), indent=2)
print("pieces total:", len(pieces))
