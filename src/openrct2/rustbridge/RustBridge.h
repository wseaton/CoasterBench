/*****************************************************************************
 * Fork-only file (wseaton/OpenRCT2): C++ side of the Rust agent bridge.
 * See rust/orct2-agent for the Rust side; orct2_agent.h is cbindgen output.
 *****************************************************************************/

#pragma once

#ifdef ENABLE_RUST_AGENT

    #include <cstdint>
    #include <string_view>

struct Orct2ProgramOutcome;

namespace OpenRCT2::RustBridge
{
    // Calls orct2_agent_init(). Safe to call when the agent is disabled at
    // runtime; logs and carries on if the Rust side reports failure.
    void Initialise();

    void Tick(uint32_t tick);

    // Asks the Rust agent to log its end-of-eval ride summary.
    void EvalSummary();

    // Executes a JSON track program file; the returned handle is owned by the
    // Rust side and must be passed to EvalFinish exactly once.
    Orct2ProgramOutcome* RunProgram(const char* path);

    // Writes the JSON eval report (consumes the outcome; either may be null).
    int32_t EvalFinish(Orct2ProgramOutcome* outcome, const char* outPath);

    // Park screenshot to a PNG path; fitTrack crops to the track's bounding
    // box (full map when no track exists). Returns 0 on success.
    int32_t Capture(const char* path, int32_t zoom, uint8_t rotation, bool fitTrack, bool xray);

    // Films the park's coaster into an mp4 (ffmpeg encodes), one station-to-
    // station cycle, bounded by maxSeconds. Advances the simulation, so call it
    // after the report and any screenshot. Returns 0 on success.
    int32_t CaptureReplay(const char* path, uint32_t maxSeconds, int32_t zoom);

    // Films the track program at programPath being assembled piece by piece
    // into an mp4, in the colours reportPath records (may be null). Demolishes
    // and rebuilds the ride, so it must run last. Returns 0 on success.
    int32_t CaptureBuildMontage(const char* path, const char* programPath, const char* reportPath, int32_t zoom);

    // Films a whole run: every round in the manifest built in order on one
    // camera (the union of their footprints), ending held on the last. Builds
    // everything itself, so it wants a park with no track of its own.
    // Returns 0 on success.
    // lapPath (may be null) additionally films the last round's lap on the very
    // same camera, so the two clips can run back to back without the coaster
    // shifting at the cut.
    int32_t CaptureEvolution(
        const char* path, const char* manifestPath, const char* lapPath, uint32_t lapSeconds, int32_t zoom);

    // Films a round's recorded session: the tool calls the game accepted, in
    // order, demolitions and all. Builds everything itself, so it wants a park
    // with no track of its own. Returns 0 on success.
    int32_t CaptureTraceMontage(const char* path, const char* actionsPath, int32_t zoom);

    // Applies a round's recorded name and colours (report.json) to the built
    // ride, so a rerun's artifacts look like the round's. Returns 0 on success,
    // and on a report that records no styling.
    int32_t ApplyPresentation(const char* reportPath);

    // Writes the park as a .park save, the artifact that lets a result be
    // reopened and checked instead of taken on trust. Returns true on success.
    bool SavePark(std::string_view path);

    // Runs the MCP server on bind:port (null bind = 127.0.0.1) and, when
    // controlPort is non-zero, a loopback-only control server beside it for the
    // harness. Blocks the game thread until the process exits.
    int32_t Serve(const char* bind, uint16_t port, uint16_t controlPort);

    // Writes the stock track design library as JSON for the eval driver's
    // library mode. Returns 0 on success.
    int32_t DumpLibrary(const char* path);

    // Renders a preview PNG (370x217, rotation 0) of every stock track design
    // into outDir, named after the design. Needs a park loaded (the preview
    // renderer stashes and restores the live map). Returns 0 on success.
    int32_t RenderTrackLibrary(const char* outDir);
} // namespace OpenRCT2::RustBridge

#endif // ENABLE_RUST_AGENT
