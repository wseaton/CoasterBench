# OpenRCT2 Fork: Rust Agentic Eval Harness

This is a fork of OpenRCT2 (github.com/wseaton/OpenRCT2) with one goal: a Rust
plugin system that runs an agentic eval harness through the game. The flagship
eval: an LLM designs the best roller coaster, scored by in-game ratings.

## Fork rules

- We rebase onto upstream `develop` regularly. Almost all new code lives in new
  directories; edits to existing OpenRCT2 files must stay tiny and mechanical
  (single call sites, not refactors).
- New C++ files under `src/openrct2/` need no CMake edits (GLOB_RECURSE picks
  them up).

## Architecture

- Rust attaches as a **staticlib linked into the binary** (not dlopen, not IPC).
  Crate lives in `rust/orct2-agent`, bridged via cbindgen C ABI.
- C++ side of the bridge goes in `src/openrct2/rustbridge/` with a
  `NativeHookEngine` (the existing `scripting/HookEngine.h` Hook is
  QuickJS-specific and gated on ENABLE_SCRIPTING; we stay independent of it).
- Eval entry point: `coasterbench-cli eval` command, modeled on
  `src/openrct2/command_line/SimulateCommands.cpp` (load park, tick N via
  `gameStateUpdateLogic()`, score).
- Mutations go through `GameActions::Execute` only, never direct game state
  pokes (same validation path as JS plugins).

## Key facts (verified in code)

- `gOpenRCT2Headless` and `gOpenRCT2NoGraphics` are separate flags. The eval
  command needs Headless=true, NoGraphics=false so the software renderer works:
  `CaptureImage()` (`src/openrct2/interface/Screenshot.cpp`) renders PNGs with
  no window/GPU, but asserts if sprite data wasn't loaded.
- Ride ratings hook site: `src/openrct2/ride/RideRatings.cpp` (~line 1076),
  next to the JS `rideRatingsCalculate` hook.
- Replay video = fixed-camera frames + external ffmpeg encode in the harness.
  No video encoder in-tree, keep it that way. The camera is the track's
  bounding box (the same crop as park.png) and never moves, so consecutive
  frames differ only where the train is, which is what makes the encode cheap.
  A chase cam would move every pixel and defeat that.
- Determinism/repro: `ReplayManager.h`, `GameStateSnapshots.h` exist upstream.

## Rust bridge workflow

- ABI changes in `rust/orct2-agent/src/lib.rs` (any `#[no_mangle] extern "C"`
  item) require regenerating the checked-in header:
  `./scripts/generate-rust-header.sh` (needs `cargo install cbindgen`).
- Corrosion (pinned v0.6.1, FetchContent in `cmake/rustagent.cmake`) builds the
  crate; `ENABLE_RUST_AGENT` (default ON) gates everything, same pattern as
  ENABLE_SCRIPTING.
- Rust checks: `cargo fmt && cargo clippy --all --benches --tests --examples
  --all-features && cargo test` inside `rust/orct2-agent`.

## Build (macOS)

```bash
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=RelWithDebInfo
cmake --build build
```

CMake auto-downloads prebuilt universal dylibs on macOS (MACOS_USE_DEPENDENCIES).
Note `LIBOPENRCT2_LINKAGE STATIC` on APPLE in `src/openrct2/CMakeLists.txt` (RTTI
across dylib boundary is broken on macOS, so libopenrct2 links static there).

## Running the CLI from the build dir

Non-bundled binaries look for `data/` next to the exe. One-time setup:
`ln -sf $PWD/build/OpenRCT2.app/Contents/Resources build/data`. Bare
`coasterbench-cli` (launch mode) sets NoGraphics and works without RCT2 assets;
`simulate` (and our `eval`) load g1.dat and need the real game files.

## Eval workflow (phase 3+)

- Track programs: JSON (ride_type, start tile+dir, piece list) executed by
  `rust/orct2-agent/src/program.rs`; piece vocabulary in `pieces.rs` mirrors
  `src/openrct2/ride/ted/TrackElemType.h`. Example: `evals/programs/test_oval.json`.
- Full run: `./build/coasterbench-cli eval <scenario> --ticks 25000
  --rct2-data-path ~/rct2-assets --program p.json --out report.json --capture park.png`
- Entrance/exit are brute-force auto-placed next to station tiles (game
  validation is the oracle). Testing status requires them.
- Direction semantics: dir 0 faces -x, 1 +y, 2 +x, 3 -y. Cursor z step 16.
- Placement is NOT a sufficient oracle for drawability: TrackPlaceAction never
  checks the ride's enabled track groups (that gate only feeds the construction
  window), so pieces the ride style has no artwork for build and rate fine and
  render as nothing (wooden RC + 1-tile flat<->60 transitions is the classic
  case). `CheckPieceDrawable` in RustBridge.cpp gates place and query against
  the renderer's own dispatch (GetTrackPaintFunction vs TrackPaintFunctionDummy).
  See issue #1 and the readme implementation notes.
- Stalls never get ratings (RatingsCalculationType::Stall); tracked rides need
  a completed test circuit (RideFlag::tested) before ratings compute.
- Save-park artifact: `coasterbench-cli eval ... --save-park <path>` writes a
  .park (~55 KB) after the tick loop, and coaster-bench collects one per round
  as `round_N/park.park`. Round-trip verified: reloading a saved park gives the
  same ride, rating and circuit audit. Both paths go through
  `RustBridge::SavePark`, which packs the loaded objects the way the crash
  handler's save does, so a park using non-standard objects still reopens.
  - `save_park` writes a host filesystem path, so it lives on the loopback
    control listener only (see the MCP section), never in the agent toolset.
  - `evals/runs/**/*.park` is gitignored like the other heavy artifacts.
- Replay video: the control tool `capture_replay(dir, frames, every_ticks, zoom)`
  ticks the running ride and writes cropped PNG frames; coaster-bench encodes
  them with ffmpeg into `round_N/replay.mp4` (libx264, crf 28, `-g` = frame
  count so the clip has a single keyframe) and deletes the frames. Off with
  `--replay-seconds 0`; 20s by default. The site embeds it with park.png as the
  poster.
  - Length comes from the game's own measured lap ("Ride time" =
    `Ride::getTotalTime()`, the sum of the stations' SegmentTime, in seconds)
    plus a 3s tail, bounded by `--replay-seconds` (90 default). Report.json
    carries it as `ride_time`. Do not guess: the test oval laps in 56s, so the
    old fixed 20s cut it off two thirds in. A ride that never tested has no
    measured time and falls back to the cap.
  - Measured at 672x432, 20fps, zoom 0: 4 ms and 29 KB per frame to capture.
    20s encodes to 255 KB, a full 59s lap to 577 KB (1180 frames, 4.5s to
    capture). Tripling the length costs 2.3x the bytes, so length is cheap; the
    PNG intermediate (37 MB for that lap) is transient.
  - ffmpeg comes from brew and is not vendored; a missing binary logs and skips
    the video rather than failing the round.
- Circuit audit: `orct2_host_circuit_stats` walks a ride with the game's own
  TrackCircuitIterator (what findTrackGap uses), seeded from the station origin
  element, and report.json gains per-ride
  `circuit: {walked_pieces, total_pieces, orphan_pieces, looped}`. Ratings only
  witness that the ridden loop closes and completes, so orphans (track placed
  but joined to nothing) are the gap this closes; the site shows a green
  "verified circuit" chip at 0 orphans, amber otherwise.
  - Only sequence-0 elements count on both sides. A multi-tile piece has an
    element per tile but the walk visits it once, so counting raw elements
    invents orphans.
  - The walk starts at the station's *departure* element and runs forward, so
    an unclosed track needs the backward sweep too, otherwise the pieces behind
    the station read as stranded (a plain station + straight measured 6 of 8).
  - Verified live: closed oval 16/16/0 looped, open track 14/14/0 not looped,
    station+straight 8/8/0. The orphan>0 path has no live repro (the executor
    no longer strands track), only unit coverage in coaster-site.
- Head-to-head driver: `uv run evals/driver.py` (needs ANTHROPIC_API_KEY, or
  `--vertex` with GCP ADC; project defaults from $ANTHROPIC_VERTEX_PROJECT_ID);
  results under `evals/runs/<timestamp>/`. Models get a validate_track_program
  dry-run tool (placement + closure errors with a net-displacement hint) with
  a shared per-round budget alongside library lookups.
- Sub-lane orchestrator: `rust/coaster-bench` (own crate, not linked into the
  game). Spawns the game MCP server, then one sandboxed Claude Code session
  per model per round (OpenShell sandbox `coaster-sub`, personal-sub OAuth via
  the `claude-sub` provider) building interactively through the MCP tools;
  collects report/program/park.png per round into the same evals/runs layout.
  `./rust/coaster-bench/target/release/coaster-bench --models claude-fable-5
  --rounds 4 --ride-type 51 --name my-run`. Port must be in the sandbox
  policy (default 8791). `--open-note` stages upstream's tree (git archive of
  `git merge-base HEAD origin/develop`, cached under $TMPDIR by revision) into
  the sandbox at /tmp/openrct2-src via `openshell sandbox upload`
  (exec stdin caps at 4 MiB; upload nests the local dir under its dest, so the
  staging dir is named openrct2-src and uploaded to /tmp), then chmod -R a-w.
  `--fresh-sandbox` creates `<base>-<hex id>` per run from
  rust/coaster-bench/sandbox/{Dockerfile,policy.yaml} (recovered 2026-07-25 from
  the running image; the originals were lost) and deletes it on the way out, so
  no agent state survives a run. Gateway caps sandbox names at 19 chars;
  `sandbox create` attaches and never returns, but the sandbox outlives the
  client, so the harness spawns it, polls readiness, then drops the client.
  `sandbox exec` stdin caps at 4 MiB and deletes are eventually consistent.
  SIGINT/SIGTERM/SIGHUP set a flag (signal-hook) that the round loop and the
  session poll loop check, so the agent is killed, the sandbox swept and the
  game server dropped instead of orphaned; a run refuses to start if its port
  is already serving (use --attach deliberately).
  Records open_note + open_note_source in run.json; the site labels those runs
  "<mode> + open note". RideRatings.cpp is unmodified in this fork, so upstream
  source is a faithful oracle for scoring.
- Second coaster-bench lane: `--models opencode:openrouter/<author>/<model>`
  runs opencode in the `coaster-or` sandbox against OpenRouter (key in the
  login keychain as `openrouter-api-key`, cost tracked by spend delta).
  coaster-bench writes that sandbox's ~/.config/opencode/opencode.json per
  session, so the MCP endpoint is per contender. Model input modalities come
  from the OpenRouter catalogue and are recorded in run.json; a text-only
  model gets `?modalities=text` and a prompt with no screenshot tool.
- Third coaster-bench lane: `--models codex:<model>` runs codex in the
  `codex-arena` sandbox on the ChatGPT subscription (`codex-oauth` provider),
  and `--models codex:openrouter/<author>/<model>` runs the same harness against
  OpenRouter instead. coaster-bench writes `$CODEX_HOME` (config.toml, and
  auth.json on the subscription backend) per session, so the MCP endpoint,
  model and credentials are all per contender.
  - `$HOME` is `/sandbox` in that image, not `/home/sandbox` like the other two
    lanes, and codex refuses a CODEX_HOME under /tmp. Keep the paths
    `$HOME`-relative.
  - Auth: the gateway injects `CODEX_AUTH_*` as `openshell:` placeholders the
    egress proxy substitutes. The access and refresh tokens stay placeholders;
    the id token and account id are copied from the host's `~/.codex/auth.json`,
    because codex decodes the JWT locally ("invalid ID token format" otherwise).
    `last_refresh` is stamped at session start so codex never tries a refresh,
    which would write a real token into the sandbox.
  - Codex has no per-tool allowlist, so open note reaches it as
    `sandbox_mode = "danger-full-access"` (OpenShell is the real jail) versus
    `read-only` for a blind run, recorded as `codex_sandbox_mode` in run.json.
    A blind codex round still has a shell, unlike the other two lanes.
  - The OpenRouter backend needs `wire_api = "responses"` (codex 0.145 dropped
    "chat") and needs `/home/sandbox/bin/codex` in the *openrouter provider
    profile's* binary list: a provider-injected policy owns its own hosts, so
    `codex-policy.yaml` cannot grant openrouter.ai on its own. The edited
    profile is checked in at `rust/coaster-bench/sandbox/openrouter-profile.yaml`.
  - Build the image for the host arch (`podman build --platform linux/arm64`):
    the OpenShell base is multi-arch, and an amd64 image under Rosetta dies at
    sandbox start with "Unable to open /proc/self/exe".
  - Cheap smoke of the whole lane (one round, dollars not cents):
    `--models codex:openrouter/openai/gpt-5-mini --rounds 1 --open-note
    --fresh-sandbox --name codex-smoke`. Model support for codex's built-in
    tools varies on OpenRouter and shows up as a 400 on the first turn:
    gpt-5-nano and gpt-5-mini negotiate the toolset, gpt-5.4-nano refuses
    (`Tool 'tool_search' is not supported`). The subscription lane can be
    smoked for free while quota is out (`--models codex:gpt-5.6-sol
    --codex-sandbox codex-arena`): reaching the usage-limit error proves the
    auth file and egress substitution worked.
- Two driver modes (`--mode`, recorded in run.json + standings.json, separate
  leaderboard sections in the site): `design` (from scratch) and `library`
  (model can search the stock .TD6 library via extra tools; tests retrieval +
  adaptation). Both modes penalize similarity to stock designs: report.json
  carries the nearest-design similarity (edit distance + longest common
  substring, mirrored variants included; rust similarity.rs), and scores scale
  linearly to zero above the grace threshold (driver's SIMILARITY_GRACE,
  recorded per run in run.json; site reads it from there). Library JSON dump:
  `coasterbench-cli eval <scenario> --rct2-data-path ... --dump-library lib.json`;
  preview PNGs (game's own TrackDesignDrawPreview, 370x217, needs a park
  loaded): `... --render-library <dir>`, cached at evals/library-previews/
  (gitignored, auto-rendered by the driver in library mode).
- Library lookups are persisted per round as `round_N/lookups.json` (written
  by driver.py, but any harness can drop the same file); the site shows
  per-round lookup chips, a "studied designs" preview gallery per model, and
  a full library.html gallery.
- Heavy artifacts (PNGs, videos) are gitignored, not tracked; only run JSON
  is. After a run, `uv run evals/publish.py` uploads them to the Cloudflare
  R2 bucket `coasterbench` (public at https://artifacts.wseaton.com) through
  the local `wrangler login` OAuth session (no static keys anywhere) and
  records manifests (`runs/<run>/artifacts.json`, `evals/library-previews.json`).
  The SSG prefers local files, falls back to manifest URLs, so the GitHub
  Pages CI build works from JSON alone. **Both must be committed and pushed**
  or the deployed site renders with no images at all.

## Site generator (rust/coaster-site)

- `cargo run --manifest-path rust/coaster-site/Cargo.toml` writes `evals/site/`
  (own crate, askama templates in `templates/`, CSS/JS in `static/`, raster
  work — index thumbnails, og-card, favicon — via the `image` crate). This is
  what the Pages workflow builds. The old `evals/site.py` is gone; this crate
  is the only site generator.
- Three page types: `index.html` (one row per model per run, runs pivoted,
  grouped by run, faceted by mode/coaster/harness/model), `run-<run>.html`
  (models side by side for comparison), and `run-<run>-<model>.html` (one
  model's whole run in detail: headline stats, chart, studied designs, every
  round). Index rows link to the model page, the run name to the comparison.
- Runs that never finished are skipped, with the reason on stderr and a note
  on the page: any model short of run.json's promised `models`/`rounds` (or,
  for runs from before run.json recorded them, short of the run's own best
  round count). `--include-partial` renders them anyway.

## MCP server (interactive per-piece building)

- `./build/coasterbench-cli eval <scenario> --rct2-data-path ~/rct2-assets
  --serve 8791` runs an MCP server at `http://127.0.0.1:8791/mcp`
  (streamable HTTP, JSON response mode).
- Hand-rolled server in `rust/orct2-agent/src/mcp.rs`, no async runtime. Socket
  I/O runs on a thread per connection; every request crosses one mpsc channel to
  the game thread, which owns the session and is the only thread that may call
  `host::*` (the logger included, so connection threads never log). Requests
  still execute serially in arrival order, so determinism is unchanged. Before
  this the accept loop lived on the game thread and a client that connected then
  went silent hung the server for everyone.
- Two listeners, and the one a request arrives on chooses its tool table:
  `--serve` (bind 0.0.0.0 for the sandboxes) serves the agent tools, and
  `--serve-control <port>` binds 127.0.0.1 only and serves the harness's tools
  (`save_park`). A control tool is therefore absent from the agent's table
  rather than filtered out of it, and the listener is not routable from a
  container at all. coaster-bench defaults to `--control-port 8792` and only
  ever calls save_park there. Tools: new_ride, place_piece, place_pieces (batch placement, max 200
  pieces), valid_next_pieces (game-as-oracle query of the whole catalog),
  get_state, finish_and_test, screenshot (MCP image content), demolish,
  search_track_designs + get_track_design (stock .TD6 library browsing;
  recorded ratings deliberately hidden).
- Cursor state now tracks bank (TrackRoll: 0=none, 2=left, 4=right, 15=upside_down)
  and slope (TrackPitch: 0=none, 2=up25, 4=up60, 6=down25, 8=down60); circuit
  closure check compares full cursor (x/y/z/dir/bank/slope) to catch incomplete
  circuits that re-enter the station still banked or sloped.
- Clients advertise what content they can take in the request target:
  `/mcp?modalities=text,image` (vocabulary lifted from OpenRouter's
  `input_modalities`; absent = everything). Tools answering outside the set are
  dropped from tools/list and refused on call, so a text-only model never sees
  screenshot — one image content block fails the whole upstream request
  (Poolside answers "please check the model you provided").
- Register with Claude Code:
  `claude mcp add --transport http coaster http://127.0.0.1:8791/mcp`
- Rust unit tests link against `#[cfg(test)]` stubs in host.rs (the C++ host
  only exists in the game binary); game behavior is tested e2e via the CLI.

## OpenShell sandbox harness (containerized Claude Code + one MCP)

Docs live in ~/git/OpenShell/docs — READ THEM before changing anything here.
- Gateway: `openshell-gateway --config ~/.config/openshell/gateway.toml`
  (podman driver). Client mTLS certs live in
  ~/.config/openshell/gateways/<name>/mtls and must match the server PKI in
  ~/.local/state/openshell/tls (copy ca.crt + client/tls.* on BadSignature).
- Sandbox image (scratchpad/coaster-sandbox/Dockerfile): node:22-slim +
  iproute2 (required for netns isolation) + `useradd sandbox` (required by
  installed 0.0.88; newer versions accept bare UIDs). Image CMD is replaced by
  the supervisor — pass start commands via `-- <cmd>` or `sandbox exec`.
- Game server must bind 0.0.0.0 (`--serve-bind 0.0.0.0`); sandboxes reach the
  host at host.containers.internal.
- Policy (scratchpad/coaster-sandbox/policy.yaml): vertex endpoints + game MCP
  only. Binary attribution uses kernel-resolved paths — Claude Code 2.x is
  `/usr/local/lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe`, not
  node. Denials log to `openshell logs <sandbox>` with binary + reason.
- Inference: gateway-managed vertex provider (`--from-gcloud-adc` +
  VERTEX_AI_PROJECT_ID/REGION/PUBLISHER config). inference.local rejects
  Claude Code's `context_management` param (Vertex schema), so the working
  path is Claude Code vertex mode + placeholder substitution:
  `CLAUDE_CODE_USE_VERTEX=1 CLAUDE_CODE_SKIP_VERTEX_AUTH=1
  ANTHROPIC_AUTH_TOKEN="$GOOGLE_VERTEX_AI_TOKEN"` — the proxy resolves the
  placeholder to the gateway-minted token at egress; no real creds in sandbox.

## Runtime assets

RCT2 asset files are required at **runtime** (even headless park loading), not
compile time. Source: GOG installer + innoextract (preferred on macOS) or RCT
Classic. Point `general.game_path` in the OpenRCT2 config at them.
