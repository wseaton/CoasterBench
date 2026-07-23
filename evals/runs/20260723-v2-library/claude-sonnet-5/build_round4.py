import json
from geom import run


def chain(*names):
    return [{"t": n, "chain": True} for n in names]


def dz(pieces):
    c, _ = run(pieces, {"x": 0, "y": 0, "dir": 2})
    return c["z"]


def tiles(pieces):
    c, _ = run(pieces, {"x": 0, "y": 0, "dir": 2})
    return c["x"] // 32


station = ["begin_station"] + ["middle_station"] * 5 + ["end_station"]  # 7 tiles

# Chained lift + chained ascending helices, all the way to the apex -- an
# unchained climb right off the lift exit stalls the train (near-zero momentum
# at the top can't crest an extra unpowered rise; it rocks in place forever
# and "tested" never flips). Keep every uphill element on the chain.
climb = chain("flat_to_up_25", *["up_25"] * 12, "up_25_to_flat")  # +208z, 14 tiles
helix_up_r = chain("flat_to_right_bank", "right_helix_up_large", "right_helix_up_large", "right_bank_to_flat")  # +32z, 2 tiles
helix_up_l = chain("flat_to_left_bank", "left_helix_up_small", "left_helix_up_small", "left_bank_to_flat")  # +32z, 2 tiles
wiggle = [
    "flat_to_left_bank", "banked_left_turn_3", "left_bank_to_flat",
    "flat_to_right_bank", "banked_right_turn_3", "right_bank_to_flat",
    "flat_to_right_bank", "banked_right_turn_3", "right_bank_to_flat",
    "flat_to_left_bank", "banked_left_turn_3", "left_bank_to_flat",
]  # dz 0, net rotation 0, mirrored so the lateral s-shift also cancels -- a
   # banked double s-wiggle for extra turn-element variety

LH = ["flat_to_left_bank", "banked_left_turn_5", "banked_left_turn_5", "left_bank_to_flat"]  # 90 deg banked corner

bigdrop = [
    "flat_to_down_25", "down_25_to_down_60", "down_60", "down_60", "down_60",
    "down_60_to_down_25", "down_25_to_flat",
]  # -272z, 7 tiles


def hill():
    return ["flat_to_up_25", "up_25", "up_25_to_flat", "flat_to_down_25", "down_25", "down_25_to_flat"]  # 0z, 6 tiles


sbend = ["s_bend_left", "s_bend_right"]  # 0z, 6 tiles
BRAKES = ["brakes"] * 3  # decelerate before the station so arrival/test-finish triggers

lane1_feat = climb + helix_up_r + helix_up_l + wiggle           # dz +272, tiles 23
lane2_feat = bigdrop + hill() + hill() + hill() + sbend + BRAKES + ["flat"]  # dz -272

print("lane1", dz(lane1_feat), tiles(lane1_feat))
print("lane2", dz(lane2_feat), tiles(lane2_feat))
total = dz(lane1_feat) + dz(lane2_feat)
print("total dz (should be 0):", total)

T2 = tiles(station) + tiles(lane1_feat)
T0 = tiles(lane2_feat)
FILLER = T0 - T2
print("T2", T2, "T0", T0, "filler needed:", FILLER)
assert total == 0, f"dz mismatch {total}, fix lane composition"
assert FILLER >= 0

lane1 = ["flat"] * FILLER + lane1_feat
lane2 = lane2_feat

pieces = station + lane1 + LH + lane2 + LH

start = {"x": 30, "y": 30, "dir": 2}
cur, hist = run(pieces, start)
print("end cursor:", cur, "expect", (start["x"] * 32, start["y"] * 32, 0, 2))
prog = {"ride_type": 52, "start": start, "pieces": pieces}
json.dump(prog, open("program.json", "w"), indent=2)
print("pieces total:", len(pieces))
