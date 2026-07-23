#!/usr/bin/env python3
"""Offline cursor simulator: parse TrackData.cpp TEDs, replay a program, report closure.

Mirrors RustBridge.cpp orct2_host_track_place cursor advance so we can check
geometry (closure, collisions, transitions) without paying for a full eval run.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
TRACKDATA = ROOT / "src/openrct2/ride/TrackData.cpp"
PIECES = ROOT / "rust/orct2-agent/src/pieces.rs"

src = TRACKDATA.read_text()
for h in sorted((ROOT / "src/openrct2/ride/ted").glob("TED.*.h")):
    src += "\n" + h.read_text()

# name -> (coords tuple, pitchBegin, pitchEnd, rollBegin, rollEnd)
teds = {}
for m in re.finditer(r"constexpr auto (kTED\w+) = TrackElementDescriptor\{(.*?)\n    \};", src, re.S):
    name, body = m.group(1), m.group(2)
    c = re.search(r"\.coordinates\s*=\s*\{([^}]*)\}", body)
    if not c:
        continue
    coords = tuple(int(x.strip()) for x in c.group(1).split(","))
    d = re.search(r"\.definition\s*=\s*\{(.*?)\}", body, re.S)
    defn = [x.strip() for x in d.group(1).split(",")] if d else []
    teds[name] = (coords, defn)

# array order gives enum id
arr = re.search(r"kTrackElementDescriptors = std::to_array<TrackElementDescriptor>\(\{(.*?)\n    \}\);", src, re.S)
order = [x.strip().rstrip(",") for x in arr.group(1).strip().splitlines()]
order = [x.rstrip(",") for x in order if x.startswith("kTED")]

by_id = {}
for i, n in enumerate(order):
    if n in teds:
        by_id[i] = (n,) + teds[n]

catalog = {}
for m in re.finditer(r'\("(\w+)", (\d+)\)', PIECES.read_text()):
    catalog[m.group(1)] = int(m.group(2))

DIRDELTA = [(-32, 0), (0, 32), (32, 0), (0, -32)]


def rotate(x, y, d):
    return [(x, y), (y, -x), (-x, -y), (-y, x)][d & 3]


def simulate(prog, verbose=True):
    st = prog["start"]
    x, y, z, d = st["x"] * 32, st["y"] * 32, st.get("z", 0), st["dir"] & 3
    occupied = {}
    ok = True
    for i, p in enumerate(prog["pieces"]):
        name = p["t"] if isinstance(p, dict) else p
        tid = catalog.get(name)
        if tid is None or tid not in by_id:
            print(f"  [{i}] UNKNOWN PIECE {name}")
            ok = False
            break
        tname, coords, defn = by_id[tid]
        rb, re_, zb, ze, cx, cy = coords
        # occupancy: approximate by start tile of the piece
        key = (x // 32, y // 32, z)
        occupied.setdefault(key, []).append(i)
        ox, oy = rotate(cx, cy, d)
        nx, ny, nz = x + ox, y + oy, z - zb + ze
        nd = (d + re_ - rb) & 3
        if (re_ & 4) == 0:
            dx, dy = DIRDELTA[nd]
            nx, ny = nx + dx, ny + dy
        x, y, z, d = nx, ny, nz, nd
        pitch_end = defn[2] if len(defn) > 2 else "?"
        roll_end = defn[4] if len(defn) > 4 else "?"
        if verbose:
            print(f"  [{i:3d}] {name:24s} -> tile({x//32},{y//32}) z={z} dir={d} "
                  f"pitch={pitch_end.split('::')[-1]} roll={roll_end.split('::')[-1]}")
    print(f"END tile=({x//32},{y//32}) z={z} dir={d}")
    st0 = (st["x"], st["y"], st.get("z", 0), st["dir"] & 3)
    end = (x // 32, y // 32, z, d)
    print(f"START tile=({st0[0]},{st0[1]}) z={st0[2]} dir={st0[3]}")
    print("CLOSED" if end == st0 else f"NOT CLOSED  delta=({end[0]-st0[0]},{end[1]-st0[1]},{end[2]-st0[2]},rot {(end[3]-st0[3])%4})")
    dupes = {k: v for k, v in occupied.items() if len(v) > 1}
    if dupes:
        print(f"WARNING duplicate start tiles: {dupes}")
    return ok


if __name__ == "__main__":
    prog = json.loads(Path(sys.argv[1]).read_text())
    simulate(prog, verbose="-q" not in sys.argv)
