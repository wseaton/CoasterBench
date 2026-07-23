#!/usr/bin/env python3
"""Helper to build a rectangular 4-corner track program and sanity-check
that opposite sides have equal forward-tile counts (translation closure)
and that every ascending piece type has a matching descending mirror
somewhere in the whole loop (height closure), without needing to know the
engine's actual z-delta magnitudes.

Confirmed empirically (round_1, round_2 of 20260723-design-v2):
  - up_25 <-> down_25, flat_to_up_25 <-> flat_to_down_25,
    up_25_to_flat <-> down_25_to_flat all cancel exactly when used in
    equal counts.
  - up_25_to_up_60 <-> down_60_to_down_25, up_60 <-> down_60,
    up_60_to_up_25 <-> down_25_to_down_60 also cancel exactly (round_2's
    closure succeeded with these paired 1:1; the crash there was a ride
    dynamics problem, not a geometry problem).
  - flat_to_up_60 does NOT mirror up_60_to_flat (off by +48 in round_1
    testing) - avoid pairing those two as a self-contained "pop".
  - Chained pieces cannot exceed 25 degrees ("Too steep for lift hill").
"""
import json
import sys

MIRROR = {
    "up_25": "down_25",
    "flat_to_up_25": "flat_to_down_25",
    "up_25_to_flat": "down_25_to_flat",
    "up_25_to_up_60": "down_60_to_down_25",
    "up_60": "down_60",
    "up_60_to_up_25": "down_25_to_down_60",
}
MIRROR.update({v: k for k, v in MIRROR.items()})

FORWARD_TYPES = {
    "flat", "begin_station", "middle_station", "end_station",
    "up_25", "up_60", "down_25", "down_60",
    "flat_to_up_25", "up_25_to_up_60", "up_60_to_up_25", "up_25_to_flat",
    "flat_to_down_25", "down_25_to_down_60", "down_60_to_down_25", "down_25_to_flat",
    "flat_to_up_60", "up_60_to_flat", "flat_to_down_60", "down_60_to_flat",
    "left_bank", "right_bank", "flat_to_left_bank", "flat_to_right_bank",
    "left_bank_to_flat", "right_bank_to_flat",
}
CORNER_TYPES = {
    "left_turn_5", "right_turn_5", "banked_left_turn_5", "banked_right_turn_5",
}


def piece_name(p):
    return p["t"] if isinstance(p, dict) else p


def check(sides, corners):
    """sides: list of 4 piece-lists (side1..side4). corners: list of 4 corner piece names."""
    assert len(sides) == 4 and len(corners) == 4
    counts = [sum(1 for p in s if piece_name(p) in FORWARD_TYPES) for s in sides]
    ok = True
    if counts[0] != counts[2]:
        print(f"FAIL: side1({counts[0]}) != side3({counts[2]})")
        ok = False
    if counts[1] != counts[3]:
        print(f"FAIL: side2({counts[1]}) != side4({counts[3]})")
        ok = False
    for c in corners:
        if c not in CORNER_TYPES:
            print(f"FAIL: unknown corner type {c}")
            ok = False

    # height bookkeeping: count every ascend/descend piece across the WHOLE
    # loop and make sure each type's count matches its mirror's count.
    from collections import Counter
    all_pieces = []
    for s in sides:
        all_pieces.extend(piece_name(p) for p in s)
    cnt = Counter(all_pieces)
    for t, mirror in MIRROR.items():
        if cnt[t] != cnt.get(mirror, 0):
            print(f"FAIL: {t} count={cnt[t]} but mirror {mirror} count={cnt.get(mirror,0)}")
            ok = False
    if ok:
        print(f"OK: side counts {counts}, total pieces {sum(counts)+4}")
    return ok


def build_program(ride_type, start, sides, corners):
    pieces = []
    for i in range(4):
        pieces.extend(sides[i])
        pieces.append(corners[i])
    return {"ride_type": ride_type, "start": start, "pieces": pieces}
