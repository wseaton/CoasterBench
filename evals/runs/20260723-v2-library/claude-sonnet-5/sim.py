#!/usr/bin/env python3
"""Offline geometry checker for track programs.

Mirrors the cursor-advance math in RustBridge.cpp orct2_host_track_place and
the per-sequence tile footprints from the TED tables, so closure and
self-intersection can be checked before burning a full eval run.
"""
import json, re, sys, glob, os

REPO = os.path.expanduser("~/git/OpenRCT2")

def parse_ted():
    files = [f"{REPO}/src/openrct2/ride/TrackData.cpp"] + glob.glob(f"{REPO}/src/openrct2/ride/ted/TED.*.h")
    src = "\n".join(open(f).read() for f in files)
    seqs = {}
    for m in re.finditer(r'SequenceDescriptor (k\w+) = \{\s*\.clearance = \{([^}]*)\}', src):
        vals = [v.strip() for v in m.group(2).split(',')]
        seqs[m.group(1)] = (int(vals[0]), int(vals[1]), int(vals[2]))
    teds = {}
    for m in re.finditer(r'(kTED\w+) = TrackElementDescriptor\{(.*?)\n    \};', src, re.S):
        name, body = m.group(1), m.group(2)
        c = re.search(r'\.coordinates = \{([^}]*)\}', body)
        s = re.search(r'\.sequenceData = \{\s*\d+,\s*\{(.*?)\}\s*\}', body, re.S)
        if not c:
            continue
        teds[name] = {
            "coords": [int(v.strip()) for v in c.group(1).split(',')],
            "seqs": [seqs.get(n.strip(), (0, 0, 0)) for n in s.group(1).split(',') if n.strip()] if s else [(0, 0, 0)],
        }
    arr = open(f"{REPO}/src/openrct2/ride/TrackData.cpp").read()
    arr = arr[arr.index('kTrackElementDescriptors = std::to_array'):]
    arr = arr[:arr.index('});')]
    order = re.findall(r'(kTED\w+),', arr)
    return {i: teds[n] for i, n in enumerate(order) if n in teds}

def parse_catalog():
    src = open(f"{REPO}/rust/orct2-agent/src/pieces.rs").read()
    return {m.group(1): int(m.group(2)) for m in re.finditer(r'\("(\w+)", (\d+)\)', src)}

TED = parse_ted()
CAT = parse_catalog()
DELTA = [(-32, 0), (0, 32), (32, 0), (0, -32)]

def rot(x, y, d):
    return [(x, y), (y, -x), (-x, -y), (-y, x)][d & 3]

def simulate(prog, verbose=True):
    cur = {"x": prog["start"]["x"] * 32, "y": prog["start"]["y"] * 32, "z": 0, "d": prog["start"]["dir"] & 3}
    start = dict(cur)
    occupied = {}
    errors = []
    minz = 0
    for i, p in enumerate(prog["pieces"]):
        t = p["t"] if isinstance(p, dict) else p
        tid = CAT[t] if isinstance(t, str) else t
        ted = TED[tid]
        rb, re_, zb, ze, cx, cy = ted["coords"]
        d = cur["d"]
        for (sx, sy, sz) in ted["seqs"]:
            ox, oy = rot(sx, sy, d)
            tile = ((cur["x"] + ox) // 32, (cur["y"] + oy) // 32, (cur["z"] - zb + sz))
            key = (tile[0], tile[1])
            zs = occupied.setdefault(key, [])
            for (oz, oi) in zs:
                if abs(oz - tile[2]) < 24 and oi != i - 1:
                    errors.append(f"piece {i} ({t}) tile {key} z={tile[2]} conflicts with piece {oi} z={oz}")
            zs.append((tile[2], i))
        ox, oy = rot(cx, cy, d)
        nx, ny = cur["x"] + ox, cur["y"] + oy
        nz = cur["z"] - zb + ze
        nd = (d + re_ - rb) & 3
        if (re_ & 4) == 0:
            nx += DELTA[nd][0]
            ny += DELTA[nd][1]
        cur = {"x": nx, "y": ny, "z": nz, "d": nd}
        minz = min(minz, nz)
        if verbose:
            print(f"{i:3d} {t:24s} -> tile({nx//32:3d},{ny//32:3d}) z={nz:4d} dir={nd}")
    print("start:", start["x"] // 32, start["y"] // 32, 0, start["d"])
    print("end:  ", cur["x"] // 32, cur["y"] // 32, cur["z"], cur["d"])
    closed = (cur["x"] == start["x"] and cur["y"] == start["y"] and cur["z"] == 0 and cur["d"] == start["d"])
    print("CLOSED" if closed else "*** NOT CLOSED ***", "min z:", minz)
    xs = [k[0] for k in occupied]; ys = [k[1] for k in occupied]
    print(f"bbox x {min(xs)}..{max(xs)} y {min(ys)}..{max(ys)}  tiles={len(occupied)} pieces={len(prog['pieces'])}")
    for e in errors[:20]:
        print("COLLISION:", e)
    if not errors:
        print("no footprint collisions")
    return closed and not errors

if __name__ == "__main__":
    prog = json.load(open(sys.argv[1]))
    ok = simulate(prog, verbose="-q" not in sys.argv)
    sys.exit(0 if ok else 1)
