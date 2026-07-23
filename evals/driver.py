# /// script
# requires-python = ">=3.11"
# dependencies = ["anthropic[vertex]>=0.40"]
# ///
"""Coaster design head-to-head: two Claude models iteratively design a coaster.

Each round the model submits a JSON track program (via forced tool use); the
harness runs `openrct2-cli eval` on a fresh copy of the scenario, then feeds
back the eval report and a park screenshot. Best excitement across rounds wins.

Usage (first-party API):
  ANTHROPIC_API_KEY=... uv run evals/driver.py \
      --models claude-fable-5 claude-sonnet-5 --rounds 4

Usage (Google Vertex AI; auth via `gcloud auth application-default login`):
  uv run evals/driver.py --vertex --project my-gcp-project \
      --models claude-opus-4-6 claude-sonnet-5 --rounds 4

Vertex model IDs for current-generation models are the bare first-party
strings (claude-opus-4-6, claude-sonnet-5) — no prefix, no @date suffix.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

import anthropic

REPO = Path(__file__).resolve().parent.parent
CLI = REPO / "build" / "openrct2-cli"
DEFAULT_SCENARIO = Path.home() / "rct2-assets" / "Scenarios" / "Build your own Six Flags Park.SC6"
RCT2_DATA = Path.home() / "rct2-assets"

PIECE_CATALOG = """
Station (required, place these FIRST, 3+ in a row): begin_station, middle_station, end_station
Straight & slopes: flat, up_25, up_60, down_25, down_60
Slope transitions: flat_to_up_25, up_25_to_up_60, up_60_to_up_25, up_25_to_flat,
  flat_to_down_25, down_25_to_down_60, down_60_to_down_25, down_25_to_flat,
  flat_to_up_60, up_60_to_flat, flat_to_down_60, down_60_to_flat
Turns (90 degrees, radius in tiles): left_turn_5, right_turn_5, left_turn_3, right_turn_3, left_turn_1, right_turn_1
Sloped turns: left_turn_5_up_25, right_turn_5_up_25, left_turn_5_down_25, right_turn_5_down_25,
  left_turn_3_up_25, right_turn_3_up_25, left_turn_3_down_25, right_turn_3_down_25
Banking: flat_to_left_bank, flat_to_right_bank, left_bank_to_flat, right_bank_to_flat,
  left_bank, right_bank, banked_left_turn_5, banked_right_turn_5, banked_left_turn_3, banked_right_turn_3,
  left_bank_to_up_25, right_bank_to_up_25, up_25_to_left_bank, up_25_to_right_bank,
  left_bank_to_down_25, right_bank_to_down_25, down_25_to_left_bank, down_25_to_right_bank
S-bends: s_bend_left, s_bend_right
Inversions (wooden coaster does NOT support these; steel types do): left_vertical_loop, right_vertical_loop,
  half_loop_up, half_loop_down, left_corkscrew_up, right_corkscrew_up, left_corkscrew_down, right_corkscrew_down
Helices: left_helix_up_small, right_helix_up_small, left_helix_down_small, right_helix_down_small,
  left_helix_up_large, right_helix_up_large, left_helix_down_large, right_helix_down_large
Special: brakes, booster
"""

SYSTEM_PROMPT = f"""You are competing to design the best RollerCoaster Tycoon 2 roller coaster.
You submit a "track program": a ride type, a start tile, and an ordered list of track pieces.
The game engine builds it piece by piece, tests it with a real train, and rates it.

## Rules of track geometry
- Pieces chain sequentially from a cursor (position + facing direction). Each piece moves/rotates the cursor.
- The track must form a CLOSED CIRCUIT: the last piece must end exactly where the first begins, facing the same direction, at the same height. Total up-slope pieces must equal total down-slope pieces of the same steepness.
- A trick for closure: any identical piece sequence ending in a 90-degree turn, repeated 4 times, closes a rectangle.
- Start with begin_station, middle_station, end_station (station must be on flat ground, 3-7 pieces).
- up_25 rises 16 z-units per piece; up_60 rises 48. You cannot go below the starting height (the ground).
- Use {{"t": "up_25", "chain": true}} for chain lift hill pieces (needed to climb; trains start slow!).
- Banking must be entered and exited: flat_to_left_bank ... left_bank ... left_bank_to_flat.
- Sloped pieces cannot be banked. Transitions matter: up_25 cannot follow flat directly, use flat_to_up_25.
- The train coasts on gravity after the lift. If it stalls (too little energy for a hill), the test fails or takes forever. Drops give speed; friction bleeds it.

## Ride types
52 = wooden roller coaster (no inversions). This is the required type for this competition.

## Scoring (from the real game engine)
Excitement is primary (higher wins). It rewards: drops, speed, airtime, direction changes, banked turns, length. Intensity above ~10 tanks excitement (guests won't ride); keep intensity under 10.00. Crashes disqualify.

## Piece catalog
{PIECE_CATALOG}

## Map
Flat grass around tile (60, 60); a lake sits near map centre roughly tiles (68-85, 55-75) — do NOT build into it. Stay within tiles 20-120. Directions: dir 0 faces -x, dir 1 faces +y, dir 2 faces +x, dir 3 faces -y.

Submit via the submit_track_program tool. After each attempt you get the eval report (placement errors with exact piece index, or ride stats) and a park screenshot. Iterate and maximise excitement."""

TOOL = {
    "name": "submit_track_program",
    "description": "Submit the coaster track program to build and test.",
    "input_schema": {
        "type": "object",
        "required": ["ride_type", "start", "pieces"],
        "properties": {
            "ride_type": {"type": "integer"},
            "start": {
                "type": "object",
                "required": ["x", "y", "dir"],
                "properties": {
                    "x": {"type": "integer"},
                    "y": {"type": "integer"},
                    "dir": {"type": "integer", "minimum": 0, "maximum": 3},
                },
            },
            "pieces": {
                "type": "array",
                "minItems": 4,
                "items": {
                    "anyOf": [
                        {"type": "string"},
                        {
                            "type": "object",
                            "required": ["t"],
                            "properties": {"t": {"type": "string"}, "chain": {"type": "boolean"}},
                        },
                    ]
                },
            },
        },
    },
}


@dataclass
class Attempt:
    round: int
    program: dict
    report: dict
    screenshot: Path | None

    @property
    def excitement(self) -> float:
        for ride in self.report.get("rides", []):
            if ride.get("excitement") is not None:
                return ride["excitement"]
        return 0.0

    @property
    def summary(self) -> str:
        prog = self.report.get("program") or {}
        if not prog.get("ok"):
            err = (prog.get("error") or {}).get("message", "unknown error")
            placed = prog.get("pieces_placed", 0)
            total = prog.get("pieces_total", 0)
            idx = (prog.get("error") or {}).get("piece_index")
            where = f" at piece {idx}" if idx is not None else ""
            return f"BUILD FAILED{where} ({placed}/{total} placed): {err}"
        rides = self.report.get("rides", [])
        if not rides:
            return "built but no ride data"
        r = rides[0]
        return (
            f"excitement={r.get('excitement')} intensity={r.get('intensity')} nausea={r.get('nausea')} "
            f"tested={r.get('tested')} crashed={r.get('crashed')} length={r.get('ride_length')} "
            f"drops={r.get('num_drops')} airtime={r.get('total_air_time')}"
        )


@dataclass
class Contender:
    model: str
    attempts: list[Attempt] = field(default_factory=list)

    @property
    def best(self) -> Attempt | None:
        rated = [a for a in self.attempts if a.excitement > 0]
        return max(rated, key=lambda a: a.excitement) if rated else None


def run_eval(program: dict, scenario: Path, workdir: Path, ticks: int) -> tuple[dict, Path | None]:
    workdir.mkdir(parents=True, exist_ok=True)
    program_path = workdir / "program.json"
    report_path = workdir / "report.json"
    capture_path = workdir / "park.png"
    program_path.write_text(json.dumps(program, indent=2))

    cmd = [
        str(CLI), "eval", str(scenario),
        "--ticks", str(ticks),
        "--rct2-data-path", str(RCT2_DATA),
        "--program", str(program_path),
        "--out", str(report_path),
        "--capture", str(capture_path),
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if not report_path.exists():
        return {"program": {"ok": False, "error": {"message": f"eval crashed: {proc.stderr[-500:]}"}}}, None

    report = json.loads(report_path.read_text())
    shot = None
    if capture_path.exists():
        small = workdir / "park_small.png"
        subprocess.run(["sips", "-Z", "1500", str(capture_path), "--out", str(small)], capture_output=True)
        shot = small if small.exists() else None
    return report, shot


def feedback_content(attempt: Attempt) -> list[dict]:
    content: list[dict] = [
        {
            "type": "text",
            "text": (
                f"Round {attempt.round} result: {attempt.summary}\n\n"
                f"Full report:\n{json.dumps(attempt.report, indent=1)}\n\n"
                "Revise your design and submit again. Aim for higher excitement (intensity < 10, no crashes)."
            ),
        }
    ]
    if attempt.screenshot is not None:
        content.append(
            {
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/png",
                    "data": base64.b64encode(attempt.screenshot.read_bytes()).decode(),
                },
            }
        )
    return content


def compete(
    client: anthropic.Anthropic | anthropic.AnthropicVertex,
    model: str,
    rounds: int,
    scenario: Path,
    run_dir: Path,
    ticks: int,
) -> Contender:
    contender = Contender(model=model)
    messages: list[dict] = [
        {"role": "user", "content": "Design your best wooden coaster (ride_type 52). Submit your first track program."}
    ]
    for rnd in range(1, rounds + 1):
        response = client.messages.create(
            model=model,
            max_tokens=8000,
            system=SYSTEM_PROMPT,
            messages=messages,
            tools=[TOOL],
            tool_choice={"type": "tool", "name": "submit_track_program"},
        )
        tool_use = next(b for b in response.content if b.type == "tool_use")
        program = tool_use.input
        print(f"  [{model}] round {rnd}: {len(program.get('pieces', []))} pieces submitted", flush=True)

        report, shot = run_eval(program, scenario, run_dir / model.replace("/", "_") / f"round_{rnd}", ticks)
        attempt = Attempt(round=rnd, program=program, report=report, screenshot=shot)
        contender.attempts.append(attempt)
        print(f"  [{model}] round {rnd}: {attempt.summary}", flush=True)

        messages.append({"role": "assistant", "content": response.content})
        messages.append(
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": tool_use.id, "content": feedback_content(attempt)}
                ],
            }
        )
    return contender


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", nargs="+", default=["claude-fable-5", "claude-sonnet-5"])
    parser.add_argument("--rounds", type=int, default=4)
    parser.add_argument("--ticks", type=int, default=25000)
    parser.add_argument("--scenario", type=Path, default=DEFAULT_SCENARIO)
    parser.add_argument("--vertex", action="store_true", help="use Google Vertex AI instead of the first-party API")
    parser.add_argument(
        "--project",
        default=os.environ.get("ANTHROPIC_VERTEX_PROJECT_ID"),
        help="GCP project id (default: $ANTHROPIC_VERTEX_PROJECT_ID)",
    )
    parser.add_argument(
        "--region",
        default=os.environ.get("CLOUD_ML_REGION", "global"),
        help="Vertex region (default: $CLOUD_ML_REGION or global)",
    )
    args = parser.parse_args()

    if not CLI.exists():
        print(f"error: {CLI} not built", file=sys.stderr)
        return 1
    if not args.scenario.exists():
        print(f"error: scenario not found: {args.scenario}", file=sys.stderr)
        return 1

    run_dir = REPO / "evals" / "runs" / time.strftime("%Y%m%d-%H%M%S")
    run_dir.mkdir(parents=True)
    print(f"run dir: {run_dir}")

    if args.vertex:
        # Auth is GCP application-default credentials, not an Anthropic key.
        kwargs = {"region": args.region}
        if args.project:
            kwargs["project_id"] = args.project
        client = anthropic.AnthropicVertex(**kwargs)
    else:
        client = anthropic.Anthropic()
    contenders = [
        compete(client, model, args.rounds, args.scenario, run_dir, args.ticks) for model in args.models
    ]

    print("\n=== FINAL STANDINGS ===")
    ranked = sorted(contenders, key=lambda c: c.best.excitement if c.best else 0.0, reverse=True)
    for place, contender in enumerate(ranked, 1):
        best = contender.best
        if best is None:
            print(f"{place}. {contender.model}: no successful coaster")
        else:
            print(f"{place}. {contender.model}: excitement {best.excitement:.2f} (round {best.round}) — {best.summary}")
    (run_dir / "standings.json").write_text(
        json.dumps(
            [
                {
                    "model": c.model,
                    "best_excitement": c.best.excitement if c.best else None,
                    "attempts": [{"round": a.round, "summary": a.summary} for a in c.attempts],
                }
                for c in ranked
            ],
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
