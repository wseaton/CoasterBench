# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow>=10"]
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
import re
import shutil
import sys
from dataclasses import dataclass, field
from pathlib import Path

from PIL import Image, ImageDraw

RUNS_DIR = Path(__file__).resolve().parent / "runs"

TAGLINE = (
    "LLMs design roller coasters. RollerCoaster Tycoon 2's real physics engine "
    "builds, tests, and rates them."
)

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
  --excitement: #124a26;
  --intensity: #6b2807;
  --nausea: #6d2255;
  --fail: #701818;
  --excitement-mark: #24824a;
  --intensity-mark: #96420f;
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
.wrap-wide { max-width: min(1960px, 96vw); }

/* Run pages: stack model windows side by side when the viewport allows,
   so comparing rounds across models doesn't mean scrolling one full model
   at a time. The standings window above the grid keeps full width. */
.model-grid { display: grid; grid-template-columns: 1fr; gap: 0 1.5rem; align-items: start; }
@media (min-width: 1200px) {
  .model-grid { grid-template-columns: repeat(auto-fit, minmax(560px, 1fr)); }
}

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

/* Track design preview gallery (library mode) */
.gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(185px, 1fr));
  gap: .6rem; margin-top: .7rem; }
.gallery figure { border: 2px solid var(--ink); background: var(--sky);
  box-shadow: 2px 2px 0 rgba(0,0,0,.3); margin: 0;
  display: flex; flex-direction: column; }
.gallery img, .gallery .no-preview {
  display: block; width: 100%; aspect-ratio: 370 / 217; object-fit: cover;
  image-rendering: pixelated; }
.gallery .no-preview { display: flex; align-items: center; justify-content: center;
  color: var(--ink-soft); font-size: .65rem; }
.gallery figcaption { font-family: "JetBrains Mono", monospace; font-weight: 700;
  font-size: .62rem; padding: .3rem .4rem; color: var(--titlebar-text);
  background: linear-gradient(180deg, #6f4a2e, var(--titlebar));
  border-top: 2px solid var(--ink); overflow-wrap: anywhere;
  /* Uniform two-line caption well so every card is the same height and the
     grid rows (and the model columns above them) stay aligned. */
  margin-top: auto; min-height: 2.55em; display: flex; align-items: center;
  overflow: hidden; }
.chart { margin-bottom: 1rem; }
.chart svg { display: block; width: 100%; height: auto; }
.chart-legend { display: flex; gap: 1.2rem; font-size: .75rem; margin-bottom: .2rem; }
.chart-legend .chip { display: inline-block; width: 10px; height: 10px; margin-right: .35rem; }
.rotator { position: relative; }
.rotator .rot-btn {
  position: absolute; bottom: 10px; width: 30px; height: 26px;
  font-family: "JetBrains Mono", monospace; font-size: .7rem; cursor: pointer;
  color: var(--ink); background: var(--panel);
  border: 2px solid var(--ink);
  box-shadow: inset 1px 1px 0 rgba(255,255,255,.5), inset -1px -1px 0 rgba(0,0,0,.35);
}
.rotator .rot-btn:active { box-shadow: inset -1px -1px 0 rgba(255,255,255,.5), inset 1px 1px 0 rgba(0,0,0,.35); }
.rotator .rot-prev { right: 78px; }
.rotator .rot-next { right: 10px; }
.rotator .rot-count {
  position: absolute; bottom: 12px; right: 44px; width: 30px; text-align: center;
  font-family: "JetBrains Mono", monospace; font-size: .65rem; font-weight: 700;
  color: var(--titlebar-text); text-shadow: 1px 1px 0 rgba(0,0,0,.7);
}
details { margin-top: .7rem; font-size: .8rem; }
summary { cursor: pointer; font-family: "JetBrains Mono", monospace;
  font-weight: 700; font-size: .7rem; }
.footer { color: #6c8394; font-size: .75rem; text-align: center; margin-top: 3rem; }
.footer a { color: #9db4c4; }
.backlink a { color: #9db4c4; font-family: "JetBrains Mono", monospace;
  font-weight: 700; font-size: .8rem; }
"""


# The driver's copy penalty: similarity to a stock library design up to the
# grace threshold is free, then the score scales linearly to zero at 1.0.
# Each run records its threshold in run.json (the driver is the source of
# truth); this default only covers runs from before it was recorded.
DEFAULT_SIMILARITY_GRACE = 0.5


def similarity_multiplier(similarity: float, grace: float) -> float:
    if similarity <= grace:
        return 1.0
    return max(0.0, (1.0 - similarity) / (1.0 - grace))


MODE_TAGLINES = {
    "design": "design mode — models design from scratch",
    "library": "library mode — models may search the stock track design library; tests retrieval and adaptation, copies score zero",
}


@dataclass
class Round:
    number: int
    report: dict
    program: dict | None
    screenshot: Path | None
    # Additional view rotations of the same capture (park-r1/2/3.png).
    rotation_shots: list[Path] = field(default_factory=list)
    # See-through verification capture (park-x.png): terrain and supports
    # hidden so tunnelled track is visible. The upstream sprite-sort glitch
    # that hides track near track crossings at some rotations still applies.
    xray_shot: Path | None = None
    # Library tool calls the model made before submitting (library mode).
    lookups: list[dict] = field(default_factory=list)
    grace: float = DEFAULT_SIMILARITY_GRACE

    @property
    def ride(self) -> dict | None:
        for ride in self.report.get("rides", []):
            if ride.get("excitement") is not None:
                return ride
        return None

    @property
    def similarity(self) -> dict | None:
        return self.report.get("similarity") or None

    @property
    def excitement(self) -> float:
        """Raw excitement scaled down for copying a stock library design."""
        ride = self.ride
        if ride is None:
            return 0.0
        sim = (self.similarity or {}).get("similarity", 0.0)
        return ride["excitement"] * similarity_multiplier(sim, self.grace)

    @property
    def fetched_designs(self) -> list[str]:
        return [l["name"] for l in self.lookups if l.get("tool") == "get" and l.get("found")]

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
    mode: str
    grace: float
    models: list[ModelRun]

    @property
    def ranked(self) -> list[ModelRun]:
        return sorted(self.models, key=lambda m: m.best.excitement if m.best else 0.0, reverse=True)


def esc(text: object) -> str:
    return html.escape(str(text))


PREVIEWS_DIR = Path(__file__).resolve().parent / "library-previews"


def sanitise_name(name: str) -> str:
    # Keep in sync with RenderTrackLibrary in src/openrct2/rustbridge/RustBridge.cpp.
    return re.sub(r"[^A-Za-z0-9._-]", "_", name)


def preview_path(design_name: str) -> Path | None:
    path = PREVIEWS_DIR / f"{sanitise_name(design_name)}.png"
    return path if path.is_file() else None


def load_runs(runs_dir: Path) -> list[EvalRun]:
    runs: list[EvalRun] = []
    if not runs_dir.is_dir():
        return runs
    for run_dir in sorted(runs_dir.iterdir(), reverse=True):
        if not run_dir.is_dir():
            continue
        # Mode and penalty parameters live in run.json (newer runs); older
        # runs are design mode with the default grace.
        meta = {}
        meta_path = run_dir / "run.json"
        if meta_path.is_file():
            meta = json.loads(meta_path.read_text())
        mode = meta.get("mode", "design")
        grace = meta.get("similarity_grace", DEFAULT_SIMILARITY_GRACE)
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
                lookups_path = round_dir / "lookups.json"
                shot = next(
                    (p for p in (round_dir / "park_small.png", round_dir / "park.png") if p.is_file()),
                    None,
                )
                rotation_shots = [
                    p for i in (1, 2, 3) if (p := round_dir / f"park-r{i}.png").is_file()
                ]
                xray = round_dir / "park-x.png"
                rounds.append(
                    Round(
                        number=int(round_dir.name.split("_")[1]),
                        report=json.loads(report_path.read_text()),
                        program=json.loads(program_path.read_text()) if program_path.is_file() else None,
                        screenshot=shot,
                        rotation_shots=rotation_shots,
                        xray_shot=xray if xray.is_file() else None,
                        lookups=json.loads(lookups_path.read_text()) if lookups_path.is_file() else [],
                        grace=grace,
                    )
                )
            if rounds:
                models.append(ModelRun(model=model_dir.name, rounds=rounds))
        if models:
            runs.append(EvalRun(name=run_dir.name, mode=mode, grace=grace, models=models))
    return runs


def unfurl_meta(title: str, path: str, base_url: str | None) -> str:
    """Open Graph / Twitter tags so Slack (and friends) unfurl a rich card.

    Slack ignores relative og:image/og:url, so those tags only appear when a
    base URL is known.
    """
    tags = [
        f'<meta property="og:title" content="{esc(title)}">',
        f'<meta property="og:description" content="{esc(TAGLINE)}">',
        '<meta property="og:type" content="website">',
        '<meta property="og:site_name" content="CoasterBench">',
        '<meta name="twitter:card" content="summary_large_image">',
    ]
    if base_url:
        base = base_url.rstrip("/")
        tags += [
            f'<meta property="og:url" content="{esc(base)}/{esc(path)}">',
            f'<meta property="og:image" content="{esc(base)}/og-card.png">',
            '<meta property="og:image:width" content="1200">',
            '<meta property="og:image:height" content="630">',
        ]
    return "\n".join(tags)


def page(title: str, titlebar: str, body: str, path: str, base_url: str | None, wide: bool = False) -> str:
    wrap_class = "wrap wrap-wide" if wide else "wrap"
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{esc(title)}</title>
{unfurl_meta(title, path, base_url)}
<link rel="icon" href="favicon.ico" sizes="16x16 32x32 48x48">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="{FONTS}">
<style>{CSS}</style>
</head>
<body>
<div class="{wrap_class}">
<h1>{esc(titlebar)}</h1>
<p class="tagline">{esc(TAGLINE)}</p>
{body}
<script>
document.addEventListener('click', function (e) {{
  var btn = e.target.closest('.rot-btn');
  if (!btn) return;
  var rot = btn.closest('.rotator');
  var shots = JSON.parse(rot.dataset.shots);
  var step = btn.classList.contains('rot-next') ? 1 : shots.length - 1;
  var i = (parseInt(rot.dataset.i, 10) + step) % shots.length;
  var labels = JSON.parse(rot.dataset.labels);
  rot.dataset.i = i;
  rot.querySelector('img').src = shots[i];
  rot.querySelector('.rot-count').textContent = labels[i];
}});
</script>
<p class="footer">generated by evals/site.py &middot; <a href="https://github.com/wseaton/CoasterBench/tree/eval">wseaton/CoasterBench#eval</a></p>
</div>
</body>
</html>"""


def window(title: str, inner: str) -> str:
    return (
        f'<div class="window"><div class="titlebar"><span>{esc(title)}</span>'
        f'<span class="btn"></span></div><div class="body">{inner}</div></div>'
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
            sim = (best.similarity or {}).get("similarity")
            sim_cell = f'<td class="dim">{sim:.2f}</td>' if sim is not None else '<td class="dim">&mdash;</td>'
            cells = (
                f'<td class="rating-excitement">{best.excitement:.2f}</td>'
                f'<td class="rating-intensity">{ride.get("intensity", 0):.2f}</td>'
                f'<td class="rating-nausea">{ride.get("nausea", 0):.2f}</td>'
                f"{sim_cell}<td>round {best.number}/{len(model.rounds)}</td>"
            )
        else:
            cells = '<td colspan="5" class="fail">no successful coaster</td>'
        rows.append(
            f'<tr{cls}><td class="medal">{medal}</td><td>{esc(model.model)}</td>{cells}</tr>'
        )
    return (
        "<table><tr><th></th><th>model</th><th>score</th><th>intensity</th>"
        "<th>nausea</th><th>similarity</th><th>best</th></tr>" + "".join(rows) + "</table>"
        '<p class="dim" style="margin-top:.5rem;font-size:.75rem">score = excitement, scaled down when '
        f"similarity to a stock library design exceeds {run.grace:g} (an exact or mirrored copy scores 0)</p>"
    )


def round_chart(model: ModelRun) -> str:
    """Inline SVG of excitement/intensity per round, failed builds marked."""
    rounds = model.rounds
    if len(rounds) < 2:
        return ""
    width, height = 640, 200
    left, right, top, bottom = 36, 12, 16, 26
    plot_w, plot_h = width - left - right, height - top - bottom
    y_max = 16.0

    def x_of(i: int) -> float:
        return left + plot_w * (i / max(1, len(rounds) - 1))

    def y_of(value: float) -> float:
        return top + plot_h * (1 - min(value, y_max) / y_max)

    parts: list[str] = []
    for gy in (0, 5, 15):
        parts.append(
            f'<line x1="{left}" y1="{y_of(gy):.1f}" x2="{width - right}" y2="{y_of(gy):.1f}" '
            'stroke="var(--panel-deep)" stroke-width="1"/>'
        )
        parts.append(
            f'<text x="{left - 6}" y="{y_of(gy) + 4:.1f}" text-anchor="end" font-size="10" '
            f'fill="var(--ink-soft)">{gy}</text>'
        )
    # The intensity ceiling is the story; draw it as a labeled dashed line.
    parts.append(
        f'<line x1="{left}" y1="{y_of(10):.1f}" x2="{width - right}" y2="{y_of(10):.1f}" '
        'stroke="var(--ink-soft)" stroke-width="1" stroke-dasharray="5 4"/>'
    )
    parts.append(
        f'<text x="{width - right}" y="{y_of(10) - 5:.1f}" text-anchor="end" font-size="10" '
        'fill="var(--ink-soft)">intensity ceiling 10</text>'
    )

    series = [
        ("excitement", "var(--excitement-mark)", [(r, r.ride.get("excitement") if r.ride else None) for r in rounds]),
        ("intensity", "var(--intensity-mark)", [(r, r.ride.get("intensity") if r.ride else None) for r in rounds]),
    ]
    best = model.best
    for name, colour, points in series:
        run: list[str] = []
        for i, (_, value) in enumerate(points):
            if value is None:
                if len(run) > 1:
                    parts.append(
                        f'<polyline points="{" ".join(run)}" fill="none" stroke="{colour}" stroke-width="2"/>'
                    )
                run = []
            else:
                run.append(f"{x_of(i):.1f},{y_of(value):.1f}")
        if len(run) > 1:
            parts.append(f'<polyline points="{" ".join(run)}" fill="none" stroke="{colour}" stroke-width="2"/>')
        for i, (rnd, value) in enumerate(points):
            if value is None:
                continue
            cx, cy = x_of(i), y_of(value)
            title = f"<title>round {rnd.number}: {name} {value:.2f}</title>"
            if name == "excitement":
                parts.append(
                    f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="5" fill="{colour}" '
                    f'stroke="var(--panel-dark)" stroke-width="2">{title}</circle>'
                )
                if best is not None and rnd.number == best.number:
                    parts.append(
                        f'<text x="{cx - 9:.1f}" y="{cy - 9:.1f}" text-anchor="end" font-size="11" '
                        f'font-weight="600" fill="var(--ink)">{value:.2f}</text>'
                    )
            else:
                parts.append(
                    f'<rect x="{cx - 4:.1f}" y="{cy - 4:.1f}" width="8" height="8" fill="{colour}" '
                    f'stroke="var(--panel-dark)" stroke-width="2">{title}</rect>'
                )

    for i, rnd in enumerate(rounds):
        cx = x_of(i)
        if rnd.build_error is not None:
            parts.append(
                f'<text x="{cx:.1f}" y="{y_of(0) + 4:.1f}" text-anchor="middle" font-size="13" '
                f'font-weight="600" fill="var(--fail)">&#215;<title>round {rnd.number}: {esc(rnd.build_error)}</title></text>'
            )
        elif rnd.ride is None:
            parts.append(
                f'<circle cx="{cx:.1f}" cy="{y_of(0):.1f}" r="4" fill="none" stroke="var(--ink-soft)" '
                f'stroke-width="2"><title>round {rnd.number}: built but not rated (test never completed)</title></circle>'
            )
        parts.append(
            f'<text x="{cx:.1f}" y="{height - 8}" text-anchor="middle" font-size="10" '
            f'fill="var(--ink-soft)">R{rnd.number}</text>'
        )

    legend = (
        '<div class="chart-legend">'
        '<span><span class="chip" style="background:var(--excitement-mark);border-radius:50%"></span>excitement</span>'
        '<span><span class="chip" style="background:var(--intensity-mark)"></span>intensity</span>'
        f'<span><span class="chip" style="background:none;color:var(--fail)">&#215;</span>build failed</span>'
        "</div>"
    )
    svg = (
        f'<svg viewBox="0 0 {width} {height}" role="img" '
        f'aria-label="excitement and intensity per round for {esc(model.model)}">{"".join(parts)}</svg>'
    )
    return f'<div class="chart">{legend}{svg}</div>'


def round_block(model: ModelRun, rnd: Round, assets: list[tuple[str, str]]) -> str:
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
        sim = rnd.similarity
        if sim:
            stats += f"<span class=\"dim\">similarity {sim.get('similarity', 0.0):.2f} (nearest: {esc(sim.get('nearest_design'))})</span>"
            if rnd.excitement < ride["excitement"]:
                stats += f'<span class="fail">penalized score {rnd.excitement:.2f}</span>'
        parts.append(f'<div class="stats">{stats}</div>')
    elif not error:
        parts.append('<p class="dim">built, but the ride was never rated</p>')
    if rnd.lookups:
        searches = sum(1 for lookup in rnd.lookups if lookup.get("tool") == "search")
        chips = f"library: {searches} search(es)" if searches else "library:"
        if rnd.fetched_designs:
            chips += " studied " + ", ".join(rnd.fetched_designs)
        parts.append(f'<p class="dim" style="font-size:.75rem;margin-top:.4rem">{esc(chips)}</p>')
    if rnd.program is not None:
        pieces = rnd.program.get("pieces", [])
        parts.append(
            f"<details><summary>track program ({len(pieces)} pieces)</summary>"
            f"<pre>{esc(json.dumps(rnd.program, indent=1))}</pre></details>"
        )
    if assets:
        img = (
            f'<img src="{esc(assets[0][0])}" alt="park screenshot, {esc(model.model)} '
            f'round {rnd.number}" loading="lazy">'
        )
        if len(assets) > 1:
            shots_attr = html.escape(json.dumps([src for src, _ in assets]))
            labels_attr = html.escape(json.dumps([label for _, label in assets]))
            parts.append(
                f'<div class="rotator" data-shots="{shots_attr}" data-labels="{labels_attr}" data-i="0">{img}'
                '<button class="rot-btn rot-prev" aria-label="rotate view left">&#9664;</button>'
                f'<span class="rot-count">{esc(assets[0][1])}</span>'
                '<button class="rot-btn rot-next" aria-label="rotate view right">&#9654;</button></div>'
            )
        else:
            parts.append(img)
    return f'<div class="round">{"".join(parts)}</div>'


def copy_preview(out: Path, design_name: str) -> str | None:
    """Copies a design's preview into the site assets; returns the relative
    asset path, or None when no preview was rendered for it."""
    src = preview_path(design_name)
    if src is None:
        return None
    rel = Path("assets") / "library" / src.name
    dest = out / rel
    dest.parent.mkdir(parents=True, exist_ok=True)
    if not dest.is_file():
        shutil.copyfile(src, dest)
    return rel.as_posix()


def design_gallery(out: Path, designs: list[tuple[str, str]]) -> str:
    """Thumbnail grid of (design name, caption) pairs."""
    figures = []
    for name, caption in designs:
        asset = copy_preview(out, name)
        if asset is None:
            figures.append(
                f'<figure><div class="no-preview">no preview</div>'
                f'<figcaption>{esc(caption)}</figcaption></figure>'
            )
        else:
            figures.append(
                f'<figure><img src="{esc(asset)}" alt="track design preview: {esc(name)}" loading="lazy">'
                f"<figcaption>{esc(caption)}</figcaption></figure>"
            )
    return f'<div class="gallery">{"".join(figures)}</div>'


def latest_library_index(runs_dir: Path) -> list[dict]:
    """The most recent run's library.json: name/ride_type/piece_count per
    design, used to caption and order the full gallery page."""
    if not runs_dir.is_dir():
        return []
    for run_dir in sorted(runs_dir.iterdir(), reverse=True):
        library_path = run_dir / "library.json"
        if library_path.is_file():
            return json.loads(library_path.read_text())
    return []


def write_og_card(runs: list[EvalRun], out: Path) -> bool:
    """Render og-card.png: the best-rated coaster's screenshot, center-cropped
    to 1200x630 inside an RCT-style beveled panel frame."""
    shots = [
        (rnd.excitement, rnd.screenshot)
        for run in runs
        for model in run.models
        for rnd in model.rounds
        if rnd.screenshot is not None and rnd.excitement > 0
    ]
    if not shots:
        return False
    shot_path = max(shots, key=lambda s: s[0])[1]

    W, H = 1200, 630
    panel, panel_hi, panel_dark, ink = "#c2b280", "#dccfa4", "#8c7a52", "#2a2015"
    outer, bevel = 3, 6
    border = outer + bevel + 2
    inner_w, inner_h = W - 2 * border, H - 2 * border

    img = Image.open(shot_path).convert("RGB")
    # Cover-fit so the whole ride reads at a glance (captures are cropped to
    # track bounds already). NEAREST when upscaling keeps pixel art crisp.
    scale = max(inner_w / img.width, inner_h / img.height)
    img = img.resize(
        (round(img.width * scale), round(img.height * scale)),
        Image.NEAREST if scale > 1 else Image.LANCZOS,
    )
    left = (img.width - inner_w) // 2
    top = (img.height - inner_h) // 2
    img = img.crop((left, top, left + inner_w, top + inner_h))

    card = Image.new("RGB", (W, H), panel)
    draw = ImageDraw.Draw(card)
    draw.rectangle((0, 0, W - 1, H - 1), outline=ink, width=outer)
    draw.rectangle(
        (outer, outer, W - 1 - outer, H - 1 - outer), outline=panel_hi, width=bevel
    )
    # Bevel shading: dark on the bottom/right edges for the classic inset look.
    draw.rectangle(
        (outer, H - outer - bevel, W - 1 - outer, H - 1 - outer), fill=panel_dark
    )
    draw.rectangle(
        (W - outer - bevel, outer, W - 1 - outer, H - 1 - outer), fill=panel_dark
    )
    card.paste(img, (border, border))
    card.save(out / "og-card.png", optimize=True)
    return True


def write_favicon(out: Path) -> None:
    """A 32x32 RCT-style window (tan bevel panel + brown titlebar) as favicon.ico."""
    img = Image.new("RGB", (32, 32), "#10202e")
    draw = ImageDraw.Draw(img)
    draw.rectangle((2, 4, 29, 27), fill="#c2b280", outline="#2a2015", width=2)
    draw.rectangle((4, 6, 27, 11), fill="#5c3a24")
    draw.line((4, 6, 27, 6), fill="#8a5f3d")
    draw.line((4, 25, 27, 25), fill="#8c7a52")
    draw.line((27, 8, 27, 25), fill="#8c7a52")
    draw.line((4, 8, 4, 24), fill="#dccfa4")
    img.save(out / "favicon.ico", sizes=[(16, 16), (32, 32), (48, 48)])


def build_site(runs: list[EvalRun], out: Path, runs_dir: Path, base_url: str | None) -> None:
    out.mkdir(parents=True, exist_ok=True)
    # Clear previously generated output so renamed/deleted runs don't linger.
    shutil.rmtree(out / "assets", ignore_errors=True)
    for stale in out.glob("*.html"):
        stale.unlink()

    have_previews = PREVIEWS_DIR.is_dir() and any(PREVIEWS_DIR.glob("*.png"))

    index_body = [how_it_works()]
    for mode in ("design", "library"):
        mode_runs = [r for r in runs if r.mode == mode]
        if not mode_runs:
            continue
        mode_intro = f"<p>{esc(MODE_TAGLINES[mode])}</p>"
        if mode == "library" and have_previews:
            mode_intro += '<p style="margin-top:.4rem"><a href="library.html">browse the full track design library &rarr;</a></p>'
        index_body.append(window(f"{mode.capitalize()} mode", mode_intro))
        for run in mode_runs:
            inner = standings_table(run) + (
                f'<p style="margin-top:.8rem"><a href="run-{esc(run.name)}.html">full rounds, programs &amp; screenshots &rarr;</a></p>'
            )
            index_body.append(window(f"Run {run.name}", inner))
    if not runs:
        index_body.append(window("No runs yet", "<p>Run <code>uv run evals/driver.py</code> to generate one.</p>"))
    (out / "index.html").write_text(
        page("Coaster Evals", "COASTER EVALS", "".join(index_body), "index.html", base_url)
    )

    for run in runs:
        body = [
            window("Standings", f"<p class=\"dim\">{esc(MODE_TAGLINES.get(run.mode, run.mode))}</p>" + standings_table(run))
        ]
        model_windows = []
        for model in run.ranked:
            blocks = []
            for rnd in model.rounds:
                assets: list[tuple[str, str]] = []
                shots = ([] if rnd.screenshot is None else [(rnd.screenshot, "view 1")]) + [
                    (p, f"view {i + 2}") for i, p in enumerate(rnd.rotation_shots)
                ]
                if rnd.xray_shot is not None:
                    shots.append((rnd.xray_shot, "x-ray"))
                for shot, label in shots:
                    rel = Path("assets") / run.name / model.model / f"round_{rnd.number}_{shot.stem}{shot.suffix}"
                    dest = out / rel
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(shot, dest)
                    assets.append((rel.as_posix(), label))
                blocks.append(round_block(model, rnd, assets))
            # Gallery of the stock designs this model studied, in fetch order.
            studied: list[str] = []
            for rnd in model.rounds:
                for name in rnd.fetched_designs:
                    if name not in studied:
                        studied.append(name)
            inner = round_chart(model) + "".join(blocks)
            if studied:
                inner = (
                    f"<h2>studied {len(studied)} library design(s)</h2>"
                    + design_gallery(out, [(name, name) for name in studied])
                    + '<div style="height:1rem"></div>'
                    + inner
                )
            model_windows.append(window(model.model, inner))
        body.append(f'<div class="model-grid">{"".join(model_windows)}</div>')
        body.append('<p class="backlink"><a href="index.html">&larr; all runs</a></p>')
        (out / f"run-{run.name}.html").write_text(
            page(
                f"Coaster Evals — {run.name}",
                f"RUN {run.name} ({run.mode})",
                "".join(body),
                f"run-{run.name}.html",
                base_url,
                wide=True,
            )
        )

    if have_previews:
        index = latest_library_index(runs_dir)
        if index:
            entries = [
                (d["name"], f"{d['name']} · type {d['ride_type']} · {d['piece_count']} pieces")
                for d in sorted(index, key=lambda d: (d["ride_type"], d["name"]))
            ]
        else:
            # No library.json yet: caption with the (sanitised) file names.
            entries = [(p.stem, p.stem) for p in sorted(PREVIEWS_DIR.glob("*.png"))]
        body = [
            window(
                f"Track design library ({len(entries)} designs)",
                "<p>The stock RCT2 designs models can search in library mode. "
                "Previews are rendered by the game's own track-preview pipeline.</p>"
                + design_gallery(out, entries),
            ),
            '<p class="backlink"><a href="index.html">&larr; all runs</a></p>',
        ]
        (out / "library.html").write_text(
            page(
                "Coaster Evals — Track Design Library",
                "TRACK DESIGN LIBRARY",
                "".join(body),
                "library.html",
                base_url,
            )
        )

    write_favicon(out)
    if write_og_card(runs, out):
        print(f"wrote og-card.png to {out}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=Path, default=RUNS_DIR, help="runs directory (default evals/runs)")
    parser.add_argument("--out", type=Path, default=RUNS_DIR.parent / "site", help="output directory (default evals/site)")
    parser.add_argument(
        "--base-url",
        default="https://wseaton.github.io/CoasterBench",
        help="public URL the site deploys to; required for Slack unfurl images, "
        "which must be absolute URLs (default %(default)s)",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="render every run; by default only the newest run per mode is shown",
    )
    args = parser.parse_args()

    all_runs = load_runs(args.runs)
    runs = all_runs
    if not args.all:
        # load_runs returns newest-first; keep only the latest run per mode.
        newest: dict[str, str] = {}
        runs = [r for r in all_runs if newest.setdefault(r.mode, r.name) == r.name]
    build_site(runs, args.out, args.runs, args.base_url)
    # Superseded run pages have been shared as links; keep their URLs alive
    # with a redirect to the current board for the same mode.
    kept = {r.name for r in runs}
    latest_by_mode = {r.mode: r.name for r in runs}
    stubs = 0
    for run in all_runs:
        if run.name in kept:
            continue
        target = f"run-{latest_by_mode[run.mode]}.html" if run.mode in latest_by_mode else "index.html"
        (args.out / f"run-{esc(run.name)}.html").write_text(
            "<!doctype html><html><head><meta charset=\"utf-8\">"
            f"<meta http-equiv=\"refresh\" content=\"0; url={target}\">"
            f"<link rel=\"canonical\" href=\"{target}\">"
            f"<title>Run {esc(run.name)} (superseded)</title></head>"
            f"<body><p>Run {esc(run.name)} has been superseded. "
            f"<a href=\"{target}\">Latest {esc(run.mode)} board &rarr;</a></p></body></html>"
        )
        stubs += 1
    if not args.base_url:
        print(
            "note: no --base-url given, og:image/og:url omitted (Slack unfurls will be text-only)",
            file=sys.stderr,
        )
    pages = 1 + len(runs)
    print(f"wrote {pages} page(s) for {len(runs)} run(s) (+{stubs} redirect stub(s)) to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
