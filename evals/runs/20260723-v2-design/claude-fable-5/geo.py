"""Offline geometry checker for track programs.

Parses the game's own TrackElementDescriptor tables (TrackData.cpp + TED.*.h)
so the cursor math here is identical to RustBridge.cpp's, then walks a program
and reports closure / bank / slope continuity errors before we burn a minute
on a real eval run.
"""

from __future__ import annotations

import glob
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]

PITCH = {
    "none": 0, "up25": 2, "up60": 4, "down25": 6, "down60": 8,
    "up90": 10, "down90": 12,
}
ROLL = {"none": 0, "left": 2, "right": 4, "upsideDown": 15}


def parse_enum_ids() -> dict[str, int]:
    """name -> numeric id from TrackElemType.h."""
    txt = (ROOT / "src/openrct2/ride/ted/TrackElemType.h").read_text()
    body = txt.split("enum class TrackElemType", 1)[1]
    body = body.split("{", 1)[1]
    ids: dict[str, int] = {}
    nxt = 0
    for line in body.splitlines():
        line = line.split("//")[0].strip().rstrip(",")
        if not line or line.startswith("}"):
            if line.startswith("}"):
                break
            continue
        m = re.match(r"^(\w+)\s*(?:=\s*([0-9xXa-fA-F]+))?$", line)
        if not m:
            continue
        if m.group(2):
            nxt = int(m.group(2), 0)
        ids[m.group(1)] = nxt
        nxt += 1
    return ids


@dataclass
class Ted:
    rot_begin: int
    rot_end: int
    z_begin: int
    z_end: int
    x: int
    y: int
    pitch_end: int
    pitch_start: int
    roll_end: int
    roll_start: int


def parse_teds() -> dict[str, Ted]:
    """kTED<Name> blocks -> Ted, keyed by lowerCamel TrackElemType name."""
    sources = [ROOT / "src/openrct2/ride/TrackData.cpp"]
    sources += [Path(p) for p in glob.glob(str(ROOT / "src/openrct2/ride/ted/TED.*.h"))]
    out: dict[str, Ted] = {}
    blk = re.compile(
        r"kTED(\w+)\s*=\s*TrackElementDescriptor\{(.*?)\n    \};", re.S)
    for src in sources:
        txt = src.read_text()
        for name, body in blk.findall(txt):
            mc = re.search(r"\.coordinates = \{([^}]*)\}", body)
            md = re.search(r"\.definition = \{([^}]*)\}", body)
            if not mc or not md:
                continue
            c = [int(v.strip()) for v in mc.group(1).split(",")]
            d = [v.strip() for v in md.group(1).split(",")]
            try:
                pitch_end = PITCH[d[1].split("::")[1]]
                pitch_start = PITCH[d[2].split("::")[1]]
                roll_end = ROLL[d[3].split("::")[1]]
                roll_start = ROLL[d[4].split("::")[1]]
            except (KeyError, IndexError):
                continue  # non-coaster piece (towers, mazes); we never use these
            key = name[0].lower() + name[1:]
            out[key] = Ted(c[0], c[1], c[2], c[3], c[4], c[5],
                           pitch_end, pitch_start, roll_end, roll_start)
    return out


def parse_sequences() -> dict[str, list[tuple[int, int, int]]]:
    """kTED<Name> -> [(x, y, z)] clearance offsets of each sequence tile.

    A 5-tile turn is a single program piece but occupies five tiles; without
    these offsets the collision check only sees the entry tile.
    """
    sources = [ROOT / "src/openrct2/ride/TrackData.cpp"]
    sources += [Path(p) for p in glob.glob(str(ROOT / "src/openrct2/ride/ted/TED.*.h"))]
    seqs: dict[str, tuple[int, int, int]] = {}
    teds: dict[str, list[str]] = {}
    seq_re = re.compile(
        r"SequenceDescriptor\s+(k\w+)\s*=\s*\{\s*\.clearance = \{\s*(-?\d+),\s*(-?\d+),\s*(-?\d+)")
    ted_re = re.compile(r"kTED(\w+)\s*=\s*TrackElementDescriptor\{(.*?)\n    \};", re.S)
    for src in sources:
        txt = src.read_text()
        for name, x, y, z in seq_re.findall(txt):
            seqs[name] = (int(x), int(y), int(z))
        for name, body in ted_re.findall(txt):
            m = re.search(r"\.sequenceData = \{\s*\d+,\s*\{(.*?)\}\s*\}", body, re.S)
            if m:
                teds[name[0].lower() + name[1:]] = re.findall(r"k\w+", m.group(1))
    return {k: [seqs[s] for s in v if s in seqs] for k, v in teds.items()}


def parse_catalog() -> dict[str, int]:
    txt = (ROOT / "rust/orct2-agent/src/pieces.rs").read_text()
    return {n: int(i) for n, i in re.findall(r'\("(\w+)", (\d+)\)', txt)}


NAMES = parse_enum_ids()
TEDS = parse_teds()
SEQS = parse_sequences()
CATALOG = parse_catalog()
ID_TO_NAME = {v: k for k, v in NAMES.items()}
# Highest-priority duplicate wins; enum has aliases, keep the first spelling.
for k, v in parse_enum_ids().items():
    ID_TO_NAME.setdefault(v, k)

DIR_DELTA = [(-32, 0), (0, 32), (32, 0), (0, -32)]


def rotate(x: int, y: int, rot: int) -> tuple[int, int]:
    """Mirror of CoordsXY::Rotate (Location.hpp): one step is (x, y) -> (y, -x)."""
    for _ in range(rot & 3):
        x, y = y, -x
    return x, y


@dataclass
class Cursor:
    x: int
    y: int
    z: int
    d: int
    bank: int = 0
    slope: int = 0


def ted_for(piece: str | int) -> tuple[int, Ted]:
    tid = CATALOG[piece] if isinstance(piece, str) else int(piece)
    name = ID_TO_NAME[tid]
    return tid, TEDS[name]


def simulate(program: dict, verbose: bool = False):
    start = program["start"]
    cur = Cursor(start["x"] * 32, start["y"] * 32, start.get("z", 0) * 8, start["dir"] & 3)
    tiles: dict[tuple[int, int], list] = {}
    errs = []
    for i, p in enumerate(program["pieces"]):
        name = p["t"] if isinstance(p, dict) else p
        chain = isinstance(p, dict) and p.get("chain", False)
        tid, t = ted_for(name)
        if t.roll_start != cur.bank:
            errs.append(f"[{i}] {name}: enters roll {t.roll_start} but track is {cur.bank}")
            return errs, cur, tiles
        if t.pitch_start != cur.slope:
            errs.append(f"[{i}] {name}: enters pitch {t.pitch_start} but track is {cur.slope}")
            return errs, cur, tiles
        # Every sequence tile of the piece, at its own height, so multi-tile
        # turns and hops are collision-checked properly.
        base = cur.z - t.z_begin
        for sx, sy, sz in SEQS.get(ID_TO_NAME[tid], [(0, 0, 0)]):
            rx, ry = rotate(sx, sy, cur.d)
            tiles.setdefault(((cur.x + rx) // 32, (cur.y + ry) // 32), []).append(
                (i, name, base + sz))
        ox, oy = rotate(t.x, t.y, cur.d)
        nx, ny = cur.x + ox, cur.y + oy
        nz = cur.z - t.z_begin + t.z_end
        nd = (cur.d + t.rot_end - t.rot_begin) & 3
        if (t.rot_end & 4) == 0:
            dx, dy = DIR_DELTA[nd]
            nx, ny = nx + dx, ny + dy
        cur = Cursor(nx, ny, nz, nd, t.roll_end, t.pitch_end)
        if verbose:
            print(f"  [{i:3}] {name:<24} chain={int(chain)} -> "
                  f"({cur.x//32},{cur.y//32}) z={cur.z} d={cur.d} b={cur.bank} s={cur.slope}")
    return errs, cur, tiles


def check(path: str, verbose: bool = False) -> bool:
    program = json.loads(Path(path).read_text())
    start = program["start"]
    errs, cur, tiles = simulate(program, verbose)
    ok = True
    for e in errs:
        print("ERR", e)
        ok = False
    sx, sy, sz, sd = start["x"] * 32, start["y"] * 32, start.get("z", 0) * 8, start["dir"] & 3
    if (cur.x, cur.y, cur.z, cur.d) != (sx, sy, sz, sd):
        print(f"ERR not closed: end ({cur.x//32},{cur.y//32},z={cur.z},d={cur.d}) "
              f"vs start ({sx//32},{sy//32},z={sz},d={sd})")
        ok = False
    if cur.bank or cur.slope:
        print(f"ERR ends banked/sloped: bank={cur.bank} slope={cur.slope}")
        ok = False
    minz = min((z for v in tiles.values() for _, _, z in v), default=0)
    if minz < sz:
        print(f"ERR dips below ground: min z={minz} start z={sz}")
        ok = False
    # Multi-tile pieces occupy more than the origin tile; this only catches
    # origin-tile collisions, which is still most of them.
    for tile, occ in sorted(tiles.items()):
        if len(occ) > 1:
            zs = [z for _, _, z in occ]
            if max(zs) - min(zs) < 24:
                print(f"WARN tile {tile} reused at z {zs}: {[n for _, n, _ in occ]}")
    xs = [t[0] for t in tiles]
    ys = [t[1] for t in tiles]
    print(f"pieces={len(program['pieces'])} bbox x[{min(xs)},{max(xs)}] y[{min(ys)},{max(ys)}] "
          f"maxz={max(z for v in tiles.values() for _, _, z in v)} tiles={len(tiles)}")
    print("OK" if ok else "FAIL")
    return ok


if __name__ == "__main__":
    v = "-v" in sys.argv
    args = [a for a in sys.argv[1:] if a != "-v"]
    sys.exit(0 if check(args[0], v) else 1)
