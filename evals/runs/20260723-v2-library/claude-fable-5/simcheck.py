"""Offline port of rust/orct2-agent/src/similarity.rs so I can check the
plagiarism score before burning an eval run."""
import json
import pathlib
import re
import sys

import geom

LIB = json.loads((pathlib.Path(__file__).resolve().parents[0] / ".." / "library.json").read_text())

# mirrorElement per TED, resolved to numeric ids through the array order
NAME2ID = {}
for i, n in enumerate(geom.order):
    NAME2ID.setdefault(n, i)
MIRROR = {}
for i, n in enumerate(geom.order):
    m = re.search(r"\.mirrorElement = TrackElemType::(\w+)", geom.ALL[geom.ALL.find(f"constexpr auto {n} = TrackElementDescriptor"):][:4000])
    if m:
        key = "kTED" + m.group(1)[0].upper() + m.group(1)[1:]
        MIRROR[i] = NAME2ID.get(key, i)
    else:
        MIRROR[i] = i


def ids(pieces):
    out = []
    for p in pieces:
        name = p["t"] if isinstance(p, dict) else p
        out.append(name if isinstance(name, int) else geom.CAT[name])
    return out


def lev(a, b):
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a):
        cur = [i + 1] + [0] * len(b)
        for j, cb in enumerate(b):
            cur[j + 1] = min(prev[j] + (ca != cb), prev[j + 1] + 1, cur[j] + 1)
        prev = cur
    return prev[len(b)]


def substring_share(prog, other):
    prev = [0] * (len(other) + 1)
    longest = 0
    for cp in prog:
        cur = [0] * (len(other) + 1)
        for j, co in enumerate(other):
            cur[j + 1] = prev[j] + 1 if cp == co else 0
            longest = max(longest, cur[j + 1])
        prev = cur
    return longest / len(prog)


def best(prog):
    out = None
    for d in LIB:
        pieces = d["pieces"]
        pieces = [p if isinstance(p, int) else geom.CAT.get(p, -1) for p in pieces]
        mirrored = [MIRROR.get(p, p) for p in pieces]
        for variant, is_m in ((pieces, False), (mirrored, True)):
            if is_m and variant == pieces:
                continue
            e = 1 - lev(prog, variant) / max(len(prog), len(variant))
            s = substring_share(prog, variant)
            c = max(e, s)
            if out is None or c > out[0]:
                out = (c, d["name"], e, s, is_m)
    return out


if __name__ == "__main__":
    mod = __import__(sys.argv[1].replace(".py", ""))
    prog = ids(mod.build())
    c, name, e, s, m = best(prog)
    print(f"similarity={c:.3f} nearest={name!r} edit={e:.3f} substring={s:.3f} mirrored={m}")
