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
  bundled objects, checked-in scenario; ~240 MB). Build once, push, pin the
  digest. Verified on Linux arm64 (podman): the containerized MCP server
  builds, tests, and rates a coaster with ratings identical to the macOS
  build. x86_64 not yet exercised.
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

## Field notes: what the first live runs caught (vLLM 0.25.1, A100)

Three findings from the eval's first day out, all now handled by the driver —
kept here because they are exactly the failure classes this job exists to
surface, and the first two are worth checking against vLLM upstream:

1. **`tool_choice: "required"` returned zero tool calls** on the server's
   first-ever structured-output request after startup (Qwen2.5-7B-Instruct,
   hermes parser). A contract violation; not reproduced in 15 identical
   probes afterwards, so it looks cold-start-related. The driver retries up
   to 3× rather than failing the run.
2. **Reasoning models silently starve the tool call.** Laguna-S-2.1
   (`poolside_v1` tool + reasoning parsers, thinking enabled) spent the
   entire completion budget on interleaved thinking and hit
   `finish_reason: "length"` with `tool_calls: []` — surfacing, misleadingly,
   as another `required` violation. Any agentic harness with a fixed
   `max_tokens` sized for non-reasoning models hits this. The driver fails
   fast with the real cause and takes `--max-tokens` (Laguna wants ~24000).
3. **`generation_config.json` overrides server sampling defaults** (vLLM
   warns but serves): Qwen shipped temp 0.7 / top-p 0.8 / rep-penalty 1.05,
   so "default" runs are not the sampling you assumed. Pin
   `--generation-config vllm` if you want vLLM defaults.
