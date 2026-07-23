#!/usr/bin/env python3
"""Track builder DSL. Emits a program.json and reports the cursor as it goes.

The cursor math mirrors sim.py (which mirrors RustBridge.cpp), so a layout can
be composed with high-level moves ("run until y == 40") instead of hand-counted
piece lists.
"""
import json
import sys

sys.path.insert(0, __file__.rsplit('/', 1)[0])
from sim import TED, CAT, DELTA, rot, simulate  # noqa: E402


class Track:
    def __init__(self, x, y, d):
        self.x, self.y, self.d, self.z = x * 32, y * 32, d & 3, 0
        self.pieces = []

    def add(self, name, chain=False, n=1):
        for _ in range(n):
            tid = CAT[name] if isinstance(name, str) else name
            rb, re_, zb, ze, cx, cy = TED[tid]["coords"]
            ox, oy = rot(cx, cy, self.d)
            nx, ny = self.x + ox, self.y + oy
            nz = self.z - zb + ze
            nd = (self.d + re_ - rb) & 3
            if (re_ & 4) == 0:
                nx += DELTA[nd][0]
                ny += DELTA[nd][1]
            self.x, self.y, self.z, self.d = nx, ny, nz, nd
            self.pieces.append({"t": name, "chain": True} if chain else name)
        return self

    def run_to(self, axis, value, name="flat", chain=False):
        """Repeat a straight piece until the given tile axis reaches value."""
        guard = 0
        sign = {0: (-1, 0), 1: (0, 1), 2: (1, 0), 3: (0, -1)}[self.d]["xy".index(axis)]
        cur = getattr(self, "tile_" + axis)
        if sign * (value - cur) < 0:
            raise RuntimeError(f"run_to {axis}={value} is behind the cursor {self.at}")
        while getattr(self, "tile_" + axis) != value:
            self.add(name, chain)
            guard += 1
            if guard > 200:
                raise RuntimeError(f"run_to {axis}={value} overshot from {self.at}")
        return self

    @property
    def tile_x(self):
        return self.x // 32

    @property
    def tile_y(self):
        return self.y // 32

    @property
    def at(self):
        return f"({self.tile_x},{self.tile_y}) z={self.z} dir={self.d}"

    def dump(self, path, start):
        prog = {"ride_type": 52, "start": {"x": start[0], "y": start[1], "dir": start[2]},
                "pieces": self.pieces}
        json.dump(prog, open(path, "w"), indent=1)
        return prog
