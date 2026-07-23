#!/usr/bin/env python3
"""Run layout variants through the real game and print the ratings table.

An eval is under a second, so the design is tuned by measurement rather than by
guessing at the rating formula.
"""
import json
import os
import subprocess
import sys

sys.path.insert(0, __file__.rsplit('/', 1)[0])
import layout  # noqa: E402

HOME = os.environ["HOME"]
SCENARIO = f"{HOME}/rct2-assets/Scenarios/Build your own Six Flags Park.SC6"
CLI = "./build/openrct2-cli"


def run(prog_path, report_path, ticks=25000, capture=None):
    cmd = [CLI, "eval", SCENARIO, "--ticks", str(ticks), "--rct2-data-path",
           f"{HOME}/rct2-assets", "--program", prog_path, "--out", report_path]
    if capture:
        cmd += ["--capture", capture]
    subprocess.run(cmd, capture_output=True, text=True, cwd=os.getcwd())
    return json.load(open(report_path))


def score(report):
    sim = (report.get("similarity") or {}).get("similarity", 0.0)
    mult = 1.0 if sim <= 0.5 else max(0.0, (1.0 - sim) / 0.5)
    ride = report["rides"][0] if report["rides"] else {}
    exc = ride.get("excitement") or 0.0
    return exc * mult, sim, ride


def evaluate(name, params, build=layout.build_track, keep=None):
    t = build(params)
    prog = f"/tmp/{name}.json"
    t.dump(prog, layout.START)
    rep = run(prog, keep + "/report.json" if keep else f"/tmp/{name}.report",
              capture=keep + "/park.png" if keep else None)
    if keep:
        os.replace(prog, keep + "/program.json")
    penalised, sim, ride = score(rep)
    err = (rep["program"].get("error") or {}).get("message", "")
    print(f"{name:16s} n={len(t.pieces):3d} exc={ride.get('excitement')} int={ride.get('intensity')} "
          f"nau={ride.get('nausea')} sim={sim:.2f} -> {penalised:.2f} | "
          f"drops={ride.get('num_drops')} hi={ride.get('highest_drop')} "
          f"len={(ride.get('ride_length') or 0) // 65536} spd={(ride.get('max_speed') or 0) >> 16}/"
          f"{(ride.get('average_speed') or 0) >> 16} "
          f"g={ride.get('max_positive_g')}/{ride.get('max_negative_g')}/{ride.get('max_lateral_g')} "
          f"air={ride.get('total_air_time')} {err[:80]}")
    return penalised, rep
