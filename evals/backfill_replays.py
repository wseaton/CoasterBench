# /// script
# requires-python = ">=3.11"
# ///
"""Refilm and re-render rounds recorded before the current artifacts existed.

Reruns a round's program.json to film it and re-render park.png cropped. The
rerun is checked: the video needs the excitement to match the round's own (a clip
shows motion, which is what an inert booster changed), the picture only needs the
program to build in full (a still shows track). Neither is written otherwise.

A build montage (--montage) is held to the picture's bar, not the video's: it
films the track being assembled with nothing running, so what it shows is the
same thing a still shows.

Usage:
  uv run evals/backfill_replays.py                        # best round of every run
  uv run evals/backfill_replays.py 20260725-opus5-opennote
  uv run evals/backfill_replays.py --all --no-replay      # pictures only
  uv run evals/backfill_replays.py --montage --no-replay  # add build montages
  uv run evals/backfill_replays.py <run> --evolution       # one clip per model
  uv run evals/backfill_replays.py <run> --trace-montage --all   # replay sessions

An evolution (--evolution) is per model, not per round: every round of a run
built in order on one camera, plus the champion's lap shot from the same place.
It is what the index hero plays.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

EVALS_DIR = Path(__file__).resolve().parent
REPO = EVALS_DIR.parent
RUNS_DIR = EVALS_DIR / "runs"
CLI = REPO / "build" / "coasterbench-cli"
DEFAULT_SCENARIO = "~/rct2-assets/Scenarios/Build your own Six Flags Park.SC6"
DEFAULT_RCT2_DATA = "~/rct2-assets"


def penalised_excitement(report: dict, grace: float) -> float:
    """Round score, by the same rule as driver.py and coaster-bench."""
    rides = report.get("rides") or []
    raw = max((r.get("excitement") or 0.0 for r in rides), default=0.0)
    similarity = (report.get("similarity") or {}).get("similarity") or 0.0
    if similarity <= grace:
        return raw
    return raw * max((1.0 - similarity) / (1.0 - grace) if grace < 1.0 else 0.0, 0.0)


def rounds_of(model_dir: Path) -> list[Path]:
    return sorted(
        (d for d in model_dir.iterdir() if d.is_dir() and d.name.startswith("round_")),
        key=lambda d: int(d.name.split("_")[1]),
    )


def pick_rounds(run_dir: Path, every_round: bool) -> list[Path]:
    """The round directories to film in one run."""
    run_json = run_dir / "run.json"
    grace = 0.5
    if run_json.exists():
        grace = json.loads(run_json.read_text()).get("similarity_grace", 0.5)
    picked: list[Path] = []
    for model_dir in sorted(p for p in run_dir.iterdir() if p.is_dir()):
        candidates = [d for d in rounds_of(model_dir) if (d / "program.json").exists()]
        if every_round:
            picked.extend(candidates)
            continue
        scored = []
        for round_dir in candidates:
            report = round_dir / "report.json"
            if not report.exists():
                continue
            scored.append(
                (penalised_excitement(json.loads(report.read_text()), grace), round_dir)
            )
        best = max(scored, default=None)
        if best is not None and best[0] > 0:
            picked.append(best[1])
    return picked


def built_in_full(report: dict) -> bool:
    """Every piece went down, so the track on screen is the round's."""
    program = report.get("program") or {}
    total = program.get("pieces_total") or 0
    return bool(program.get("ok")) and total > 0 and program.get("pieces_placed") == total


def raw_excitement(report: dict) -> float:
    rides = report.get("rides") or []
    return max((r.get("excitement") or 0.0 for r in rides), default=0.0)


def rerun(
    round_dir: Path,
    scenario: str,
    rct2_data: str,
    ticks: int,
    seconds: int,
    capture: bool,
    replay: bool,
    montage: bool,
) -> tuple[bool, bool, bool]:
    """Rebuilds the round's ride. Returns (picture, video, montage) written."""
    out = round_dir / "replay.mp4"
    # These live and die with the video, or a rejected round keeps a picture of
    # a ride it never had.
    poster = round_dir / "replay.png"
    sidecar = round_dir / "replay.json"
    check = round_dir / "replay-check.json"
    shot = round_dir / "park-new.png"
    montage_out = round_dir / "montage.mp4"
    montage_extras = [round_dir / "montage.png", round_dir / "montage.json"]
    report = json.loads((round_dir / "report.json").read_text())
    styled = bool(report.get("presentation"))
    result = subprocess.run(
        [
            str(CLI),
            "eval",
            str(Path(scenario).expanduser()),
            "--ticks",
            str(ticks),
            "--rct2-data-path",
            str(Path(rct2_data).expanduser()),
            "--program",
            str(round_dir / "program.json"),
            "--out",
            str(check),
        ]
        # program.json carries no colours, so a styled round reruns stock gold
        # while its archived clip is blue. The montage and the lap have to
        # match: on the index hero they play back to back.
        + (["--presentation", str(round_dir / "report.json")] if styled else [])
        + (["--replay", str(out), "--replay-seconds", str(seconds)] if replay else [])
        # Staged: a rerun that builds a different coaster must not have already
        # overwritten the round's picture on its way to being rejected.
        + (["--capture", str(shot)] if capture else [])
        # Last on the command line as well as in the run: the montage tears the
        # ride down to rebuild it, so nothing after it would see the scored one.
        + (["--build-montage", str(montage_out)] if montage else []),
        capture_output=True,
        text=True,
    )
    try:
        if result.returncode != 0 or (replay and not out.exists()):
            tail = (result.stderr or result.stdout).strip().splitlines()[-3:]
            print(f"  failed: {' / '.join(tail)}", file=sys.stderr)
            out.unlink(missing_ok=True)
            poster.unlink(missing_ok=True)
            sidecar.unlink(missing_ok=True)
            montage_out.unlink(missing_ok=True)
            for extra in montage_extras:
                extra.unlink(missing_ok=True)
            return (False, False, False)
        rerun_report = json.loads(check.read_text()) if check.exists() else {}
        want = raw_excitement(report)
        got = raw_excitement(rerun_report)
        # Ratings are fixed-point hundredths, so the ride is either exactly the
        # round's or it is a different one.
        same_ride = abs(want - got) <= 0.005
        drew_track = built_in_full(rerun_report)
        wrote_picture = capture and shot.exists() and drew_track
        if wrote_picture:
            shot.replace(round_dir / "park.png")
        # Nothing moves in a montage, so it is judged on the track being the
        # round's, the same bar as the picture.
        wrote_montage = montage and montage_out.exists() and drew_track
        if montage and not wrote_montage:
            montage_out.unlink(missing_ok=True)
            for extra in montage_extras:
                extra.unlink(missing_ok=True)
        if not same_ride:
            out.unlink(missing_ok=True)
            poster.unlink(missing_ok=True)
            sidecar.unlink(missing_ok=True)
            drawn = " (picture kept, track is the same)" if drew_track else ""
            print(
                f"  rated {want:.2f}, rerun {got:.2f}: no video{drawn}",
                file=sys.stderr,
            )
        return (wrote_picture, same_ride and replay, wrote_montage)
    finally:
        check.unlink(missing_ok=True)
        shot.unlink(missing_ok=True)


def trace_actions(round_dir: Path) -> list[dict]:
    """The park-changing calls a round's session made, in order.

    Only the ones the game accepted: a rejected placement changed nothing, so
    filming it would show a frame identical to the last. Queries
    (valid_next_pieces, piece_geometry, screenshot) never move a piece and are
    dropped too.
    """
    actions: list[dict] = []
    trace = round_dir / "trace.jsonl"
    if not trace.exists():
        return actions
    for line in trace.read_text().splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("status") != "completed":
            continue
        name = event.get("name")
        args = event.get("input") or {}
        if name == "new_ride":
            actions.append(
                {
                    "op": "new_ride",
                    "ride_type": args.get("ride_type", 51),
                    "x": args.get("x", 0),
                    "y": args.get("y", 0),
                    "dir": args.get("dir", 0),
                }
            )
        elif name == "place_piece":
            # The tool's own arguments are already a piece spec.
            actions.append({"op": "place", "pieces": [args]})
        elif name == "place_pieces":
            pieces = args.get("pieces") or []
            if pieces:
                actions.append({"op": "place", "pieces": pieces})
        elif name == "undo_piece":
            actions.append({"op": "undo"})
        elif name == "demolish":
            actions.append({"op": "demolish"})
        elif name == "finish_and_test":
            actions.append({"op": "test"})
    return actions


def piece_names(pieces: list) -> list[str]:
    """Piece specs down to bare names, for comparing a replay to a program."""
    names = []
    for piece in pieces:
        if isinstance(piece, str):
            names.append(piece)
        elif isinstance(piece, dict):
            names.append(str(piece.get("t") or piece.get("piece") or ""))
        else:
            names.append(str(piece))
    return names


def film_trace(round_dir: Path, scenario: str, rct2_data: str) -> bool:
    """Replays a round's own session on film: placements, undos, demolitions.

    Verified the way the picture is: what the replay leaves standing has to be
    the round's recorded program. A round that ends demolished leaves nothing,
    and that is the truth about that round, so an empty program matches.
    """
    actions = trace_actions(round_dir)
    if not actions:
        print("  no accepted session calls in the trace", file=sys.stderr)
        return False
    listing = round_dir / "trace-actions.json"
    listing.write_text(json.dumps(actions, indent=1))
    out = round_dir / "trace-montage.mp4"
    sidecar = round_dir / "trace-montage.json"
    try:
        result = subprocess.run(
            [
                str(CLI),
                "eval",
                str(Path(scenario).expanduser()),
                "--ticks",
                "1",
                "--rct2-data-path",
                str(Path(rct2_data).expanduser()),
                "--trace-montage",
                str(out),
                "--trace-actions",
                str(listing),
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0 or not out.exists():
            tail = (result.stderr or result.stdout).strip().splitlines()[-3:]
            print(f"  failed: {' / '.join(tail)}", file=sys.stderr)
            return False
        meta = json.loads(sidecar.read_text()) if sidecar.exists() else {}
        surviving = meta.get("surviving_pieces") or []
        program = round_dir / "program.json"
        recorded = (
            piece_names(json.loads(program.read_text()).get("pieces") or [])
            if program.exists()
            else []
        )
        if surviving != recorded:
            print(
                f"  replay left {len(surviving)} piece(s), the program records "
                f"{len(recorded)}: not the round's ride, no montage",
                file=sys.stderr,
            )
            out.unlink(missing_ok=True)
            sidecar.unlink(missing_ok=True)
            (round_dir / "trace-montage.png").unlink(missing_ok=True)
            return False
        return True
    finally:
        listing.unlink(missing_ok=True)


def film_evolution(
    model_dir: Path, scenario: str, rct2_data: str, seconds: int
) -> bool:
    """One clip of a model's whole run: every round in order, on one camera.

    Plus the champion's lap shot from the same place, because the index hero
    plays them back to back and a lap framed to its own round would jump.
    """
    rounds = [d for d in rounds_of(model_dir) if (d / "program.json").exists()]
    if not rounds:
        return False
    manifest = model_dir / "evolution-manifest.json"
    manifest.write_text(
        json.dumps(
            [
                {"program": str(d / "program.json"), "report": str(d / "report.json")}
                for d in rounds
            ],
            indent=1,
        )
    )
    out = model_dir / "evolution.mp4"
    try:
        result = subprocess.run(
            [
                str(CLI),
                "eval",
                str(Path(scenario).expanduser()),
                # No --program and no ratings to compute: the evolution builds
                # every round itself and nothing needs simulating first.
                "--ticks",
                "1",
                "--rct2-data-path",
                str(Path(rct2_data).expanduser()),
                "--evolution",
                str(manifest),
                "--evolution-montage",
                str(out),
                "--evolution-lap",
                str(model_dir / "evolution-lap.mp4"),
                "--replay-seconds",
                str(seconds),
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0 or not out.exists():
            tail = (result.stderr or result.stdout).strip().splitlines()[-3:]
            print(f"  failed: {' / '.join(tail)}", file=sys.stderr)
            return False
        return True
    finally:
        manifest.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runs", nargs="*", help="run names (default: all)")
    parser.add_argument("--all", action="store_true", help="every round, not just the best")
    parser.add_argument("--force", action="store_true", help="redo rounds that already have one")
    parser.add_argument(
        "--no-replay",
        action="store_true",
        help="only re-render park.png, do not film (much faster, no new videos)",
    )
    parser.add_argument(
        "--no-capture",
        action="store_true",
        help="leave park.png alone instead of re-rendering it cropped",
    )
    parser.add_argument(
        "--montage",
        action="store_true",
        help="also film montage.mp4, the track being built piece by piece",
    )
    parser.add_argument(
        "--evolution",
        action="store_true",
        help="film one evolution.mp4 per model: every round in order on one camera",
    )
    parser.add_argument(
        "--trace-montage",
        action="store_true",
        help="film each round's own session (placements, undos, demolitions) from its trace",
    )
    parser.add_argument("--scenario", default=DEFAULT_SCENARIO)
    parser.add_argument("--rct2-data", default=DEFAULT_RCT2_DATA)
    parser.add_argument("--ticks", type=int, default=25000)
    parser.add_argument("--replay-seconds", type=int, default=90)
    args = parser.parse_args()

    if not CLI.exists():
        print(f"{CLI} not built (cmake --build build)", file=sys.stderr)
        return 1

    names = args.runs or sorted(p.name for p in RUNS_DIR.iterdir() if p.is_dir())
    pictures = videos = montages = evolutions = traces = skipped = failed = 0
    for name in names:
        run_dir = RUNS_DIR / name
        if not run_dir.is_dir():
            print(f"no such run: {name}", file=sys.stderr)
            failed += 1
            continue
        if args.trace_montage:
            for round_dir in pick_rounds(run_dir, args.all):
                rel = round_dir.relative_to(RUNS_DIR)
                if (round_dir / "trace-montage.mp4").exists() and not args.force:
                    skipped += 1
                    continue
                print(f"replaying {rel} session ...", flush=True)
                started = time.monotonic()
                if film_trace(round_dir, args.scenario, args.rct2_data):
                    traces += 1
                    print(f"  done in {time.monotonic() - started:.0f}s")
                else:
                    failed += 1
            continue
        if args.evolution:
            for model_dir in sorted(p for p in run_dir.iterdir() if p.is_dir()):
                if (model_dir / "evolution.mp4").exists() and not args.force:
                    skipped += 1
                    continue
                print(f"filming {model_dir.relative_to(RUNS_DIR)} evolution ...", flush=True)
                started = time.monotonic()
                if film_evolution(model_dir, args.scenario, args.rct2_data, args.replay_seconds):
                    evolutions += 1
                    print(f"  done in {time.monotonic() - started:.0f}s")
                else:
                    failed += 1
            continue
        for round_dir in pick_rounds(run_dir, args.all):
            rel = round_dir.relative_to(RUNS_DIR)
            wanted_clips = [
                round_dir / clip
                for clip, asked in (
                    ("replay.mp4", not args.no_replay),
                    ("montage.mp4", args.montage),
                )
                if asked
            ]
            if wanted_clips and all(p.exists() for p in wanted_clips) and not args.force:
                skipped += 1
                continue
            print(f"{'filming' if wanted_clips else 'rendering'} {rel} ...", flush=True)
            started = time.monotonic()
            picture, video, montage = rerun(
                round_dir,
                args.scenario,
                args.rct2_data,
                args.ticks,
                args.replay_seconds,
                not args.no_capture,
                not args.no_replay,
                args.montage,
            )
            pictures += picture
            videos += video
            montages += montage
            if not (picture or video or montage):
                failed += 1
            else:
                print(f"  done in {time.monotonic() - started:.0f}s")
    print(
        f"{pictures} picture(s), {videos} video(s), {montages} montage(s), "
        f"{evolutions} evolution(s), {traces} trace replay(s), "
        f"{skipped} already had one, {failed} wrote nothing"
    )
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
