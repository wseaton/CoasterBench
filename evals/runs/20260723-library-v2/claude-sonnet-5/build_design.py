import json, sys
from geom import run

def chain(*names):
    return [{"t": n, "chain": True} for n in names]

station = ["begin_station", "middle_station", "end_station"]

climb = chain("flat_to_up_25", *["up_25"]*8, "up_25_to_flat")   # +144z, 10 tiles
helix_up = ["flat_to_right_bank", "right_helix_up_large", "right_helix_up_large", "right_bank_to_flat"]  # +32z, 2 tiles (bulges away from lane2)
LH = ["flat_to_left_bank", "banked_left_turn_5", "banked_left_turn_5", "left_bank_to_flat"]

bigdrop = ["flat_to_down_25", "down_25_to_down_60", "down_60", "down_60_to_down_25", "down_25_to_flat"]  # -144z, 5 tiles
def hill():
    return ["flat_to_up_25", "up_25", "up_25_to_flat", "flat_to_down_25", "down_25", "down_25_to_flat"]  # 0z, 6 tiles
helix_down = ["flat_to_right_bank", "right_helix_down_large", "right_helix_down_large", "right_bank_to_flat"]  # -32z, 2 tiles (bulges away from lane1)
sbend = ["s_bend_left", "s_bend_right"]  # 0z, 6 tiles

FILLER1 = 10  # solve below

lane1 = ["flat"]*FILLER1 + climb + helix_up
lane2 = bigdrop + hill() + hill() + helix_down + sbend

pieces = station + lane1 + LH + lane2 + LH

start = {"x": 30, "y": 30, "dir": 2}
cur, hist = run(pieces, start)
print("end cursor:", cur)
print("start x,y*32:", start["x"]*32, start["y"]*32)
prog = {"ride_type": 52, "start": start, "pieces": pieces}
json.dump(prog, open("program.json", "w"), indent=2)
print("pieces total:", len(pieces))
