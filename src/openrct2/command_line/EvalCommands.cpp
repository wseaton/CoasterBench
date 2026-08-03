/*****************************************************************************
 * Fork-only file (wseaton/OpenRCT2): `coasterbench-cli eval` command.
 *
 * Loads a park headless, runs the game loop for a fixed number of ticks with
 * the Rust agent bridge hooked in, then asks the agent for its summary.
 * Modeled on SimulateCommands.cpp, with two differences: options for data
 * paths (root options are not parsed for subcommands), and NoGraphics stays
 * false so the software renderer is available for screenshot capture.
 *****************************************************************************/

#ifdef ENABLE_RUST_AGENT

    #include "../Context.h"
    #include "../GameState.h"
    #include "../OpenRCT2.h"
    #include "../core/Console.hpp"
    #include "../core/Path.hpp"
    #include "../ride/Ride.h"
    #include "../ride/RideManager.hpp"
    #include "../ride/RideRatings.h"
    #include "../rustbridge/RustBridge.h"
    #include "CommandLine.hpp"

    #include <memory>

using namespace OpenRCT2::CommandLine;

namespace OpenRCT2
{
    static int32_t _ticks = 10000;
    static u8string _evalOpenRCT2DataPath{};
    static u8string _evalRCT2DataPath{};
    static u8string _programPath{};
    static u8string _reportPath{};
    static u8string _capturePath{};
    static int32_t _servePort = 0;
    static int32_t _serveControlPort = 0;
    static u8string _serveBind{};
    static u8string _dumpLibraryPath{};
    static u8string _renderLibraryDir{};
    static u8string _saveParkPath{};
    static u8string _replayPath{};
    static int32_t _replaySeconds = 90;
    static u8string _buildMontagePath{};
    static u8string _presentationPath{};
    static u8string _evolutionPath{};
    static u8string _evolutionManifest{};
    static u8string _evolutionLapPath{};
    static u8string _traceMontagePath{};
    static u8string _traceActionsPath{};
    static bool _captureAllRotations = false;
    static bool _captureXray = false;

    // clang-format off
    static constexpr CommandLineOptionDefinition kEvalOptions[]
    {
        { CMDLINE_TYPE_INTEGER, &_ticks,                kNAC, "ticks",              "number of game ticks to simulate (default 10000)"       },
        { CMDLINE_TYPE_STRING,  &_evalOpenRCT2DataPath, kNAC, "openrct2-data-path", "path to the OpenRCT2 data directory"                    },
        { CMDLINE_TYPE_STRING,  &_evalRCT2DataPath,     kNAC, "rct2-data-path",     "path to the RollerCoaster Tycoon 2 install directory"   },
        { CMDLINE_TYPE_STRING,  &_programPath,          kNAC, "program",            "JSON track program to build before simulating"          },
        { CMDLINE_TYPE_STRING,  &_reportPath,           kNAC, "out",                "write a JSON eval report to this path"                  },
        { CMDLINE_TYPE_STRING,  &_capturePath,          kNAC, "capture",            "write a giant park screenshot (PNG) to this path"       },
        { CMDLINE_TYPE_INTEGER, &_servePort,            kNAC, "serve",              "run the MCP server on this port instead of a batch eval" },
        { CMDLINE_TYPE_STRING,  &_serveBind,            kNAC, "serve-bind",         "MCP server bind address (default 127.0.0.1; use 0.0.0.0 for containers)" },
        { CMDLINE_TYPE_INTEGER, &_serveControlPort,     kNAC, "serve-control",      "loopback-only control port for the harness (save_park); off when 0"      },
        { CMDLINE_TYPE_STRING,  &_dumpLibraryPath,      kNAC, "dump-library",       "write the stock track design library as JSON to this path and exit"      },
        { CMDLINE_TYPE_STRING,  &_renderLibraryDir,     kNAC, "render-library",     "render a preview PNG of every stock track design into this directory and exit" },
        { CMDLINE_TYPE_STRING,  &_saveParkPath,         kNAC, "save-park",          "write the finished park as a .park save to this path"   },
        { CMDLINE_TYPE_STRING,  &_replayPath,           kNAC, "replay",             "film the ride into this .mp4 (needs ffmpeg on PATH)"     },
        { CMDLINE_TYPE_INTEGER, &_replaySeconds,        kNAC, "replay-seconds",     "cap on --replay length; a clip normally ends at the ride's next departure (default 90)" },
        { CMDLINE_TYPE_STRING,  &_buildMontagePath,     kNAC, "build-montage",      "film --program being built piece by piece into this .mp4 (needs ffmpeg on PATH)" },
        { CMDLINE_TYPE_STRING,  &_presentationPath,     kNAC, "presentation",       "a round's report.json: restyle the built ride in the name and colours it recorded" },
        { CMDLINE_TYPE_STRING,  &_evolutionPath,        kNAC, "evolution-montage",  "film a whole run's rounds in order into this .mp4 (needs --evolution)"   },
        { CMDLINE_TYPE_STRING,  &_evolutionManifest,    kNAC, "evolution",          "JSON array of {program, report} paths, oldest round first"              },
        { CMDLINE_TYPE_STRING,  &_evolutionLapPath,     kNAC, "evolution-lap",      "with --evolution-montage, also film the last round's lap on the same camera" },
        { CMDLINE_TYPE_STRING,  &_traceMontagePath,     kNAC, "trace-montage",      "film a round's recorded session into this .mp4 (needs --trace-actions)"  },
        { CMDLINE_TYPE_STRING,  &_traceActionsPath,     kNAC, "trace-actions",      "JSON array of accepted tool calls distilled from a round's trace.jsonl" },
        { CMDLINE_TYPE_SWITCH,  &_captureAllRotations,  kNAC, "capture-all-rotations", "with --capture, also write the other three view rotations as <name>-r1/-r2/-r3.png" },
        { CMDLINE_TYPE_SWITCH,  &_captureXray,          kNAC, "capture-xray",       "with --capture, also write a see-through verification view (terrain and supports hidden, every placed piece visible) as <name>-x.png" },
        kOptionTableEnd
    };

    static ExitCode HandleEval(CommandLineArgEnumerator* argEnumerator);

    const CommandLineCommand CommandLine::kEvalCommands[]{
        DefineCommand("", "<park file> [--ticks N]", kEvalOptions, HandleEval),
        kCommandTableEnd
    };
    // clang-format on

    static ExitCode HandleEval(CommandLineArgEnumerator* argEnumerator)
    {
        const utf8* inputPath;
        if (!argEnumerator->TryPopString(&inputPath))
        {
            Console::Error::WriteLine("Expected a park/save/scenario file path");
            return ExitCode::fail;
        }

        if (_ticks <= 0)
        {
            Console::Error::WriteLine("--ticks must be positive");
            return ExitCode::fail;
        }

        if (!_evalOpenRCT2DataPath.empty())
        {
            gCustomOpenRCT2DataPath = Path::GetAbsolute(_evalOpenRCT2DataPath);
        }
        if (!_evalRCT2DataPath.empty())
        {
            gCustomRCT2DataPath = Path::GetAbsolute(_evalRCT2DataPath);
        }

        // Headless, but keep graphics data loaded (gOpenRCT2NoGraphics stays
        // false) so CaptureImage can render screenshots of the result.
        gOpenRCT2Headless = true;

        std::unique_ptr<IContext> context(CreateContext());
        if (!context->Initialise())
        {
            Console::Error::WriteLine("Context initialization failed.");
            return ExitCode::fail;
        }

        if (!_dumpLibraryPath.empty())
        {
            // Standalone mode: the library scan only needs the data paths, so
            // skip park loading and simulation entirely.
            return RustBridge::DumpLibrary(_dumpLibraryPath.c_str()) == 0 ? ExitCode::ok : ExitCode::fail;
        }

        if (!context->LoadParkFromFile(inputPath))
        {
            return ExitCode::fail;
        }

        if (!_renderLibraryDir.empty())
        {
            // Standalone mode: the preview renderer needs a live map to stash,
            // but no simulation.
            return RustBridge::RenderTrackLibrary(_renderLibraryDir.c_str()) == 0 ? ExitCode::ok : ExitCode::fail;
        }

        if (_servePort > 0)
        {
            // Interactive mode: the MCP server owns the game loop from here.
            // Blocks until the process is terminated.
            return RustBridge::Serve(
                       _serveBind.empty() ? nullptr : _serveBind.c_str(), static_cast<uint16_t>(_servePort),
                       static_cast<uint16_t>(_serveControlPort))
                    == 0
                ? ExitCode::ok
                : ExitCode::fail;
        }

        Orct2ProgramOutcome* outcome = nullptr;
        if (!_programPath.empty())
        {
            outcome = RustBridge::RunProgram(_programPath.c_str());
        }

        // Before the ticks, so every artifact downstream shows the ride in the
        // colours the round banked. A program carries no colours, so a rerun of
        // a styled round is stock gold without this.
        if (!_presentationPath.empty() && RustBridge::ApplyPresentation(_presentationPath.c_str()) != 0)
        {
            Console::Error::WriteLine("Presentation could not be applied.");
            return ExitCode::fail;
        }

        Console::WriteLine("Running eval for %d ticks...", _ticks);
        for (int32_t i = 0; i < _ticks; i++)
        {
            gameStateUpdateLogic();
            RustBridge::Tick(static_cast<uint32_t>(i));
        }

        // The in-game ratings state machine only processes a few rides per
        // tick; force the remainder synchronously so the summary is complete.
        for (auto& ride : RideManager(getGameState()))
        {
            if (ride.status != RideStatus::closed && ride.ratings.isNull())
            {
                RideRating::UpdateRide(ride);
            }
        }
        RustBridge::EvalSummary();

        auto exitCode = ExitCode::ok;
        if (!_reportPath.empty() || outcome != nullptr)
        {
            if (RustBridge::EvalFinish(outcome, _reportPath.empty() ? nullptr : _reportPath.c_str()) != 0
                && !_reportPath.empty())
            {
                exitCode = ExitCode::fail;
            }
        }
        // A screenshot shows the coaster; the save *is* the coaster. It is the
        // only artifact that lets a result be reopened later and checked
        // against a claim, rather than believed.
        if (!_saveParkPath.empty())
        {
            if (RustBridge::SavePark(_saveParkPath))
            {
                Console::WriteLine("Park saved to %s", _saveParkPath.c_str());
            }
            else
            {
                Console::Error::WriteLine("Park save failed.");
                exitCode = ExitCode::fail;
            }
        }
        if (!_capturePath.empty())
        {
            if (RustBridge::Capture(_capturePath.c_str(), 0 /*zoom*/, 0 /*rotation*/, true /*fitTrack*/, false) != 0)
            {
                Console::Error::WriteLine("Screenshot capture failed.");
                exitCode = ExitCode::fail;
            }
            auto dot = _capturePath.find_last_of('.');
            auto stem = dot == u8string::npos ? _capturePath : _capturePath.substr(0, dot);
            auto ext = dot == u8string::npos ? u8string{} : _capturePath.substr(dot);
            if (_captureAllRotations)
            {
                for (uint8_t rotation = 1; rotation < 4; rotation++)
                {
                    auto path = stem + "-r" + std::to_string(rotation) + ext;
                    if (RustBridge::Capture(path.c_str(), 0, rotation, true, false) != 0)
                    {
                        Console::Error::WriteLine("Screenshot capture failed (rotation %d).", rotation);
                        exitCode = ExitCode::fail;
                    }
                }
            }
            if (_captureXray)
            {
                auto path = stem + "-x" + ext;
                if (RustBridge::Capture(path.c_str(), 0, 0, true, true /*xray*/) != 0)
                {
                    Console::Error::WriteLine("Screenshot capture failed (xray view).");
                    exitCode = ExitCode::fail;
                }
            }
        }
        // Last: filming ticks the simulation on.
        if (!_replayPath.empty())
        {
            if (_replaySeconds <= 0)
            {
                Console::Error::WriteLine("--replay-seconds must be positive");
                return ExitCode::fail;
            }
            if (RustBridge::CaptureReplay(_replayPath.c_str(), static_cast<uint32_t>(_replaySeconds), 0 /*zoom*/)
                != 0)
            {
                Console::Error::WriteLine("Replay capture failed.");
                exitCode = ExitCode::fail;
            }
            else
            {
                Console::WriteLine("Replay written to %s", _replayPath.c_str());
            }
        }
        // Last of all: the montage tears the ride down and rebuilds it, so
        // nothing that reads the scored ride may run after this.
        if (!_buildMontagePath.empty())
        {
            if (_programPath.empty())
            {
                Console::Error::WriteLine("--build-montage needs --program: it films that program being built");
                return ExitCode::fail;
            }
            if (RustBridge::CaptureBuildMontage(
                    _buildMontagePath.c_str(), _programPath.c_str(),
                    _presentationPath.empty() ? nullptr : _presentationPath.c_str(), 0 /*zoom*/)
                != 0)
            {
                Console::Error::WriteLine("Build montage failed.");
                exitCode = ExitCode::fail;
            }
            else
            {
                Console::WriteLine("Build montage written to %s", _buildMontagePath.c_str());
            }
        }
        // Also destructive, and it builds every round itself, so it wants the
        // park it started with: run it without --program.
        if (!_evolutionPath.empty())
        {
            if (_evolutionManifest.empty())
            {
                Console::Error::WriteLine("--evolution-montage needs --evolution: the rounds to film");
                return ExitCode::fail;
            }
            if (RustBridge::CaptureEvolution(
                    _evolutionPath.c_str(), _evolutionManifest.c_str(),
                    _evolutionLapPath.empty() ? nullptr : _evolutionLapPath.c_str(),
                    static_cast<uint32_t>(_replaySeconds), 0 /*zoom*/)
                != 0)
            {
                Console::Error::WriteLine("Evolution montage failed.");
                exitCode = ExitCode::fail;
            }
            else
            {
                Console::WriteLine("Evolution montage written to %s", _evolutionPath.c_str());
            }
        }

        // Also destructive and also self-contained: it builds and demolishes
        // everything the session did, so it wants the park it started with.
        if (!_traceMontagePath.empty())
        {
            if (_traceActionsPath.empty())
            {
                Console::Error::WriteLine("--trace-montage needs --trace-actions: the calls to replay");
                return ExitCode::fail;
            }
            if (RustBridge::CaptureTraceMontage(_traceMontagePath.c_str(), _traceActionsPath.c_str(), 0 /*zoom*/)
                != 0)
            {
                Console::Error::WriteLine("Trace montage failed.");
                exitCode = ExitCode::fail;
            }
            else
            {
                Console::WriteLine("Trace montage written to %s", _traceMontagePath.c_str());
            }
        }

        return exitCode;
    }
} // namespace OpenRCT2

#endif // ENABLE_RUST_AGENT
