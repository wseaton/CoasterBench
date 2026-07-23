import variants2 as V, geom, json, pathlib
from build import chain


def spiral(nrev):
    seq = ["flat_to_up_25"]
    for i in range(nrev):
        t = "left" if i % 2 == 0 else "right"
        seq += [f"{t}_turn_3_up_25"] * 4
    return chain(seq + ["up_25_to_flat"])


def solve2(name, n60, n25, nrev, PB=None, PC=None, PD=None, quiet=False):
    PB = PB or V.PB; PC = PC or V.PC; PD = PD or V.PD
    for A1 in range(0, 14):
      for E1 in range(0, 8):
        for C1 in (8, 6, 10, 4, 12):
          for B1 in range(0, 12):
            for CORR in range(18, 34):
                V.B1, V.E1, V.C1 = B1, E1, C1
                p = V.make(0, n60, n25, A1, CORR, PB, PC, PD, lambda: spiral(nrev))
                r = geom.simulate(p, (58, 62, 0), 0)
                if (r["x"], r["y"], r["z"], r["dir"], r["roll"], r["pitch"]) != (58, 62, 0, 0, "none", "none"):
                    continue
                xs = [f[0] for f in r["foot"]]; ys = [f[1] for f in r["foot"]]
                if geom.collisions(r["foot"]) or max(xs) > 71 or min(xs) < 21 or max(ys) > 86:
                    continue
                if any(54 <= f[1] <= 76 and f[0] >= 67 for f in r["foot"]):
                    continue
                if not quiet:
                    print(f"{name}: A1={A1} B1={B1} E1={E1} CORR={CORR} n={len(p)} "
                          f"bbox={(min(xs), max(xs), min(ys), max(ys))} maxz={max(f[3] for f in r['foot'])}")
                pathlib.Path(f"/tmp/{name}.json").write_text(
                    json.dumps({"ride_type": 52, "start": {"x": 58, "y": 62, "dir": 0}, "pieces": p}))
                V.B1, V.E1, V.C1 = 0, 4, 8
                return p
    print(name, "no solution")
    V.B1, V.E1, V.C1 = 0, 4, 8
