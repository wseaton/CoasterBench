"""Round 3 exploration: lift height and first-drop pullout shape."""
import geom, json, pathlib
from build import hill, turn180, turn90, chain

MID1 = MID2 = 6
MID3 = 12
SHELF = 3
B1, C1, E1 = 0, 8, 4


def spiral_lift(n_flat):
    return chain(["flat_to_up_25"] + ["left_turn_3_up_25"] * 4
                 + ["right_turn_3_up_25"] * 4 + ["up_25"] * n_flat + ["up_25_to_flat"])


def steep_spiral_lift(n60):
    """Twin spiral (two 4+ sloped turn runs) then a 60-degree chain climb:
    336 + 64*n60 of height in 12 + n60 tiles."""
    return chain(["flat_to_up_25"] + ["left_turn_3_up_25"] * 4
                 + ["right_turn_3_up_25"] * 4 + ["up_25_to_up_60"] + ["up_60"] * n60
                 + ["up_60_to_up_25", "up_25_to_flat"])


def drop(n60, n25_out):
    return (["flat_to_down_25", "down_25_to_down_60"] + ["down_60"] * n60
            + ["down_60_to_down_25"] + ["down_25"] * n25_out + ["down_25_to_flat"])


def climb(n):
    return ["flat_to_up_25"] + ["up_25"] * n + ["up_25_to_flat"]


def dive(n):
    return ["flat_to_down_25"] + ["down_25"] * n + ["down_25_to_flat"]


def bank_s(first, n=5):
    a = "left" if first == "l" else "right"
    b = "right" if first == "l" else "left"
    return [f"flat_to_{a}_bank", f"banked_{a}_turn_{n}", f"{a}_bank_to_flat",
            f"flat_to_{b}_bank", f"banked_{b}_turn_{n}", f"{b}_bank_to_flat"]


def slalom(first="l", n=5):
    return bank_s(first, n) + bank_s("r" if first == "l" else "l", n)


STATION = 4


def make(n_flat, n60, n25_out, A1, CORR, pass_b, pass_c, pass_d, lift=None):
    p = []
    p += ["begin_station"] + ["middle_station"] * (STATION - 2) + ["end_station"]
    p += (lift() if lift else spiral_lift(n_flat))
    p += drop(n60, n25_out)
    p += ["flat"] * A1
    p += climb(SHELF)
    p += turn180("l", banked=True, mid=MID1)
    p += dive(SHELF); p += pass_b(); p += ["flat"] * B1
    p += climb(SHELF)
    p += turn180("r", banked=True, mid=MID2)
    p += dive(SHELF); p += pass_c(); p += ["flat"] * C1
    p += climb(SHELF)
    p += turn90("r", banked=True)
    p += dive(SHELF); p += ["flat"] * CORR
    p += turn90("r", banked=True)
    p += pass_d(); p += ["flat"] * E1
    p += turn180("r", banked=True, mid=MID3)
    return p


def solve(name, n_flat, n60, n25_out, pass_b, pass_c, pass_d, lift=None):
    for A1 in range(0, 18):
        for CORR in range(18, 34):
            p = make(n_flat, n60, n25_out, A1, CORR, pass_b, pass_c, pass_d, lift)
            r = geom.simulate(p, (58, 62, 0), 0)
            if (r["x"], r["y"], r["z"], r["dir"], r["roll"], r["pitch"]) != (58, 62, 0, 0, "none", "none"):
                continue
            xs = [f[0] for f in r["foot"]]
            ys = [f[1] for f in r["foot"]]
            if geom.collisions(r["foot"]) or max(xs) > 71 or min(xs) < 26 or max(ys) > 82:
                continue
            if any(54 <= f[1] <= 76 and f[0] >= 67 for f in r["foot"]):
                continue
            print(f"{name}: A1={A1} CORR={CORR} n={len(p)} bbox={(min(xs), max(xs), min(ys), max(ys))} "
                  f"maxz={max(f[3] for f in r['foot'])}")
            pathlib.Path(f"/tmp/{name}.json").write_text(
                json.dumps({"ride_type": 52, "start": {"x": 58, "y": 62, "dir": 0}, "pieces": p}))
            return p
    print(name, "no solution")


PB = lambda: hill(3, 3) + hill(2, 2)
PC = lambda: hill(2, 2) + hill(2, 2)
PD = lambda: hill(2, 2) + slalom("r")

if __name__ == "__main__":
    solve("x0", 1, 3, 1, PB, PC, PD)                  # baseline v1, lift 288
    solve("x1", 7, 3, 1, PB, PC, PD)                  # lift 384
    solve("x2", 7, 4, 2, PB, PC, PD)                  # lift 384, deeper+softer drop
    solve("x3", 13, 4, 2, PB, PC, PD)                 # lift 480
