# CoasterBench in CI (no RCT2 assets)

Everything the eval scores — park loading, track placement, ride testing,
ratings — runs without any RollerCoaster Tycoon 2 files via `--no-graphics`
(objects come from the bundled JSON pack; only pixels need `g1.dat`). That
makes the whole benchmark legally shippable in a CI image. What you give up:
screenshots (feedback is the eval report only, so contenders run text-only)
and the stock TD6 library (library mode is unavailable and the similarity
penalty is inert, so scores are not comparable with asset-full leaderboard
runs — `run.json` records `no_graphics: true`).

## Pieces

- `Dockerfile` — CPU-only game image (headless `openrct2-cli`, Rust agent,
  bundled objects, checked-in scenario). Build once, push, pin the digest.
  Draft: authored on macOS, not yet exercised on a Linux builder.
- `check_run.py` — pass/fail gate: every model needs ≥1 round that built and
  completed a test circuit. Scores are metrics, not assertions (models are
  nondeterministic; don't gate merges on excitement).

## Shape of a job against vLLM

Nightly / non-blocking is the realistic tier — the wall clock is model
inference, the game sim is ~7s per 25k-tick round.

```bash
# 1. serve the model under test (own container/step; needs the GPU)
vllm serve "$MODEL" --enable-auto-tool-choice --tool-call-parser hermes &

# 2. game + driver are CPU-only
uv run evals/driver.py \
    --base-url http://localhost:8000/v1 \
    --models "$MODEL" --rounds "${ROUNDS:-2}" --no-graphics \
    --name "ci-${BUILD_NUMBER:-local}"

# 3. gate on protocol success, keep the run dir as the artifact
python3 evals/ci/check_run.py evals/runs/*-"ci-${BUILD_NUMBER:-local}"
```

What this exercises end-to-end that parser unit tests don't: forced and named
`tool_choice` (guided decoding), multi-turn tool loops with tool_result
round-trips, and large JSON tool arguments (a 148-piece track program is a
few KB of structured output).

There is also a zero-GPU smoke: replay a canned program with no model at all —
`openrct2-cli eval test/tests/testdata/parks/BigMapTest.sv6 --no-graphics
--ticks 25000 --program evals/programs/test_oval.json --out report.json`
must produce `program.ok == true` and a tested ride.
