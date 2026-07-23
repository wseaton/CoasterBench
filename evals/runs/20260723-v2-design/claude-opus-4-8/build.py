"""Build + offline-verify a track program, then write it to round_N/program.json."""
import json
import sys
from itertools import product
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from geom import simulate

HERE = Path(__file__).parent


def C(name):
    return {"t": name, "chain": True}


def lift25(n):
    """Chained 25 climb: rise = 16 + 16n over n+2 tiles."""
    return [C("flat_to_up_25")] + [C("up_25")] * n + [C("up_25_to_flat")]


def lift60(n):
    """Chained steep climb: rise = 80 + 64n over n+4 tiles."""
    return ([C("flat_to_up_25"), C("up_25_to_up_60")] + [C("up_60")] * n
            + [C("up_60_to_up_25"), C("up_25_to_flat")])


def hill(n=1):
    """Airtime hump, net zero, height 16+16n, over 2n+4 tiles."""
    return (["flat_to_up_25"] + ["up_25"] * n + ["up_25_to_flat"]
            + ["flat_to_down_25"] + ["down_25"] * n + ["down_25_to_flat"])


def steep_hill(n=0):
    """Steep airtime hump: 60-up crest then 60-down. Height 80+64n."""
    return (["flat_to_up_25", "up_25_to_up_60"] + ["up_60"] * n
            + ["up_60_to_up_25", "up_25_to_flat"]
            + ["flat_to_down_25", "down_25_to_down_60"] + ["down_60"] * n
            + ["down_60_to_down_25", "down_25_to_flat"])


def drop(pre=0, steep=1, post=0):
    """Big drop: fall = 80 + 16*(pre+post) + 64*steep over 5+pre+post+steep tiles."""
    return (["flat_to_down_25"] + ["down_25"] * pre + ["down_25_to_down_60"]
            + ["down_60"] * steep + ["down_60_to_down_25"] + ["down_25"] * post
            + ["down_25_to_flat"])


def spike(n=0):
    """Steep airtime hump: 60-up, sharp crest, 60-down. Height 48+64n over 4+2n tiles.
    Both crest transitions have verticalFactor -56, the sharpest in the catalog."""
    return (["flat_to_up_60"] + ["up_60"] * n + ["up_60_to_flat"]
            + ["flat_to_down_60"] + ["down_60"] * n + ["down_60_to_flat"])


def softspike():
    """Airtime hump with the sharpest legal crest (up_60_to_flat + flat_to_down_60,
    verticalFactor -56 both) but eased valleys, so negative G stays high while
    positive G does not blow the intensity budget. +/-64 over 6 tiles."""
    return ["flat_to_up_25", "up_25_to_up_60", "up_60_to_flat",
            "flat_to_down_60", "down_60_to_down_25", "down_25_to_flat"]


def sharpspike():
    """Compact 48-unit airtime spike: 60 up, sharp crest, 60 down, in 4 tiles.
    Sharp valleys too, so keep these for the slower back half of the course."""
    return ["flat_to_up_60", "up_60_to_flat", "flat_to_down_60", "down_60_to_flat"]


def sloped_turn(side, updown, size):
    """90 turn while climbing/descending 25. Sloped turns add excitement, zero intensity."""
    ud = "up" if updown == "up" else "down"
    return [f"flat_to_{ud}_25", f"{side}_turn_{size}_{ud}_25", f"{ud}_25_to_flat"]


def slalom(size=3):
    """Four sloped turns forming a closed slalom: net zero rotation, height and (near) offset."""
    return (sloped_turn("right", "up", size) + sloped_turn("left", "down", size)
            + sloped_turn("left", "up", size) + sloped_turn("right", "down", size))


def bank5(side, n=1):
    return [f"flat_to_{side}_bank"] + [f"banked_{side}_turn_5"] * n + [f"{side}_bank_to_flat"]


def bank3(side, n=1):
    return [f"flat_to_{side}_bank"] + [f"banked_{side}_turn_3"] * n + [f"{side}_bank_to_flat"]


def helix(side, updown, size, n=1):
    return ([f"flat_to_{side}_bank"] + [f"{side}_helix_{updown}_{size}"] * n
            + [f"{side}_bank_to_flat"])


def dip():
    """-16z over 2 tiles."""
    return ["flat_to_down_25", "down_25_to_flat"]


def rise():
    return ["flat_to_up_25", "up_25_to_flat"]


START = {"x": 20, "y": 48, "dir": 2}


def build(a=0, b=0, c=0, d=0):
    p = []
    p += ["begin_station", "middle_station", "middle_station", "end_station"]
    p += lift25(13)                           # z 0 -> 224 over 15 tiles
    p += ["flat"] * (1 + a)
    p += drop(0, 2, 0)                        # -208 -> z 16, 26-unit first drop
    p += sharpspike()                         # fastest point: sets max negative G
    p += softspike()
    p += ["s_bend_right", "s_bend_left"]
    p += softspike()
    p += helix("right", "down", "large")      # 180 turnaround, -16 -> z 0
    # Back half hugs the ground: fast, cheap proximity score, plenty of drops.
    p += hill(1)
    p += bank5("left") + bank5("right")
    p += sharpspike()
    p += hill(1)
    p += bank5("right") + bank5("left")
    p += hill(1)
    p += sharpspike()
    for _ in range(d):
        p += hill(1)
    p += (["s_bend_left"] * b if b >= 0 else ["s_bend_right"] * -b)
    p += ["flat"] * c + ["brakes", "brakes"]
    p += bank5("right")                       # dir 0 -> 1
    p += bank5("right")                       # dir 1 -> 2
    return p


def solve():
    best = None
    for a, b, c, d in product(range(0, 40), range(-9, 10), range(0, 10), range(0, 5)):
        prog = {"ride_type": 52, "start": START, "pieces": build(a, b, c, d)}
        errs, info = simulate(prog)
        if not errs:
            return (a, b, c, d)
        if info and not any("CLASH" in e or "ground" in e for e in errs):
            e = info["end"]
            dd = abs(e["x"] - START["x"]) + abs(e["y"] - START["y"]) + abs(e["z"]) // 8 + (e["dir"] != START["dir"]) * 10
            if best is None or dd < best[0]:
                best = (dd, (a, b, c, d), e)
    print("no exact solution; closest:", best)
    return None


if __name__ == "__main__":
    if "--solve" in sys.argv:
        print(solve())
        sys.exit()
    params = tuple(int(x) for x in sys.argv[2:6]) if len(sys.argv) > 5 else ()
    prog = {"ride_type": 52, "start": START, "pieces": build(*params)}
    errs, info = simulate(prog)
    print("pieces:", len(prog["pieces"]))
    if info:
        print("start:", START, "\nend:  ", info["end"])
    for e in errs:
        print("ERR", e)
    if not errs and len(sys.argv) > 1 and sys.argv[1].isdigit():
        out = HERE / f"round_{sys.argv[1]}"
        out.mkdir(exist_ok=True)
        (out / "program.json").write_text(json.dumps(prog, indent=2))
        print("wrote", out / "program.json")
