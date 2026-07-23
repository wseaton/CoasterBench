"""Track geometry simulator: parses the game's TrackData.cpp tables so closure
arithmetic matches the engine exactly instead of my memory of RCT2."""
import re, json, sys, pathlib

ROOT = pathlib.Path(__file__).resolve().parents[4]
SRC = (ROOT / "src/openrct2/ride/TrackData.cpp").read_text()
ALL = SRC + "\n".join(p.read_text() for p in (ROOT / "src/openrct2/ride/ted").glob("TED.*.h"))

# 1. per-TED block: coordinates + definition
blocks = {}
for m in re.finditer(r"constexpr auto (kTED\w+) = TrackElementDescriptor\{(.*?)\n\s*\};", ALL, re.S):
    name, body = m.group(1), m.group(2)
    c = re.search(r"\.coordinates = \{([^}]*)\}", body)
    d = re.search(r"\.definition = \{([^}]*)\}", body)
    if not c or not d:
        continue
    coords = [int(x.strip()) for x in c.group(1).split(",")]
    parts = [x.strip() for x in d.group(1).split(",")]
    seq = re.search(r"\.sequenceData = \{\s*\d+,\s*\{(.*?)\}\s*\}", body, re.S)
    seqnames = [s.strip() for s in seq.group(1).split(",") if s.strip()] if seq else []
    blocks[name] = dict(coords=coords, pitchEnd=parts[1], pitchStart=parts[2],
                        rollEnd=parts[3], rollStart=parts[4], seqs=seqnames)

# 1b. sequence clearance footprints (x, y, z, clearanceZ), unrotated
SEQ = {}
for m in re.finditer(r"constexpr SequenceDescriptor (k\w+) = \{(.*?)\n\s*\};", ALL, re.S):
    body = m.group(2)
    c = re.search(r"\.clearance = \{([^{}]*)", body)
    if not c:
        continue
    nums = re.findall(r"-?\d+", c.group(1))
    if len(nums) >= 4:
        SEQ[m.group(1)] = tuple(int(n) for n in nums[:4])

# 2. array order -> TrackElemType index
arr = SRC[SRC.index("kTrackElementDescriptors = std::to_array"):]
arr = arr[:arr.index("});")]
order = re.findall(r"^\s*(kTED\w+),", arr, re.M)
TED = {i: blocks[n] for i, n in enumerate(order) if n in blocks}

# 3. catalog names
CAT = {}
for m in re.finditer(r'\("(\w+)", (\d+)\)', (ROOT / "rust/orct2-agent/src/pieces.rs").read_text()):
    CAT[m.group(1)] = int(m.group(2))

DELTA = [(-32, 0), (0, 32), (32, 0), (0, -32)]  # dir 0=-x,1=+y,2=+x,3=-y

def rotate(x, y, rot):
    """CoordsXY::Rotate (Location.hpp): each step maps (x, y) -> (y, -x)."""
    for _ in range(rot & 3):
        x, y = y, -x
    return x, y

def simulate(pieces, start=(60, 60, 2), z0=0, verbose=False):
    x, y = start[0] * 32, start[1] * 32
    z = z0
    d = start[2]
    roll, pitch = "TrackRoll::none", "TrackPitch::none"
    rows = []
    foot = []
    for i, p in enumerate(pieces):
        name = p["t"] if isinstance(p, dict) else p
        tid = CAT.get(name, name if isinstance(name, int) else None)
        if tid is None:
            raise SystemExit(f"unknown piece {name}")
        t = TED[tid]
        if t["rollStart"] != roll:
            raise SystemExit(f"[{i}] {name}: roll mismatch, track is {roll}, piece enters {t['rollStart']}")
        if t["pitchStart"] != pitch:
            raise SystemExit(f"[{i}] {name}: pitch mismatch, track is {pitch}, piece enters {t['pitchStart']}")
        rb, re_, zb, ze, cx, cy = t["coords"]
        # footprint: sequence clearance boxes, rotated into the cursor direction
        oz = z - zb
        for sname in t["seqs"]:
            sc = SEQ.get(sname)
            if sc is None:
                continue
            sx, sy, sz, cz = sc
            rx, ry = rotate(sx, sy, d)
            foot.append(((x + rx) // 32, (y + ry) // 32, oz + sz, oz + sz + max(cz, 16), i, name))
        ox, oy = rotate(cx, cy, d)
        x += ox; y += oy
        z = z - zb + ze
        d = (d + re_ - rb) & 3
        if (re_ & 4) == 0:
            dx, dy = DELTA[d]
            x += dx; y += dy
        roll, pitch = t["rollEnd"], t["pitchEnd"]
        rows.append((i, name, x // 32, y // 32, z, d, roll.split("::")[1], pitch.split("::")[1]))
    if verbose:
        for r in rows:
            print(r)
    return dict(x=x // 32, y=y // 32, z=z, dir=d, roll=roll.split("::")[1], pitch=pitch.split("::")[1], rows=rows, foot=foot)


def collisions(foot):
    by_tile = {}
    out = []
    for tx, ty, z0, z1, i, name in foot:
        for (oz0, oz1, oi, oname) in by_tile.setdefault((tx, ty), []):
            if oi == i or abs(oi - i) == 1:
                continue
            if z0 < oz1 and oz0 < z1:
                out.append((tx, ty, (oi, oname, oz0, oz1), (i, name, z0, z1)))
        by_tile[(tx, ty)].append((z0, z1, i, name))
    return out

if __name__ == "__main__":
    prog = json.loads(pathlib.Path(sys.argv[1]).read_text())
    s = prog["start"]
    r = simulate(prog["pieces"], (s["x"], s["y"], s["dir"]), 0, verbose="-v" in sys.argv)
    print("START", s["x"], s["y"], 0, s["dir"], "none/none")
    print("END  ", r["x"], r["y"], r["z"], r["dir"], r["roll"] + "/" + r["pitch"])
    print("CLOSED" if (r["x"], r["y"], r["z"], r["dir"], r["roll"], r["pitch"]) == (s["x"], s["y"], 0, s["dir"], "none", "none") else "NOT CLOSED")
    # tile occupancy at each step (rough self-intersection check on end tiles)
    tiles = {}
    for i, name, tx, ty, tz, td, ro, pi in r["rows"]:
        tiles.setdefault((tx, ty), []).append((i, tz))
    cs = collisions(r["foot"])
    print(f"COLLISIONS: {len(cs)}")
    for c in cs[:15]:
        print("  ", c)
    xs = [f[0] for f in r["foot"]]; ys = [f[1] for f in r["foot"]]
    print(f"bbox x {min(xs)}..{max(xs)}  y {min(ys)}..{max(ys)}  maxz {max(f[3] for f in r['foot'])}")
