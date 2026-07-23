# /// script
# requires-python = ">=3.11"
# dependencies = ["anthropic[vertex]>=0.40"]
# ///
"""Coaster design head-to-head: two Claude models iteratively design a coaster.

Each round the model submits a JSON track program (via forced tool use); the
harness runs `openrct2-cli eval` on a fresh copy of the scenario, then feeds
back the eval report and a park screenshot. Best excitement across rounds wins.

Two modes:
  design  (default) — the model designs from scratch; pure design ability.
  library — the model can additionally search the stock RCT2 track design
            library and read full piece sequences; tests information retrieval
            and adaptation. Scores are penalized for similarity to any stock
            design (mirrored copies included), so copying outright scores zero.

Usage (first-party API):
  ANTHROPIC_API_KEY=... uv run evals/driver.py \
      --models claude-fable-5 claude-sonnet-5 --rounds 4 --mode library

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
Inversions (steel types only; wooden does NOT support these). Entry/exit geometry is strict:
  left_vertical_loop / right_vertical_loop: ENTER at a 25-up slope (place flat_to_up_25 first),
    EXIT at a 25-down slope (follow with down_25 or down_25_to_flat). Needs lots of entry speed.
  half_loop_up: enters at 25-up, exits flat but UPSIDE DOWN; must be followed immediately by
    half_loop_down, which exits at a 25-down slope.
  left_corkscrew_up / right_corkscrew_up: enter FLAT unbanked, exit upside down; follow immediately
    with the mirrored corkscrew_down (left_corkscrew_up -> right_corkscrew_down and vice versa), which exits flat.
Helices: left_helix_up_small, right_helix_up_small, left_helix_down_small, right_helix_down_small,
  left_helix_up_large, right_helix_up_large, left_helix_down_large, right_helix_down_large
Special: brakes, booster
"""

# Competition ride types the prompt knows how to describe; other ids work but
# get a generic description.
RIDE_TYPES = {
    51: ("steel twister coaster", "51 = steel twister roller coaster (inversions ALLOWED and rewarded)."),
    52: ("wooden coaster", "52 = wooden roller coaster (no inversions)."),
}


def ride_type_info(ride_type: int) -> tuple[str, str]:
    name, line = RIDE_TYPES.get(ride_type, (f"ride type {ride_type} coaster", f"{ride_type} = the required ride type."))
    return name, line + " This is the required type for this competition."


def build_system_prompt(ride_type: int) -> str:
    _, ride_line = ride_type_info(ride_type)
    return SYSTEM_PROMPT.replace("{RIDE_TYPE_LINE}", ride_line)


SYSTEM_PROMPT = f"""You are competing to design the best RollerCoaster Tycoon 2 roller coaster.
You submit a "track program": a ride type, a start tile, and an ordered list of track pieces.
The game engine builds it piece by piece, tests it with a real train, and rates it.

## Rules of track geometry
- Pieces chain sequentially from a cursor (position + facing direction). Each piece moves/rotates the cursor.
- The track must form a CLOSED CIRCUIT: the last piece must end exactly where the first begins, facing the same direction, at the same height. Total up-slope pieces must equal total down-slope pieces of the same steepness.
- A trick for closure: any identical piece sequence ending in a 90-degree turn, repeated 4 times, closes a rectangle.
- Start with begin_station, middle_station, end_station (station must be on flat ground, 3-7 pieces).
- up_25 rises 16 z-units per piece; up_60 rises 48. You cannot go below the starting height (the ground).
- Use {{"t": "up_25", "chain": true}} for chain lift hill pieces (needed to climb; trains start slow!). Chain lifts only work on 25-degree slopes, never on 60-degree pieces.
- Banking must be entered and exited: flat_to_left_bank ... left_bank ... left_bank_to_flat.
- Sloped pieces cannot be banked. Transitions matter: up_25 cannot follow flat directly, use flat_to_up_25.
- The train coasts on gravity after the lift. If it stalls (too little energy for a hill), the test fails or takes forever. Drops give speed; friction bleeds it.

## Ride types
{{RIDE_TYPE_LINE}}

## Scoring (from the real game engine)
Excitement is primary (higher wins). It rewards: drops, speed, airtime, direction changes, banked turns, length. Intensity above ~10 tanks excitement (guests won't ride); keep intensity under 10.00. Crashes disqualify.

Your track is also compared against the stock RCT2 track design library (mirrored variants included). Similarity up to 0.5 is free; above that your excitement is scaled down linearly, reaching zero for an exact copy. Design something original; reproducing a stock coaster from memory scores nothing.

## Piece catalog
{PIECE_CATALOG}

## Map
Flat grass around tile (60, 60); a lake sits near map centre roughly tiles (68-85, 55-75) — do NOT build into it. Stay within tiles 20-120. Directions: dir 0 faces -x, dir 1 faces +y, dir 2 faces +x, dir 3 faces -y.

Submit via the submit_track_program tool. After each attempt you get the eval report (placement errors with exact piece index, or ride stats) and a park screenshot. Iterate and maximise excitement."""

LIBRARY_PROMPT = """

## Track design library
You can browse the stock RCT2 track design library before submitting:
- search_track_designs lists designs (name, ride type, piece count); filter with the ride_type parameter.
- get_track_design returns a design's full piece sequence in the same format you submit.
Use them to study proven layouts, then design your own. The similarity penalty applies to these exact designs, so copying (or mirroring) one scores zero; the winning move is understanding why they work and building something original with that knowledge."""

LIBRARY_TOOLS = [
    {
        "name": "search_track_designs",
        "description": "List the stock track design library, optionally filtered by ride type. Returns name, ride type, and piece count per design.",
        "input_schema": {
            "type": "object",
            "properties": {"ride_type": {"type": "integer"}},
        },
    },
    {
        "name": "get_track_design",
        "description": "Full piece sequence of one stock library design, in the same format submit_track_program accepts.",
        "input_schema": {
            "type": "object",
            "required": ["name"],
            "properties": {"name": {"type": "string"}},
        },
    },
]

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


# Similarity below this is free; above it the score scales linearly to zero
# at 1.0 (an exact copy of a stock design).
SIMILARITY_GRACE = 0.5


def similarity_multiplier(similarity: float) -> float:
    if similarity <= SIMILARITY_GRACE:
        return 1.0
    return max(0.0, (1.0 - similarity) / (1.0 - SIMILARITY_GRACE))


@dataclass
class Attempt:
    round: int
    program: dict
    report: dict
    screenshot: Path | None
    # Library tool calls made before this round's submission (library mode).
    lookups: list[dict] = field(default_factory=list)

    @property
    def raw_excitement(self) -> float:
        for ride in self.report.get("rides", []):
            if ride.get("excitement") is not None:
                return ride["excitement"]
        return 0.0

    @property
    def similarity(self) -> float:
        sim = self.report.get("similarity") or {}
        return sim.get("similarity", 0.0)

    @property
    def excitement(self) -> float:
        """Raw excitement scaled down for copying a stock library design."""
        return self.raw_excitement * similarity_multiplier(self.similarity)

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
        text = (
            f"excitement={r.get('excitement')} intensity={r.get('intensity')} nausea={r.get('nausea')} "
            f"tested={r.get('tested')} crashed={r.get('crashed')} length={r.get('ride_length')} "
            f"drops={r.get('num_drops')} airtime={r.get('total_air_time')}"
        )
        sim = self.report.get("similarity") or {}
        if sim:
            text += f" similarity={sim.get('similarity', 0.0):.2f} (nearest: {sim.get('nearest_design')})"
        if similarity_multiplier(self.similarity) < 1.0:
            text += f" -> penalized excitement {self.excitement:.2f}"
        return text


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
        "--capture-xray",
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if not report_path.exists():
        return {"program": {"ok": False, "error": {"message": f"eval crashed: {proc.stderr[-500:]}"}}}, None

    report = json.loads(report_path.read_text())
    shot = None
    if capture_path.exists():
        small = workdir / "park_small.png"
        # The API rejects images over 5 MB of base64 (~3.7 MB raw); tall parks
        # can exceed that even downscaled, so keep shrinking until it fits.
        for px in (1500, 1100, 800, 600):
            subprocess.run(["sips", "-Z", str(px), str(capture_path), "--out", str(small)], capture_output=True)
            if small.exists() and small.stat().st_size * 4 / 3 < 4_900_000:
                shot = small
                break
    return report, shot


def dump_library(scenario: Path, run_dir: Path) -> list[dict]:
    """Exports the stock design library via the CLI (library mode only)."""
    out = run_dir / "library.json"
    cmd = [
        str(CLI), "eval", str(scenario),
        "--rct2-data-path", str(RCT2_DATA),
        "--dump-library", str(out),
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if not out.exists():
        raise RuntimeError(f"library dump failed: {proc.stderr[-500:]}")
    return json.loads(out.read_text())


PREVIEWS_DIR = REPO / "evals" / "library-previews"


def ensure_library_previews(scenario: Path) -> None:
    """Renders design preview PNGs once; the library is static, so the cache
    survives across runs (evals/library-previews/, gitignored)."""
    if PREVIEWS_DIR.is_dir() and any(PREVIEWS_DIR.glob("*.png")):
        return
    print("rendering track design previews (one-time)...")
    cmd = [
        str(CLI), "eval", str(scenario),
        "--rct2-data-path", str(RCT2_DATA),
        "--render-library", str(PREVIEWS_DIR),
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    count = len(list(PREVIEWS_DIR.glob("*.png"))) if PREVIEWS_DIR.is_dir() else 0
    if count == 0:
        # Previews only feed the site gallery; a render failure shouldn't
        # block the eval itself.
        print(f"warning: preview render produced no images: {proc.stderr[-300:]}", file=sys.stderr)
    else:
        print(f"library previews: {count} images in {PREVIEWS_DIR}")


# In library mode the model may look designs up before submitting; cap the
# lookups per round so a browsing spree cannot stall the eval.
MAX_LOOKUPS_PER_ROUND = 6


def library_tool_result(name: str, tool_input: dict, library: list[dict]) -> tuple[str, dict]:
    """Answers a library tool call. Returns (tool_result_json, lookup_record)."""
    if name == "search_track_designs":
        ride_type = tool_input.get("ride_type")
        designs = [
            {"name": d["name"], "ride_type": d["ride_type"], "piece_count": d["piece_count"]}
            for d in library
            if ride_type is None or d["ride_type"] == ride_type
        ]
        result = json.dumps(
            {"designs": designs, "note": "final score is penalized for similarity to any of these designs"}
        )
        return result, {"tool": "search", "ride_type": ride_type, "results": len(designs)}
    target = tool_input.get("name", "")
    for d in library:
        if d["name"].lower() == target.lower():
            result = json.dumps({k: d[k] for k in ("name", "ride_type", "piece_count", "pieces")})
            return result, {"tool": "get", "name": d["name"], "found": True}
    return json.dumps({"error": f"no such design: {target}"}), {"tool": "get", "name": target, "found": False}


def prune_history(messages: list[dict]) -> None:
    """Trims completed-round bulk from the message history, in place.

    Two payloads dominate context growth: the base64 park screenshot in each
    round's feedback and the full piece list of every get_track_design result.
    The model has already consumed both, and the submitted programs stay in
    history, so all but the most recent screenshot collapse to a stub and old
    design payloads keep only their name and piece count.
    """
    image_stub = {"type": "text", "text": "[park screenshot elided; see latest round]"}
    last_image: tuple[int, int, int] | None = None
    for m, message in enumerate(messages):
        if message.get("role") != "user" or not isinstance(message.get("content"), list):
            continue
        for c, block in enumerate(message["content"]):
            if not isinstance(block, dict) or block.get("type") != "tool_result":
                continue
            content = block.get("content")
            if isinstance(content, list):
                for b, inner in enumerate(content):
                    if isinstance(inner, dict) and inner.get("type") == "image":
                        if last_image is not None:
                            pm, pc, pb = last_image
                            messages[pm]["content"][pc]["content"][pb] = image_stub
                        last_image = (m, c, b)
            elif isinstance(content, str) and '"pieces"' in content:
                try:
                    payload = json.loads(content)
                except json.JSONDecodeError:
                    continue
                if "pieces" in payload:
                    payload["pieces"] = f"[{payload.get('piece_count', '?')} pieces elided; fetch again if needed]"
                    block["content"] = json.dumps(payload)


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
    ride_type: int,
    library: list[dict] | None = None,
) -> Contender:
    contender = Contender(model=model)
    ride_name, _ = ride_type_info(ride_type)
    system_prompt = build_system_prompt(ride_type) + (LIBRARY_PROMPT if library is not None else "")
    tools = [TOOL] + (LIBRARY_TOOLS if library is not None else [])
    messages: list[dict] = [
        {
            "role": "user",
            "content": f"Design your best {ride_name} (ride_type {ride_type}). Submit your first track program.",
        }
    ]
    for rnd in range(1, rounds + 1):
        # In library mode the model may browse designs first; the last step
        # forces a submission so every round produces an attempt.
        program = None
        tool_use = None
        lookups: list[dict] = []
        for step in range(MAX_LOOKUPS_PER_ROUND + 1):
            force_submit = library is None or step == MAX_LOOKUPS_PER_ROUND
            response = client.messages.create(
                model=model,
                max_tokens=8000,
                system=system_prompt,
                messages=messages,
                tools=tools,
                tool_choice=(
                    {"type": "tool", "name": "submit_track_program"} if force_submit else {"type": "any"}
                ),
            )
            tool_use = next(b for b in response.content if b.type == "tool_use")
            messages.append({"role": "assistant", "content": response.content})
            if tool_use.name == "submit_track_program":
                program = tool_use.input
                break
            print(f"  [{model}] round {rnd}: {tool_use.name}({json.dumps(tool_use.input)})", flush=True)
            result, lookup = library_tool_result(tool_use.name, tool_use.input, library or [])
            lookups.append(lookup)
            messages.append(
                {
                    "role": "user",
                    "content": [{"type": "tool_result", "tool_use_id": tool_use.id, "content": result}],
                }
            )
        if program is None or tool_use is None:
            # Unreachable: the final loop step forces submit_track_program.
            raise RuntimeError(f"{model} never submitted a program in round {rnd}")
        if program.get("ride_type") != ride_type:
            print(
                f"  [{model}] round {rnd}: submitted ride_type {program.get('ride_type')}, forcing {ride_type}",
                flush=True,
            )
            program["ride_type"] = ride_type
        print(f"  [{model}] round {rnd}: {len(program.get('pieces', []))} pieces submitted", flush=True)

        round_dir = run_dir / model.replace("/", "_") / f"round_{rnd}"
        report, shot = run_eval(program, scenario, round_dir, ticks)
        attempt = Attempt(round=rnd, program=program, report=report, screenshot=shot, lookups=lookups)
        contender.attempts.append(attempt)
        if lookups:
            (round_dir / "lookups.json").write_text(json.dumps(lookups, indent=2))
        print(f"  [{model}] round {rnd}: {attempt.summary}", flush=True)

        messages.append(
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": tool_use.id, "content": feedback_content(attempt)}
                ],
            }
        )
        prune_history(messages)
    return contender


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", nargs="+", default=["claude-fable-5", "claude-sonnet-5"])
    parser.add_argument(
        "--mode",
        choices=["design", "library"],
        default="design",
        help="design = from scratch; library = with track design library search (retrieval eval)",
    )
    parser.add_argument("--rounds", type=int, default=4)
    parser.add_argument(
        "--ride-type",
        type=int,
        default=52,
        help="required coaster ride type for the competition (52 wooden, 51 steel twister)",
    )
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
    parser.add_argument(
        "--name",
        help="run dir suffix: evals/runs/<yyyymmdd>-<name> instead of a bare timestamp "
        "(names must sort lexically-chronologically; the site keys on it)",
    )
    args = parser.parse_args()

    if not CLI.exists():
        print(f"error: {CLI} not built", file=sys.stderr)
        return 1
    if not args.scenario.exists():
        print(f"error: scenario not found: {args.scenario}", file=sys.stderr)
        return 1

    suffix = args.name if args.name else time.strftime("%H%M%S")
    run_dir = REPO / "evals" / "runs" / f"{time.strftime('%Y%m%d')}-{suffix}"
    run_dir.mkdir(parents=True)
    print(f"run dir: {run_dir} (mode: {args.mode})")

    library = None
    if args.mode == "library":
        library = dump_library(args.scenario, run_dir)
        print(f"track design library: {len(library)} designs")
        ensure_library_previews(args.scenario)

    (run_dir / "run.json").write_text(
        json.dumps(
            {
                "mode": args.mode,
                "models": args.models,
                "rounds": args.rounds,
                "ticks": args.ticks,
                "ride_type": args.ride_type,
                "scenario": args.scenario.name,
                # The site reads the penalty parameters from here; keep the
                # driver the single source of truth for the scoring math.
                "similarity_grace": SIMILARITY_GRACE,
            },
            indent=2,
        )
    )

    if args.vertex:
        # Auth is GCP application-default credentials, not an Anthropic key.
        kwargs = {"region": args.region}
        if args.project:
            kwargs["project_id"] = args.project
        client = anthropic.AnthropicVertex(**kwargs)
    else:
        client = anthropic.Anthropic()
    contenders = [
        compete(client, model, args.rounds, args.scenario, run_dir, args.ticks, args.ride_type, library)
        for model in args.models
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
            {
                "mode": args.mode,
                "standings": [
                    {
                        "model": c.model,
                        "best_excitement": c.best.excitement if c.best else None,
                        "best_raw_excitement": c.best.raw_excitement if c.best else None,
                        "best_similarity": c.best.similarity if c.best else None,
                        "attempts": [
                            {"round": a.round, "summary": a.summary, "lookups": a.lookups} for a in c.attempts
                        ],
                    }
                    for c in ranked
                ],
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
