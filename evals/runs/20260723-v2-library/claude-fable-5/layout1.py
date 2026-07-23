"""Round 1 layout: 'Timberwolf Run'.

Serpentine woodie: station -> lift -> 224-unit steep first drop -> three
return passes of airtime hills and banked turnarounds -> helix -> east
corridor home. No track crossings by construction (passes live in the
y 42..62 band, the return corridor sits east of every turnaround).
"""
from build import lift, steep_drop, hill, steep_hill, turn180, turn90, helix


def build():
    p = []
    # Station, heading dir 0 (-x) from (62, 62).
    p += ["begin_station", "middle_station", "middle_station", "end_station"]
    p += ["flat", "flat"]
    # Lift: +224.
    p += lift(13)
    # Top turnaround, slow so unbanked.
    p += turn180("l", banked=False, mid=1)
    # Pass B (dir 2, +x): first drop then two airtime hills.
    p += steep_drop(3)                 # -224
    p += hill(2, 2)                    # crest +40
    p += hill(2, 2)
    p += ["flat"]
    # East turnaround, fast: banked.
    p += turn180("r", banked=True, mid=1)
    # Pass C (dir 0, -x): steep hill, s-bend, hill.
    p += steep_hill(1, 1)
    p += ["s_bend_left"]
    p += hill(2, 2)
    p += ["flat"]
    # West turnaround.
    p += turn180("l", banked=True, mid=1)
    # Pass D (dir 2, +x): hill, helix, hill.
    p += hill(1, 1)
    p += helix("r", "down", "small")   # -16
    p += hill(1, 1)
    p += ["flat", "flat"]
    # Into the east corridor, heading dir 1 (+y).
    p += turn90("l", banked=True)
    p += ["flat"] * 6
    p += turn90("l", banked=True)
    p += ["brakes", "brakes", "flat"]
    return p
