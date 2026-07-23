"""Offline geometry checker mirroring RustBridge orct2_host_track_place cursor math."""
import json, re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
SRC = (ROOT / "src/openrct2/ride/TrackData.cpp").read_text()
for h in sorted((ROOT / "src/openrct2/ride/ted").glob("TED.*.h")):
    SRC += h.read_text()

# order of kTED* names in kTrackElementDescriptors = TrackElemType index order
arr = SRC.split("kTrackElementDescriptors = std::to_array<TrackElementDescriptor>({", 1)[1]
arr = arr.split("});", 1)[0]
order = [m.strip().rstrip(",") for m in arr.strip().splitlines()]
order = [o for o in order if o.startswith("kTED")]

PITCH = {"none":0,"up25":2,"up60":4,"down25":6,"down60":8}
ROLL = {"none":0,"left":2,"right":4,"upsideDown":15}

blocks = {}
for m in re.finditer(r"constexpr auto (kTED\w+) = TrackElementDescriptor\{(.*?)\n\s*\};", SRC, re.S):
    blocks[m.group(1)] = m.group(2)

def parse(name):
    b = blocks[name]
    c = re.search(r"\.coordinates = \{([^}]*)\}", b)
    coords = [int(x.strip()) for x in c.group(1).split(",")]
    d = re.search(r"\.definition = \{([^}]*)\}", b, re.S)
    parts = [p.strip() for p in d.group(1).split(",")]
    pitchEnd = PITCH.get(parts[1].split("::")[1], -1)
    pitchStart = PITCH.get(parts[2].split("::")[1], -1)
    rollEnd = ROLL.get(parts[3].split("::")[1], -1)
    rollStart = ROLL.get(parts[4].split("::")[1], -1)
    return dict(rotB=coords[0], rotE=coords[1], zB=coords[2], zE=coords[3], x=coords[4], y=coords[5],
                pitchStart=pitchStart, pitchEnd=pitchEnd, rollStart=rollStart, rollEnd=rollEnd)

SEQ = {}
for m in re.finditer(r"constexpr\s+SequenceDescriptor (\w+) = \{\s*\.clearance = \{([^}]*)\}", SRC):
    vals = [v.strip() for v in m.group(2).split(",")]
    def num(v):
        try: return int(v)
        except: return 0
    SEQ[m.group(1)] = (num(vals[0]), num(vals[1]), num(vals[2]), num(vals[3]) if len(vals) > 3 else 0)

def seqs_of(name):
    b = blocks[name]
    m = re.search(r"\.sequenceData = \{\s*(\d+),\s*\{(.*?)\}\s*\}", b, re.S)
    if not m:
        return [(0, 0, 0, 0)]
    names = [x.strip() for x in m.group(2).split(",") if x.strip()]
    out = []
    for n in names:
        if n in SEQ:
            out.append(SEQ[n])
        else:
            out.append((0, 0, 0, 0))
    return out or [(0, 0, 0, 0)]

TED = {i: parse(n) for i, n in enumerate(order)}
for i, n in enumerate(order):
    TED[i]["seqs"] = seqs_of(n)

CATALOG = {}
for m in re.finditer(r'\("(\w+)", (\d+)\)', (ROOT/"rust/orct2-agent/src/pieces.rs").read_text()):
    CATALOG[m.group(1)] = int(m.group(2))

DELTA = {0: (-32, 0), 1: (0, 32), 2: (32, 0), 3: (0, -32)}

def rotate(x, y, rot):
    for _ in range(rot):
        x, y = y, -x  # CoordsXY::Rotate direction 1
    return x, y

def simulate(prog, verbose=True):
    st = prog["start"]
    x, y, z, d = st["x"]*32, st["y"]*32, 0, st["dir"] & 3
    bank = slope = 0
    tiles = {}
    errs = []
    for i, p in enumerate(prog["pieces"]):
        if isinstance(p, dict):
            t = p["t"]; chain = p.get("chain", False)
        else:
            t = p; chain = False
        tid = CATALOG[t] if isinstance(t, str) else t
        c = TED[tid]
        if c["rollStart"] != bank:
            errs.append(f"[{i}] {t}: enters roll {c['rollStart']} but track is {bank}")
            return errs, None
        if c["pitchStart"] != slope:
            errs.append(f"[{i}] {t}: enters pitch {c['pitchStart']} but track is {slope}")
            return errs, None
        origin_z = z - c["zB"]
        for (sx_, sy_, sz_, cz_) in c["seqs"]:
            rx, ry = rotate(sx_, sy_, d)
            tiles.setdefault(((x+rx)//32, (y+ry)//32), []).append((i, t, origin_z + sz_))
        ox, oy = rotate(c["x"], c["y"], d)
        nz = z - c["zB"] + c["zE"]
        nd = (d + c["rotE"] - c["rotB"]) & 3
        nx, ny = x+ox, y+oy
        if (c["rotE"] & 4) == 0:
            dx, dy = DELTA[nd]
            nx += dx; ny += dy
        x, y, z, d = nx, ny, nz, nd
        bank, slope = c["rollEnd"], c["pitchEnd"]
        if z < 0:
            errs.append(f"[{i}] {t}: z below ground ({z})")
    end = dict(x=x//32, y=y//32, z=z, dir=d, bank=bank, slope=slope)
    start = dict(x=st["x"], y=st["y"], z=0, dir=st["dir"] & 3, bank=0, slope=0)
    if end != start:
        errs.append(f"NOT CLOSED: start={start} end={end}")
    for k, v in tiles.items():
        for a in range(len(v)):
            for b in range(a+1, len(v)):
                if v[a][0] == v[b][0]:
                    continue
                if abs(v[a][2] - v[b][2]) < 32:
                    errs.append(f"TILE CLASH at {k}: piece {v[a]} vs {v[b]}")
    return errs, dict(end=end, tiles=tiles)

if __name__ == "__main__":
    prog = json.loads(Path(sys.argv[1]).read_text())
    errs, info = simulate(prog)
    if info: print("end:", info["end"], " pieces:", len(prog["pieces"]))
    for e in errs: print("ERR", e)
    if not errs: print("OK: closed, no clashes")
