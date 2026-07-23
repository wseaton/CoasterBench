"""Brute-force the free straight/turn lengths until the circuit closes cleanly.

Closure is 4 constraints (x, y, z, dir) and the knobs are linear in x/y, so a
small grid search beats hand algebra and also screens out self-collisions.
"""
import itertools
import sys

import geom


def ok_bbox(foot):
    for tx, ty, *_ in foot:
        if not (26 <= tx <= 66 and 32 <= ty <= 82):
            return False
        if 55 <= ty <= 75 and tx >= 67:      # the lake
            return False
    return True


def search(mod, knobs, start=(58, 62, 0)):
    names = list(knobs)
    hits = []
    for combo in itertools.product(*(knobs[k] for k in names)):
        for k, v in zip(names, combo):
            setattr(mod, k, v)
        pieces = mod.build()
        try:
            r = geom.simulate(pieces, start, 0)
        except SystemExit:
            continue
        if (r["x"], r["y"], r["z"], r["dir"], r["roll"], r["pitch"]) != (
                start[0], start[1], 0, start[2], "none", "none"):
            continue
        if not ok_bbox(r["foot"]):
            continue
        cs = geom.collisions(r["foot"])
        if cs:
            continue
        hits.append((len(pieces), dict(zip(names, combo))))
    return hits


if __name__ == "__main__":
    mod = __import__(sys.argv[1])
    knobs = eval(sys.argv[2])
    hits = search(mod, knobs)
    print(f"{len(hits)} clean solutions")
    for n, k in sorted(hits, key=lambda h: -h[0])[:10]:
        print(" ", n, k)
