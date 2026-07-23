# /// script
# requires-python = ">=3.11"
# ///
"""Static site generator for coaster eval runs.

Renders evals/runs/ into a self-contained static site styled after the
OpenRCT2 in-game window chrome: an index leaderboard plus one page per run
with every model's rounds, ratings, track programs, and park screenshots.

Usage:
  uv run evals/site.py                 # writes evals/site/
  uv run evals/site.py --out /tmp/x    # custom output dir
"""

from __future__ import annotations

import argparse
import html
import json
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path

RUNS_DIR = Path(__file__).resolve().parent / "runs"

FONTS = "https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@700;800&family=IBM+Plex+Mono:wght@400;600&display=swap"

CSS = """
:root {
  --sky: #10202e;
  --sky-hi: #1c3a50;
  --panel: #c2b280;
  --panel-dark: #a8956a;
  --panel-deep: #8c7a52;
  --titlebar: #5c3a24;
  --titlebar-text: #f5e6c8;
  --ink: #2a2015;
  --ink-soft: #55462e;
  --excitement: #1d7038;
  --intensity: #b0480f;
  --nausea: #a3357e;
  --fail: #8c1f1f;
  --gold: #d9a520;
}
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
  font-family: "IBM Plex Mono", monospace;
  color: var(--ink);
  background:
    repeating-linear-gradient(45deg, transparent 0 22px, rgba(255,255,255,.015) 22px 44px),
    linear-gradient(180deg, var(--sky-hi), var(--sky) 60%);
  min-height: 100vh;
  padding: 2.5rem 1.25rem 5rem;
}
.wrap { max-width: 960px; margin: 0 auto; }

/* RCT-style window: beveled panel with a title bar */
.window {
  background: var(--panel);
  border: 2px solid var(--ink);
  box-shadow:
    inset 2px 2px 0 rgba(255,255,255,.45),
    inset -2px -2px 0 rgba(0,0,0,.3),
    6px 6px 0 rgba(0,0,0,.35);
  margin-bottom: 2rem;
}
.titlebar {
  background: linear-gradient(180deg, #6f4a2e, var(--titlebar));
  color: var(--titlebar-text);
  font-family: "JetBrains Mono", monospace;
  font-weight: 700;
  font-size: .85rem;
  letter-spacing: .04em;
  padding: .45rem .75rem;
  border-bottom: 2px solid var(--ink);
  display: flex;
  justify-content: space-between;
  align-items: center;
  box-shadow: inset 2px 2px 0 rgba(255,255,255,.18);
}
.titlebar .btn {
  width: 14px; height: 14px;
  background: var(--panel);
  border: 1px solid var(--ink);
  box-shadow: inset 1px 1px 0 rgba(255,255,255,.5), inset -1px -1px 0 rgba(0,0,0,.35);
}
.body { padding: 1rem 1.1rem 1.2rem; }

h1 {
  font-family: "JetBrains Mono", monospace;
  font-weight: 700;
  color: var(--titlebar-text);
  font-size: clamp(1.3rem, 4vw, 2.1rem);
  text-shadow: 3px 3px 0 rgba(0,0,0,.6);
  margin-bottom: .4rem;
}
.tagline { color: #9db4c4; margin-bottom: 2rem; font-size: .9rem; }
h2 {
  font-family: "JetBrains Mono", monospace;
  font-weight: 700;
  font-size: 1rem;
  margin-bottom: .6rem;
}
p { line-height: 1.55; margin-bottom: .7rem; }
a { color: inherit; }
pre {
  background: var(--sky);
  color: #cfe3d2;
  padding: .9rem 1rem;
  overflow-x: auto;
  border: 2px solid var(--ink);
  box-shadow: inset 2px 2px 0 rgba(0,0,0,.5);
  font-size: .78rem;
  line-height: 1.45;
}

table { width: 100%; border-collapse: collapse; font-size: .85rem; }
th {
  font-family: "JetBrains Mono", monospace;
  font-weight: 700;
  font-size: .68rem;
  text-align: left;
  color: var(--ink-soft);
  border-bottom: 2px solid var(--ink);
  padding: .35rem .5rem;
}
td { padding: .45rem .5rem; border-bottom: 1px solid var(--panel-deep); vertical-align: top; }
tr:last-child td { border-bottom: none; }
tr.winner td { background: rgba(217,165,32,.25); }

.medal { font-family: "JetBrains Mono", monospace;
  font-weight: 700; }
.rating-excitement { color: var(--excitement); font-weight: 600; }
.rating-intensity { color: var(--intensity); font-weight: 600; }
.rating-nausea { color: var(--nausea); font-weight: 600; }
.fail { color: var(--fail); font-weight: 600; }
.dim { color: var(--ink-soft); }

.round { border: 2px solid var(--ink); background: var(--panel-dark);
  box-shadow: inset 2px 2px 0 rgba(255,255,255,.3), inset -2px -2px 0 rgba(0,0,0,.25);
  padding: .8rem .9rem; margin-bottom: 1rem; }
.round h3 { font-family: "JetBrains Mono", monospace;
  font-weight: 700; font-size: .8rem; margin-bottom: .5rem; }
.round img {
  display: block; max-width: 100%; margin-top: .7rem;
  border: 2px solid var(--ink);
  box-shadow: 3px 3px 0 rgba(0,0,0,.3);
}
.stats { display: flex; flex-wrap: wrap; gap: .3rem 1.4rem; font-size: .85rem; }
details { margin-top: .7rem; font-size: .8rem; }
summary { cursor: pointer; font-family: "JetBrains Mono", monospace;
  font-weight: 700; font-size: .7rem; }
.footer { color: #6c8394; font-size: .75rem; text-align: center; margin-top: 3rem; }
.footer a { color: #9db4c4; }
.backlink a { color: #9db4c4; font-family: "JetBrains Mono", monospace;
  font-weight: 700; font-size: .8rem; }
"""


@dataclass
class Round:
    number: int
    report: dict
    program: dict | None
    screenshot: Path | None

    @property
    def ride(self) -> dict | None:
        for ride in self.report.get("rides", []):
            if ride.get("excitement") is not None:
                return ride
        return None

    @property
    def excitement(self) -> float:
        ride = self.ride
        return ride["excitement"] if ride else 0.0

    @property
    def build_error(self) -> str | None:
        prog = self.report.get("program") or {}
        if prog.get("ok"):
            return None
        err = (prog.get("error") or {}).get("message", "unknown error")
        idx = (prog.get("error") or {}).get("piece_index")
        where = f" at piece {idx}" if idx is not None else ""
        placed = prog.get("pieces_placed", 0)
        total = prog.get("pieces_total", 0)
        return f"build failed{where} ({placed}/{total} pieces placed): {err}"


@dataclass
class ModelRun:
    model: str
    rounds: list[Round]

    @property
    def best(self) -> Round | None:
        rated = [r for r in self.rounds if r.excitement > 0]
        return max(rated, key=lambda r: r.excitement) if rated else None


@dataclass
class EvalRun:
    name: str
    models: list[ModelRun]

    @property
    def ranked(self) -> list[ModelRun]:
        return sorted(self.models, key=lambda m: m.best.excitement if m.best else 0.0, reverse=True)


def esc(text: object) -> str:
    return html.escape(str(text))


def load_runs(runs_dir: Path) -> list[EvalRun]:
    runs: list[EvalRun] = []
    if not runs_dir.is_dir():
        return runs
    for run_dir in sorted(runs_dir.iterdir(), reverse=True):
        if not run_dir.is_dir():
            continue
        models: list[ModelRun] = []
        for model_dir in sorted(run_dir.iterdir()):
            if not model_dir.is_dir():
                continue
            rounds: list[Round] = []
            for round_dir in sorted(model_dir.glob("round_*")):
                report_path = round_dir / "report.json"
                if not report_path.is_file():
                    continue
                program_path = round_dir / "program.json"
                shot = next(
                    (p for p in (round_dir / "park_small.png", round_dir / "park.png") if p.is_file()),
                    None,
                )
                rounds.append(
                    Round(
                        number=int(round_dir.name.split("_")[1]),
                        report=json.loads(report_path.read_text()),
                        program=json.loads(program_path.read_text()) if program_path.is_file() else None,
                        screenshot=shot,
                    )
                )
            if rounds:
                models.append(ModelRun(model=model_dir.name, rounds=rounds))
        if models:
            runs.append(EvalRun(name=run_dir.name, models=models))
    return runs


def page(title: str, titlebar: str, body: str) -> str:
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{esc(title)}</title>
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="{FONTS}">
<style>{CSS}</style>
</head>
<body>
<div class="wrap">
<h1>{esc(titlebar)}</h1>
<p class="tagline">LLMs design roller coasters. RollerCoaster Tycoon 2's real physics engine builds, tests, and rates them.</p>
{body}
<p class="footer">generated by evals/site.py &middot; <a href="https://github.com/wseaton/CoasterBench/tree/eval">wseaton/CoasterBench#eval</a></p>
</div>
</body>
</html>"""


def window(title: str, inner: str) -> str:
    return (
        f'<div class="window"><div class="titlebar"><span>{esc(title)}</span>'
        f'<span class="btn"></span></div><div class="body">{inner}</div></div>'
    )


def rating_cells(ride: dict | None) -> str:
    if ride is None:
        return '<td colspan="3" class="dim">not rated</td>'
    return (
        f'<td class="rating-excitement">{ride["excitement"]:.2f}</td>'
        f'<td class="rating-intensity">{ride["intensity"]:.2f}</td>'
        f'<td class="rating-nausea">{ride["nausea"]:.2f}</td>'
    )


def how_it_works() -> str:
    diagram = """\
 model (tool_use)          openrct2-cli eval               feedback
+------------------+     +----------------------+     +------------------+
| JSON track       | --> | build piece by piece | --> | ratings report   |
| program          |     | test with real train | --> | park screenshot  |
| (pieces, chain)  |     | rate the ride        |     | -> next round    |
+------------------+     +----------------------+     +------------------+"""
    inner = (
        "<p>Each round a model submits a <em>track program</em>: a ride type, a start tile, "
        "and an ordered list of track pieces. The harness executes it inside the game via the "
        "same validated action path a human player uses, runs the simulation for ~10 game-minutes, "
        "then feeds the eval report and a screenshot back to the model for the next round.</p>"
        f"<pre>{esc(diagram)}</pre>"
        "<p>Excitement is the score. Intensity above 10 tanks it; crashes disqualify. "
        "The game engine is the judge, no LLM grading anywhere.</p>"
    )
    return window("How the harness works", inner)


def standings_table(run: EvalRun) -> str:
    rows = []
    for place, model in enumerate(run.ranked, 1):
        best = model.best
        medal = {1: "1st", 2: "2nd", 3: "3rd"}.get(place, f"{place}th")
        cls = ' class="winner"' if place == 1 and best else ""
        if best:
            ride = best.ride or {}
            cells = rating_cells(ride) + f"<td>round {best.number}/{len(model.rounds)}</td>"
        else:
            cells = '<td colspan="4" class="fail">no successful coaster</td>'
        rows.append(
            f'<tr{cls}><td class="medal">{medal}</td><td>{esc(model.model)}</td>{cells}</tr>'
        )
    return (
        "<table><tr><th></th><th>model</th><th>excitement</th><th>intensity</th>"
        "<th>nausea</th><th>best</th></tr>" + "".join(rows) + "</table>"
    )


def round_block(model: ModelRun, rnd: Round, asset: str | None) -> str:
    parts = [f"<h3>Round {rnd.number}</h3>"]
    error = rnd.build_error
    if error:
        parts.append(f'<p class="fail">{esc(error)}</p>')
    ride = rnd.ride
    if ride:
        stats = (
            f'<span class="rating-excitement">excitement {ride["excitement"]:.2f}</span>'
            f'<span class="rating-intensity">intensity {ride["intensity"]:.2f}</span>'
            f'<span class="rating-nausea">nausea {ride["nausea"]:.2f}</span>'
            f"<span>length {ride.get('ride_length', 0)}</span>"
            f"<span>drops {ride.get('num_drops', 0)}</span>"
            f"<span>airtime {ride.get('total_air_time', 0)}</span>"
            f"<span>{'CRASHED' if ride.get('crashed') else 'tested ok'}</span>"
        )
        parts.append(f'<div class="stats">{stats}</div>')
    elif not error:
        parts.append('<p class="dim">built, but the ride was never rated</p>')
    if rnd.program is not None:
        pieces = rnd.program.get("pieces", [])
        parts.append(
            f"<details><summary>track program ({len(pieces)} pieces)</summary>"
            f"<pre>{esc(json.dumps(rnd.program, indent=1))}</pre></details>"
        )
    if asset is not None:
        parts.append(f'<img src="{esc(asset)}" alt="park screenshot, {esc(model.model)} round {rnd.number}" loading="lazy">')
    return f'<div class="round">{"".join(parts)}</div>'


def build_site(runs: list[EvalRun], out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)

    index_body = [how_it_works()]
    for run in runs:
        inner = standings_table(run) + (
            f'<p style="margin-top:.8rem"><a href="run-{esc(run.name)}.html">full rounds, programs &amp; screenshots &rarr;</a></p>'
        )
        index_body.append(window(f"Run {run.name}", inner))
    if not runs:
        index_body.append(window("No runs yet", "<p>Run <code>uv run evals/driver.py</code> to generate one.</p>"))
    (out / "index.html").write_text(page("Coaster Evals", "COASTER EVALS", "".join(index_body)))

    for run in runs:
        body = [window("Standings", standings_table(run))]
        for model in run.ranked:
            blocks = []
            for rnd in model.rounds:
                asset = None
                if rnd.screenshot is not None:
                    rel = Path("assets") / run.name / model.model / f"round_{rnd.number}{rnd.screenshot.suffix}"
                    dest = out / rel
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(rnd.screenshot, dest)
                    asset = rel.as_posix()
                blocks.append(round_block(model, rnd, asset))
            body.append(window(model.model, "".join(blocks)))
        body.append('<p class="backlink"><a href="index.html">&larr; all runs</a></p>')
        (out / f"run-{run.name}.html").write_text(
            page(f"Coaster Evals — {run.name}", f"RUN {run.name}", "".join(body))
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=Path, default=RUNS_DIR, help="runs directory (default evals/runs)")
    parser.add_argument("--out", type=Path, default=RUNS_DIR.parent / "site", help="output directory (default evals/site)")
    args = parser.parse_args()

    runs = load_runs(args.runs)
    build_site(runs, args.out)
    pages = 1 + len(runs)
    print(f"wrote {pages} page(s) for {len(runs)} run(s) to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
