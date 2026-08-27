//! CoasterBench orchestrator: runs a head-to-head eval where each contender is
//! a Claude Code session inside an OpenShell sandbox (personal-sub auth),
//! building interactively against the game's MCP server on this host.
//!
//!   coaster-bench --models claude-fable-5 claude-sonnet-5 --rounds 4 \
//!       --ride-type 51 --name sub-twister-1
//!
//! The game server is spawned as a child (coasterbench-cli eval --serve) and the
//! orchestrator collects per-round artifacts (report.json, program.json,
//! park.png) in the same evals/runs/<name>/ layout driver.py produces, so the
//! site generator needs no changes.

mod mcp;
mod prompt;
mod trace;

use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{Parser, ValueEnum};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const RUN_SCHEMA_VERSION: u32 = 2;
const SCORER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Contender model ids, one Claude Code session per round each.
    #[arg(long, num_args = 1.., default_values_t = vec!["claude-sonnet-5".to_string()])]
    models: Vec<String>,

    /// Reopen one archived round for a presentation-only model pass. The
    /// original model chooses a name and colours; score and layout must match.
    #[arg(long, value_name = "ROUND_DIR")]
    present_round: Option<PathBuf>,

    #[arg(long, default_value_t = 4)]
    rounds: u32,

    /// Competition ride type (51 steel twister, 52 wooden).
    #[arg(long, default_value_t = 51)]
    ride_type: u16,

    /// Benchmark condition. Design is MCP-only, library exposes stock-design
    /// lookup, and open-note stages the scorer's upstream engine source.
    #[arg(long, value_enum, default_value_t = BenchmarkMode::Design)]
    mode: BenchmarkMode,

    /// Test simulation budget passed to finish_and_test.
    #[arg(long, default_value_t = 25000)]
    ticks: u32,

    /// Run dir suffix: evals/runs/<yyyymmdd>-<name>.
    #[arg(long)]
    name: Option<String>,

    /// OpenShell sandbox that runs Claude Code (must reach this host's MCP port).
    #[arg(long, default_value = "coaster-sub")]
    sandbox: String,

    /// OpenShell sandbox for opencode contenders (OpenRouter lane).
    #[arg(long, default_value = "coaster-or")]
    opencode_sandbox: String,

    /// MCP port; must be allowed by the sandbox policy.
    #[arg(long, default_value_t = 8791)]
    port: u16,

    /// Loopback-only control port. The harness's own channel into the game
    /// (save_park), on a listener the sandboxes cannot route to, so nothing an
    /// agent can reach writes to this filesystem.
    #[arg(long, default_value_t = 8792)]
    control_port: u16,

    /// Max agentic turns per round session. Claude Code only; opencode has no
    /// equivalent, so --session-timeout is what bounds it.
    #[arg(long, default_value_t = 120)]
    max_turns: u32,

    /// Wall-clock budget per round session, in seconds. A session that runs
    /// past it is killed and the round is scored on whatever stands in the
    /// park. Without this an opencode session can iterate indefinitely.
    #[arg(long, default_value_t = 1800)]
    session_timeout: u64,

    /// Kill a session whose `session.log` has gone this long without a byte,
    /// so a hung upstream costs a retry rather than the round. 0 disables.
    /// Loose on purpose: the longest quiet stretch across healthy archived
    /// rounds is 949s, and a model thinking is indistinguishable from one
    /// stalled.
    #[arg(long, default_value_t = 1200)]
    idle_timeout: u64,

    /// Retries per round before it is scored on whatever it built.
    #[arg(long, default_value_t = 1)]
    idle_retries: u32,

    #[arg(
        long,
        default_value = "~/rct2-assets/Scenarios/Build your own Six Flags Park.SC6"
    )]
    scenario: String,

    #[arg(long, default_value = "~/rct2-assets")]
    rct2_data: String,

    /// Reuse an already-running MCP server instead of spawning one.
    #[arg(long)]
    attach: bool,

    /// Create a throwaway sandbox for this run and delete it afterwards, so no
    /// state survives between runs. Covers every lane a contender uses.
    #[arg(long)]
    fresh_sandbox: bool,

    #[arg(long, default_value = "localhost/coaster-sandbox")]
    sandbox_image: String,

    #[arg(long, default_value = "rust/coaster-bench/sandbox/policy.yaml")]
    sandbox_policy: String,

    #[arg(long, default_value = "claude-sub")]
    sandbox_provider: String,

    #[arg(long, default_value = "localhost/coaster-or")]
    opencode_sandbox_image: String,

    #[arg(
        long,
        default_value = "rust/coaster-bench/sandbox/opencode-policy.yaml"
    )]
    opencode_sandbox_policy: String,

    #[arg(long, default_value = "openrouter")]
    opencode_sandbox_provider: String,

    /// Sandbox for `claude-code:openrouter/...` contenders. Its own lane, not
    /// the opencode one: both run out of the coaster-or image, and a fresh run
    /// would otherwise create two sandboxes and keep only one name.
    #[arg(long, default_value = "coaster-cc-or")]
    claude_openrouter_sandbox: String,

    #[arg(long, default_value = "localhost/coaster-or")]
    claude_openrouter_sandbox_image: String,

    #[arg(
        long,
        default_value = "rust/coaster-bench/sandbox/opencode-policy.yaml"
    )]
    claude_openrouter_sandbox_policy: String,

    /// Continue an existing run directory instead of starting a dated one:
    /// rounds already archived there are kept, and the loop picks up at the
    /// first round each model is missing. Raise --rounds to add more. Ignores
    /// --name. Delete a round directory to have it built again.
    #[arg(long, value_name = "RUN_DIR")]
    resume: Option<PathBuf>,

    /// Pin the upstream OpenRouter routes an opencode contender to (e.g.
    /// "baseten"). OpenRouter serves one model from several upstreams that do
    /// not validate requests identically, so leaving it open makes the serving
    /// stack a lottery the run record cannot describe.
    #[arg(long)]
    openrouter_provider: Option<String>,

    #[arg(long, default_value = "codex-arena")]
    codex_sandbox: String,

    #[arg(long, default_value = "localhost/codex-arena")]
    codex_sandbox_image: String,

    #[arg(long, default_value = "rust/coaster-bench/sandbox/codex-policy.yaml")]
    codex_sandbox_policy: String,

    #[arg(long, default_value = "codex-oauth")]
    codex_sandbox_provider: String,

    #[arg(long, default_value = "/home/sandbox/bin/codex")]
    codex_bin: String,

    /// Reasoning effort for Codex contenders. Pin this instead of inheriting
    /// the model catalogue default so public runs remain reproducible.
    #[arg(long, value_enum, default_value_t = CodexReasoningEffort::Medium)]
    codex_reasoning_effort: CodexReasoningEffort,

    /// Extra tools to allow the agent, comma separated (e.g. "Bash"). Open note
    /// implies the file tools; this grants them without staging any source.
    #[arg(long)]
    extra_tools: Option<String>,

    /// Cap on the replay recorded as round_N/replay.mp4 (needs ffmpeg on
    /// PATH). The length actually used is the lap time the game measured, so
    /// this only bounds a pathological ride. 0 records nothing.
    #[arg(long, default_value_t = 90)]
    replay_seconds: u32,

    /// Compatibility spelling for `--mode open-note`.
    #[arg(long)]
    open_note: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BenchmarkMode {
    Design,
    Library,
    OpenNote,
    /// Internal mode selected by --present-round. It is a capability boundary,
    /// not a standalone benchmark condition.
    #[value(skip)]
    Presentation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CodexReasoningEffort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
}

impl CodexReasoningEffort {
    fn as_str(self) -> &'static str {
        match self {
            CodexReasoningEffort::Low => "low",
            CodexReasoningEffort::Medium => "medium",
            CodexReasoningEffort::High => "high",
            CodexReasoningEffort::Xhigh => "xhigh",
            CodexReasoningEffort::Max => "max",
            CodexReasoningEffort::Ultra => "ultra",
        }
    }
}

impl BenchmarkMode {
    /// Wire condition understood by the MCP server. Open note remains a
    /// from-scratch design condition; its extra capability is local source.
    fn mcp_condition(self) -> &'static str {
        match self {
            BenchmarkMode::Design | BenchmarkMode::OpenNote => "design",
            BenchmarkMode::Library => "library",
            BenchmarkMode::Presentation => "presentation",
        }
    }

    fn base_mode(self) -> &'static str {
        match self {
            BenchmarkMode::Library => "library",
            BenchmarkMode::Design | BenchmarkMode::OpenNote => "design",
            BenchmarkMode::Presentation => "presentation",
        }
    }

    fn condition_name(self) -> &'static str {
        match self {
            BenchmarkMode::Design => "design",
            BenchmarkMode::Library => "library",
            BenchmarkMode::OpenNote => "open-note",
            BenchmarkMode::Presentation => "presentation",
        }
    }
}

const OPEN_NOTE_DIR: &str = "/tmp/openrct2-src";

/// Longest name the gateway accepts.
const MAX_SANDBOX_NAME: usize = 19;

/// `<base>-<short id>`, trimming the base rather than the id so runs stay
/// distinguishable when the name has to be cut.
fn ephemeral_sandbox_name(base: &str, epoch: u64) -> String {
    let id = format!("{:x}", epoch % 0xffffff);
    let room = MAX_SANDBOX_NAME.saturating_sub(id.len() + 1);
    let base: String = base.chars().take(room).collect();
    format!("{base}-{id}")
}

/// A sandbox owned by this run: deleted on the way out, including when a signal
/// unwinds the run, because the flag path returns from main normally.
struct EphemeralSandbox {
    name: String,
}

impl Drop for EphemeralSandbox {
    fn drop(&mut self) {
        let out = Command::new("openshell")
            .args(["sandbox", "delete", &self.name])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        match out {
            Ok(o) if o.status.success() => println!("deleted sandbox {}", self.name),
            Ok(o) if String::from_utf8_lossy(&o.stderr).contains("not found") => {}
            _ => eprintln!(
                "could not delete sandbox {}; remove it with `openshell sandbox delete {}`",
                self.name, self.name
            ),
        }
    }
}

/// The policy file pins the MCP port, and a mismatch fails as a silent network
/// denial inside the sandbox. Parse the policy and touch only the coaster-game
/// endpoint; inference endpoints such as api.openai.com:443 are unrelated.
fn policy_with_port(policy: &str, port: u16) -> Result<String, String> {
    let mut document: serde_norway::Value =
        serde_norway::from_str(policy).map_err(|e| format!("parse sandbox policy: {e}"))?;
    let endpoints = document
        .get_mut("network_policies")
        .and_then(|v| v.get_mut("coaster_game"))
        .and_then(|v| v.get_mut("endpoints"))
        .and_then(serde_norway::Value::as_sequence_mut)
        .ok_or("sandbox policy has no network_policies.coaster_game.endpoints")?;
    let mut changed = false;
    for endpoint in endpoints {
        if endpoint.get("host").and_then(serde_norway::Value::as_str)
            == Some("host.containers.internal")
        {
            endpoint["port"] =
                serde_norway::to_value(port).map_err(|e| format!("serialize MCP port: {e}"))?;
            changed = true;
        }
    }
    if !changed {
        return Err("coaster_game policy has no host.containers.internal endpoint".into());
    }
    serde_norway::to_string(&document).map_err(|e| format!("serialize sandbox policy: {e}"))
}

struct SandboxRecipe<'a> {
    image: &'a str,
    policy: &'a str,
    provider: &'a str,
}

fn create_sandbox(
    recipe: &SandboxRecipe,
    port: u16,
    root: &Path,
    name: &str,
) -> Result<EphemeralSandbox, String> {
    let policy_path = root.join(recipe.policy);
    let policy = std::fs::read_to_string(&policy_path)
        .map_err(|e| format!("read {}: {e}", policy_path.display()))?;
    let staged = std::env::temp_dir().join(format!("coaster-policy-{name}.yaml"));
    std::fs::write(&staged, policy_with_port(&policy, port)?)
        .map_err(|e| format!("write {}: {e}", staged.display()))?;

    // `sandbox create` attaches and never returns, but the sandbox is up long
    // before that and outlives the client, so spawn it, wait for the sandbox to
    // answer, then drop the client.
    let mut cmd = Command::new("openshell");
    cmd.args(["sandbox", "create", "--name", name])
        .args(["--from", recipe.image])
        .arg("--policy")
        .arg(&staged);
    for provider in recipe.provider.split(',').filter(|p| !p.is_empty()) {
        cmd.args(["--provider", provider]);
    }
    let mut child = cmd
        .arg("--no-tty")
        .args(["--", "sleep", "infinity"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("openshell create: {e}"))?;
    // Registered before the wait: a sandbox that comes up and then fails the
    // readiness check still has to be cleaned up.
    let guard = EphemeralSandbox {
        name: name.to_string(),
    };

    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if sandbox_ready(name) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&staged);
            return Ok(guard);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    use std::io::Read;
                    let _ = pipe.read_to_string(&mut stderr);
                }
                let _ = std::fs::remove_file(&staged);
                return Err(format!(
                    "creating sandbox {name} failed ({status}): {}",
                    stderr.trim()
                ));
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&staged);
                return Err(format!("sandbox {name} never became ready"));
            }
            Ok(None) => std::thread::sleep(Duration::from_secs(2)),
            Err(e) => return Err(format!("waiting on sandbox create: {e}")),
        }
    }
}

/// Runs a command with a deadline, killing it if it overruns. `sandbox exec`
/// against a sandbox that is still coming up blocks instead of failing, so an
/// unbounded readiness probe hangs forever and never reaches its own deadline.
fn run_bounded(mut cmd: Command, budget: Duration) -> Option<bool> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(500)),
            Err(_) => return None,
        }
    }
}

fn sandbox_ready(name: &str) -> bool {
    let mut cmd = Command::new("openshell");
    cmd.args(["sandbox", "exec", "-n", name, "--"])
        .args(["true"]);
    run_bounded(cmd, Duration::from_secs(20)) == Some(true)
}

/// Set once on SIGINT/SIGTERM/SIGHUP. A handler cannot do the cleanup itself
/// (killing children and reaping them is not async-signal-safe, and process
/// exit from a handler skips every Drop), so it only raises this and the run
/// loop unwinds normally: agent killed, sandbox swept, game server dropped.
fn install_shutdown_flag() -> Result<Arc<AtomicBool>, String> {
    let flag = Arc::new(AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(signal, Arc::clone(&flag))
            .map_err(|e| format!("register signal {signal}: {e}"))?;
    }
    Ok(flag)
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn repo_root() -> PathBuf {
    // target/{debug,release}/coaster-bench -> rust/coaster-bench -> repo
    let exe = std::env::current_exe().unwrap_or_default();
    exe.ancestors()
        .find(|p| p.join("src/openrct2").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

struct GameServer {
    child: Option<Child>,
}

impl GameServer {
    fn spawn(args: &Args, root: &Path) -> Result<GameServer, String> {
        let cli = root.join("build/coasterbench-cli");
        if !cli.is_file() {
            return Err(format!("{} not built", cli.display()));
        }
        // Our own child would fail to bind and wait_for_server would then talk
        // happily to whatever is already there: a stale server holding a
        // different park, or another run in progress. Scoring against that is
        // worse than not running.
        if port_in_use(args.port) {
            return Err(format!(
                "port {} is already serving; kill the stale server (lsof -ti:{}) or pass --attach to use it deliberately",
                args.port, args.port
            ));
        }
        if port_in_use(args.control_port) {
            return Err(format!(
                "control port {} is already serving; kill the stale server (lsof -ti:{}) or choose another --control-port",
                args.control_port, args.control_port
            ));
        }
        let child = Command::new(cli)
            .arg("eval")
            .arg(expand_home(&args.scenario))
            .arg("--rct2-data-path")
            .arg(expand_home(&args.rct2_data))
            .arg("--serve")
            .arg(args.port.to_string())
            .arg("--serve-bind")
            .arg("0.0.0.0")
            .arg("--serve-control")
            .arg(args.control_port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn game server: {e}"))?;
        Ok(GameServer { child: Some(child) })
    }
}

impl Drop for GameServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn port_in_use(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(500),
    )
    .is_ok()
}

fn wait_for_server(client: &mut mcp::McpClient, budget: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        match client.initialize() {
            Ok(()) => return Ok(()),
            Err(e) if start.elapsed() > budget => return Err(format!("server not ready: {e}")),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

/// A contender: which agent harness runs it and the model id that harness
/// understands. Spec syntax: bare name = Claude Code on the sub lane;
/// "opencode:<provider/model>" = opencode in the OpenRouter sandbox.
#[derive(Debug, Clone, PartialEq)]
struct Contender {
    harness: Harness,
    model: String,
    /// What the model can actually be fed. The MCP server hides tools that
    /// answer outside this set, because a single image content block sent to a
    /// text-only model fails the whole request upstream.
    modalities: Modalities,
}

/// Input modalities a model accepts, named after the OpenRouter catalogue's
/// `input_modalities` field, which is also the wire format the game's MCP
/// server parses out of `/mcp?modalities=...`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Modalities(u8);

impl Modalities {
    const TEXT: Modalities = Modalities(1 << 0);
    const IMAGE: Modalities = Modalities(1 << 1);
    const AUDIO: Modalities = Modalities(1 << 2);
    const FILE: Modalities = Modalities(1 << 3);

    fn from_catalog_name(name: &str) -> Modalities {
        match name {
            "text" => Modalities::TEXT,
            "image" => Modalities::IMAGE,
            "audio" => Modalities::AUDIO,
            "file" => Modalities::FILE,
            _ => Modalities(0),
        }
    }

    fn contains(self, other: Modalities) -> bool {
        self.0 & other.0 == other.0
    }

    /// Wire form for the MCP query string, e.g. "text,image".
    fn as_query_value(self) -> String {
        [
            (Modalities::TEXT, "text"),
            (Modalities::IMAGE, "image"),
            (Modalities::AUDIO, "audio"),
            (Modalities::FILE, "file"),
        ]
        .iter()
        .filter(|(flag, _)| self.contains(*flag))
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(",")
    }
}

impl std::ops::BitOr for Modalities {
    type Output = Modalities;
    fn bitor(self, rhs: Modalities) -> Modalities {
        Modalities(self.0 | rhs.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Harness {
    ClaudeCode,
    Opencode,
    Codex,
}

impl Harness {
    fn name(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Opencode => "opencode",
            Harness::Codex => "codex",
        }
    }

    /// Whether the round ends when the model stops calling tools. codex and
    /// opencode never re-prompt; Claude Code runs to its turn budget.
    fn single_turn(self) -> bool {
        match self {
            Harness::Codex | Harness::Opencode => true,
            Harness::ClaudeCode => false,
        }
    }
}

/// A sandbox recipe, which is a harness plus the backend it authenticates
/// against. Claude Code has two: the subscription and OpenRouter's
/// Anthropic-compatible endpoint. They cannot share a sandbox, because the
/// policy and the injected credential differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    ClaudeCode,
    ClaudeCodeOpenRouter,
    Opencode,
    Codex,
}

impl Contender {
    fn parse(spec: &str) -> Contender {
        match spec.split_once(':') {
            Some(("opencode", model)) => Contender {
                harness: Harness::Opencode,
                // Claude models are all multimodal; OpenRouter ones are not,
                // so ask the catalogue instead of assuming.
                modalities: openrouter_modalities(model)
                    .unwrap_or(Modalities::TEXT | Modalities::IMAGE),
                model: model.to_string(),
            },
            Some(("codex", model)) => Contender {
                harness: Harness::Codex,
                modalities: match model.strip_prefix("openrouter/") {
                    Some(or) => {
                        openrouter_modalities(or).unwrap_or(Modalities::TEXT | Modalities::IMAGE)
                    }
                    None => Modalities::TEXT | Modalities::IMAGE,
                },
                model: model.to_string(),
            },
            Some(("claude-code", model)) => Contender {
                harness: Harness::ClaudeCode,
                modalities: match model.strip_prefix("openrouter/") {
                    Some(or) => {
                        openrouter_modalities(or).unwrap_or(Modalities::TEXT | Modalities::IMAGE)
                    }
                    None => Modalities::TEXT | Modalities::IMAGE,
                },
                model: model.to_string(),
            },
            _ => Contender {
                harness: Harness::ClaudeCode,
                model: spec.to_string(),
                modalities: Modalities::TEXT | Modalities::IMAGE,
            },
        }
    }

    /// Directory / display name: the model id, filesystem-safe.
    fn display(&self) -> String {
        self.model.replace('/', "_")
    }

    fn lane(&self) -> Lane {
        match self.harness {
            Harness::ClaudeCode if self.claude_openrouter_model().is_some() => {
                Lane::ClaudeCodeOpenRouter
            }
            Harness::ClaudeCode => Lane::ClaudeCode,
            Harness::Opencode => Lane::Opencode,
            Harness::Codex => Lane::Codex,
        }
    }

    /// Which OpenShell sandbox this contender's lane runs in.
    fn sandbox<'a>(&self, args: &'a Args) -> &'a str {
        match self.lane() {
            Lane::ClaudeCode => &args.sandbox,
            Lane::ClaudeCodeOpenRouter => &args.claude_openrouter_sandbox,
            Lane::Opencode => &args.opencode_sandbox,
            Lane::Codex => &args.codex_sandbox,
        }
    }

    fn codex_openrouter_model(&self) -> Option<&str> {
        (self.harness == Harness::Codex).then(|| self.model.strip_prefix("openrouter/"))?
    }

    /// The bare OpenRouter slug a Claude Code contender runs, if it is one.
    /// Claude Code takes the model without the provider prefix, because the
    /// prefix is carried by ANTHROPIC_BASE_URL instead.
    fn claude_openrouter_model(&self) -> Option<&str> {
        (self.harness == Harness::ClaudeCode).then(|| self.model.strip_prefix("openrouter/"))?
    }

    /// Whether this contender's tokens are billed by OpenRouter, and so are
    /// counted by the key's spend delta rather than the transcript.
    fn billed_by_openrouter(&self) -> bool {
        matches!(self.lane(), Lane::Opencode | Lane::ClaudeCodeOpenRouter)
            || self.codex_openrouter_model().is_some()
    }

    fn provider<'a>(&self, args: &'a Args) -> &'a str {
        match self.lane() {
            Lane::ClaudeCode => &args.sandbox_provider,
            Lane::ClaudeCodeOpenRouter => "openrouter",
            Lane::Opencode => &args.opencode_sandbox_provider,
            Lane::Codex if self.codex_openrouter_model().is_some() => "openrouter",
            Lane::Codex => &args.codex_sandbox_provider,
        }
    }

    /// MCP endpoint this contender's harness should connect to. The server
    /// hides tools answering outside the advertised modality set, and refuses
    /// anyone whose lease is not the one that currently owns the park.
    fn mcp_url(&self, port: u16, lease: &str, mode: BenchmarkMode) -> String {
        format!(
            "http://host.containers.internal:{port}/mcp?modalities={}&condition={}&lease={lease}",
            self.modalities.as_query_value(),
            mode.mcp_condition(),
        )
    }
}

/// Input modalities an OpenRouter model declares. `None` when the catalogue
/// can't be reached or doesn't list the model, so callers pick their own
/// default. The opencode model id carries a provider prefix
/// ("openrouter/author/model") that the catalogue ids don't have.
fn openrouter_modalities(model: &str) -> Option<Modalities> {
    let id = model.strip_prefix("openrouter/").unwrap_or(model);
    let catalog: Value = ureq::get("https://openrouter.ai/api/v1/models")
        .timeout(Duration::from_secs(30))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let declared = catalog
        .get("data")?
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(Value::as_str) == Some(id))?
        .pointer("/architecture/input_modalities")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .fold(Modalities(0), |set, name| {
            set | Modalities::from_catalog_name(name)
        });
    Some(declared)
}

/// Token/cost accounting for one agent session; every field best-effort.
struct SessionResult {
    usage: Value,
    error: Option<String>,
    /// Killed for going quiet, not for finishing or hitting the wall clock.
    hung: bool,
}

impl SessionResult {
    /// The session never got far enough to report usage.
    fn failed(error: String) -> SessionResult {
        SessionResult {
            usage: Value::Null,
            error: Some(error),
            hung: false,
        }
    }
}

/// `quiet` is the time since the last byte landed in `session.log`.
fn session_went_quiet(quiet: Duration, idle_timeout: u64) -> bool {
    idle_timeout > 0 && quiet >= Duration::from_secs(idle_timeout)
}

/// `hung_sessions` includes the one that just stalled.
fn retry_hung_round(hung_sessions: u32, retries: u32) -> bool {
    hung_sessions <= retries
}

/// The retry starts a fresh `session.log`, so the stalled one is renamed to
/// keep it. Never `report.json` or `park.png`: `completed_rounds` reads those
/// as a finished round.
fn keep_hung_logs(round_dir: &Path, attempt: u32) -> Vec<String> {
    let mut kept = Vec::new();
    for name in ["session.log", "session.err"] {
        let Some((stem, ext)) = name.split_once('.') else {
            continue;
        };
        let kept_name = format!("{stem}.hung_{attempt}.{ext}");
        if std::fs::rename(round_dir.join(name), round_dir.join(&kept_name)).is_ok() {
            kept.push(kept_name);
        }
    }
    kept
}

/// Current spend (USD) on the OpenRouter key, for cost-by-delta accounting.
fn openrouter_spend() -> Option<f64> {
    let key = Command::new("security")
        .args(["find-generic-password", "-s", "openrouter-api-key", "-w"])
        .output()
        .ok()?;
    let key = String::from_utf8_lossy(&key.stdout).trim().to_string();
    if key.is_empty() {
        return None;
    }
    let response: Value = ureq::get("https://openrouter.ai/api/v1/key")
        .set("Authorization", &format!("Bearer {key}"))
        .timeout(Duration::from_secs(30))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    response.pointer("/data/usage").and_then(Value::as_f64)
}

/// Point the opencode sandbox at this contender's MCP endpoint. Written fresh
/// per session so a mixed vision/text-only lineup can share one sandbox.
/// opencode's tool permissions default to allow, so without this a blind run on
/// this lane is not blind at all: it keeps bash, read and grep, while the Claude
/// Code lane is held to MCP only by its allowlist. Named explicitly rather than
/// with a wildcard, because a wildcard would also catch the MCP tools.
fn opencode_permissions(args: &Args) -> Value {
    let verdict = if args.open_note { "allow" } else { "deny" };
    json!({
        "bash": verdict,
        "read": verdict,
        "grep": verdict,
        "glob": verdict,
        "edit": verdict,
        // Open note stages the source outside the project directory, which
        // otherwise prompts and stalls a non-interactive session.
        "external_directory": verdict,
        "webfetch": "deny",
        "websearch": "deny",
        // opencode asks before letting a model repeat a tool call it thinks is
        // looping. Headless, "ask" auto-rejects and the rejection kills the
        // session, so a model that undoes several pieces in a row loses the
        // round. Repetition is normal here (undo_piece, valid_next_pieces), and
        // the real bound on a runaway session is --session-timeout.
        "doom_loop": "allow",
    })
}

/// How many rounds this model has already banked in a run directory, counted
/// as the unbroken run from round 1. A round counts as done once it archived a
/// report, whatever that report says: an unrated round still spent the agent's
/// turn at the park, and rerunning it would overwrite the evidence of what went
/// wrong. Deleting a round directory is how you ask for it to be built again.
fn completed_rounds(run_dir: &Path, model: &str) -> u32 {
    let mut done = 0;
    loop {
        let round = run_dir.join(model).join(format!("round_{}", done + 1));
        // report.json is written partway through collection, so on its own it
        // also marks a round that died collecting: the kimi run that lost
        // round 6 to a SIGTERM left an empty report behind, and counting it
        // would have made the resume a no-op. park.png is written last, so it
        // is the archive's own record that the round finished.
        if !round.join("report.json").is_file() || !round.join("park.png").is_file() {
            return done;
        }
        done += 1;
    }
}

/// Rebuild what the round loop carries forward, from the archive rather than
/// from memory: the previous round's report (which the next prompt quotes back
/// to the model) and the best score so far (which is the run's actual result).
/// Without this a continued run would restart the model cold and could crown a
/// later, worse round as its best.
fn resumed_state(
    run_dir: &Path,
    model: &str,
    through: u32,
) -> (Option<(u32, f64)>, Option<String>) {
    let mut best: Option<(u32, f64)> = None;
    let mut feedback = None;
    for round in 1..=through {
        let path = run_dir
            .join(model)
            .join(format!("round_{round}"))
            .join("report.json");
        let Ok(report) = read_json(&path) else {
            continue;
        };
        if let Some(s) = best_excitement(&report) {
            if best.is_none_or(|(_, b)| s > b) {
                best = Some((round, s));
            }
        }
        feedback = serde_json::to_string(&report).ok();
    }
    (best, feedback)
}

/// Describe a run that predates the segment log as its own first segment, from
/// what it did record, so a continued run has one shape whatever it grew from.
fn original_segment(existing: &Value) -> Value {
    let carry = |key: &str| existing.get(key).cloned().unwrap_or(Value::Null);
    json!({
        "started_at": Value::Null,
        "first_round": Value::Null,
        "rounds": carry("rounds"),
        "harness_version": carry("harness_version"),
        "source": carry("source"),
        "environment": carry("environment"),
        "openrouter_provider": carry("openrouter_provider"),
    })
}

/// Refuse to continue a run under different terms. Everything checked here is
/// something that would silently corrupt the record: rounds built blind sitting
/// beside rounds built with the library, or two ride types in one standings
/// table. Rounds may grow, and the harness build may differ (that is what the
/// segment log is for), but the experiment may not change underneath it.
fn check_resume_compatible(
    existing: &Value,
    args: &Args,
    contenders: &[Contender],
) -> Result<(), String> {
    let str_field = |key: &str| existing.get(key).and_then(Value::as_str).unwrap_or("");
    let mismatch = |what: &str, was: String, now: String| {
        Err(format!(
            "resume: this run was {what} {was}, not {now}; start a new run instead"
        ))
    };
    if str_field("condition") != args.mode.condition_name() {
        return mismatch(
            "built under condition",
            str_field("condition").into(),
            args.mode.condition_name().into(),
        );
    }
    let ride = existing.get("ride_type").and_then(Value::as_u64);
    if ride != Some(args.ride_type as u64) {
        return mismatch(
            "built for ride_type",
            ride.map_or("unknown".into(), |r| r.to_string()),
            args.ride_type.to_string(),
        );
    }
    let was: Vec<String> = existing
        .get("models")
        .and_then(Value::as_array)
        .map(|m| {
            m.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let now: Vec<String> = contenders.iter().map(Contender::display).collect();
    if was != now {
        return mismatch("run with models", was.join(","), now.join(","));
    }
    Ok(())
}

/// The openrouter provider block for this contender's opencode config, or None
/// for contenders that are not OpenRouter-served.
///
/// The model is always declared explicitly: opencode resolves model names
/// against a bundled models.dev snapshot, and a checkpoint newer than the
/// binary fails the whole session with ProviderModelNotFoundError (deepseek
/// v4-pro-0813 died this way on release day). A config entry merges into the
/// catalogue, so the run works however stale the snapshot is.
///
/// `--openrouter-provider` additionally pins one named upstream. Moonshot's own
/// endpoint rejects an empty assistant message with a non-retryable 400, and
/// kimi-k3 emits one often enough to lose rounds to it; the other upstreams
/// serving the same weights do not. Unpinned runs leave OpenRouter's routing
/// alone.
fn opencode_provider_routing(args: &Args, contender: &Contender) -> Option<Value> {
    // opencode keys models by their OpenRouter slug, without the provider it
    // already nests them under.
    let model = contender.model.strip_prefix("openrouter/")?;
    let mut entry = json!({"name": model});
    if let Some(upstream) = args.openrouter_provider.as_deref() {
        entry["options"] = json!({"provider": {
            "order": [upstream],
            // A dead upstream should cost a retry, not the round.
            "allow_fallbacks": true,
        }});
    }
    Some(json!({"openrouter": {"models": {model: entry}}}))
}

fn write_opencode_config(args: &Args, contender: &Contender, lease: &str) -> Result<(), String> {
    let mut config = json!({
        "$schema": "https://opencode.ai/config.json",
        "model": contender.model,
        "permission": opencode_permissions(args),
        "mcp": {
            "coaster": {"type": "remote", "url": contender.mcp_url(args.port, lease, args.mode), "enabled": true}
        }
    });
    if let Some(routing) = opencode_provider_routing(args, contender) {
        config["provider"] = routing;
    }
    let config = config.to_string();
    let script = format!(
        "mkdir -p ~/.config/opencode && cat > ~/.config/opencode/opencode.json <<'OPENCODE_EOF'\n{config}\nOPENCODE_EOF"
    );
    let output = Command::new("openshell")
        .args(["sandbox", "exec", "-n", &args.opencode_sandbox, "--"])
        .args(["sh", "-c", &script])
        .output()
        .map_err(|e| format!("write opencode config: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "write opencode config ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn rfc3339_now() -> String {
    let secs = epoch_secs();
    let (y, m, d) = civil_from_days((secs / 86400) as i64);
    let (h, min, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.000Z")
}

fn host_codex_identity() -> Result<(String, String), String> {
    let path = expand_home("~/.codex/auth.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {} (run `codex login` first): {e}", path.display()))?;
    let auth: Value = serde_json::from_str(&raw).map_err(|e| format!("parse codex auth: {e}"))?;
    let field = |name: &str| {
        auth.pointer(&format!("/tokens/{name}"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("codex auth.json has no tokens.{name}"))
    };
    Ok((field("id_token")?, field("account_id")?))
}

/// Whether this contender can run commands.
fn has_shell(args: &Args, contender: &Contender) -> bool {
    match contender.harness {
        Harness::Codex => codex_shell_enabled(args),
        Harness::ClaudeCode | Harness::Opencode => args.open_note,
    }
}

/// codex's exec tools. `sandbox_mode` bounds the filesystem, not exec, so
/// turning the features off is the only way a blind codex round is held to the
/// same MCP-only tool table as the other lanes. Verified against codex 0.145:
/// with both off it answers that it has no tool to run commands.
fn codex_shell_enabled(args: &Args) -> bool {
    args.open_note || allowed_tools(args).contains("Bash")
}

fn codex_sandbox_mode(args: &Args) -> &'static str {
    if args.open_note || allowed_tools(args).contains("Bash") {
        "danger-full-access"
    } else {
        "read-only"
    }
}

fn codex_config(args: &Args, contender: &Contender, lease: &str) -> String {
    let sandbox_mode = codex_sandbox_mode(args);
    let mut config = match contender.codex_openrouter_model() {
        Some(model) => format!(
            "model = \"{model}\"\n\
             model_provider = \"openrouter\"\n"
        ),
        None => format!("model = \"{}\"\n", contender.model),
    };
    config.push_str(&format!(
        "# Codex has no metadata for an \"openai/...\"-prefixed id or a model\n\
         # newer than its own table, and its fallback assumes the model cannot\n\
         # reason: OpenRouter rejects that for models where reasoning is\n\
         # mandatory, and the subscription backend silently returns no\n\
         # reasoning summaries at all (the gpt-5.6-sol run traced zero).\n\
         model_reasoning_summary = \"auto\"\n\
         model_reasoning_effort = \"{}\"\n\
         approval_policy = \"never\"\n\
         sandbox_mode = \"{sandbox_mode}\"\n",
        args.codex_reasoning_effort.as_str()
    ));
    if !codex_shell_enabled(args) {
        config.push_str(
            "\n[features]\n\
             shell_tool = false\n\
             unified_exec = false\n",
        );
    }
    if contender.codex_openrouter_model().is_some() {
        config.push_str(
            "\n[model_providers.openrouter]\n\
             name = \"OpenRouter\"\n\
             base_url = \"https://openrouter.ai/api/v1\"\n\
             env_key = \"OPENROUTER_API_KEY\"\n\
             wire_api = \"responses\"\n",
        );
    }
    format!(
        "{config}\n\
         [projects.\"$HOME\"]\n\
         trust_level = \"trusted\"\n\
         \n\
         [mcp_servers.coaster]\n\
         url = \"{}\"\n\
         # Headless `codex exec` has no user-input handler for MCP approval\n\
         # prompts. This server is the benchmark's deliberately trusted tool\n\
         # boundary, so pre-approve it while leaving the filesystem read-only.\n\
         default_tools_approval_mode = \"approve\"\n",
        contender.mcp_url(args.port, lease, args.mode)
    )
}

fn write_codex_config(args: &Args, contender: &Contender, lease: &str) -> Result<(), String> {
    let mut script = format!(
        "set -e\n\
         CODEX_HOME=\"${{CODEX_HOME:-$HOME/.codex}}\"\n\
         rm -rf \"$CODEX_HOME\"\n\
         mkdir -p \"$CODEX_HOME\"\n\
         cat > \"$CODEX_HOME/config.toml\" <<CODEX_CONFIG_EOF\n\
         {}\
         CODEX_CONFIG_EOF\n",
        codex_config(args, contender, lease)
    );
    if contender.codex_openrouter_model().is_none() {
        let (id_token, account_id) = host_codex_identity()?;
        script.push_str(&format!(
            "cat > \"$CODEX_HOME/auth.json\" <<CODEX_AUTH_EOF\n\
             {{\"auth_mode\":\"chatgpt\",\"OPENAI_API_KEY\":null,\"tokens\":{{\
             \"id_token\":\"{id_token}\",\
             \"access_token\":\"$CODEX_AUTH_ACCESS_TOKEN\",\
             \"refresh_token\":\"$CODEX_AUTH_REFRESH_TOKEN\",\
             \"account_id\":\"{account_id}\"}},\
             \"last_refresh\":\"{}\"}}\n\
             CODEX_AUTH_EOF\n\
             chmod 600 \"$CODEX_HOME/auth.json\"\n",
            rfc3339_now()
        ));
    }

    let mut cmd = Command::new("openshell");
    cmd.args(["sandbox", "exec", "-n", &args.codex_sandbox, "--"])
        .args(["sh", "-c", &script]);
    match run_bounded(cmd, Duration::from_secs(120)) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "writing codex config in {} failed",
            args.codex_sandbox
        )),
        None => Err(format!(
            "writing codex config in {} timed out",
            args.codex_sandbox
        )),
    }
}

/// Kill agent processes left behind in a sandbox. Killing the local openshell
/// client does NOT stop the process it started inside the sandbox, so a run
/// that dies (Ctrl-C, `pkill`, a timeout) strands an agent that keeps building
/// in the shared park and keeps spending. Two of those once fought over the
/// same ride for an hour, each demolishing the other's track.
/// The sandbox image has no ps/pkill, so this walks /proc and matches the
/// executable rather than the command line: the command line of this very
/// script contains "opencode", and matching that would kill the cleanup shell.
/// It must stay a single line; openshell's exec drops multi-line scripts.
fn kill_stray_agents(sandbox: &str) {
    let script = "for pid in $(ls /proc | grep \"^[0-9]\"); do [ \"$pid\" = \"$$\" ] && continue; \
                  exe=$(readlink /proc/$pid/exe 2>/dev/null); \
                  case \"$exe\" in *opencode*|*claude*|*codex*) kill -TERM \"$pid\" ;; esac; done";
    let _ = Command::new("openshell")
        .args(["sandbox", "exec", "-n", sandbox, "--"])
        .args(["sh", "-c", script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Wipes what a session can leave behind for the next one. The sandbox is
/// long-lived, and Claude Code's memory directory is writable even under a
/// tools allowlist, so without this a run inherits the previous run's playbook
/// (observed: a distilled tactics file naming the score to beat). Credentials
/// are injected by the provider at exec time, not stored here, so ~/.claude
/// goes in full.
fn reset_agent_state(sandbox: &str) -> Result<(), String> {
    let script = "rm -rf \"$HOME\"/.local/share/opencode \"$HOME\"/.config/opencode \
                  \"$HOME\"/.local/state/opencode \"$HOME\"/.cache \
                  \"$HOME\"/.claude/projects \"$HOME\"/.claude/sessions \
                  \"$HOME\"/.claude/shell-snapshots \"$HOME\"/.claude/backups \
                  \"$HOME\"/.claude/session-env \"$HOME\"/.claude/todos \
                  \"${CODEX_HOME:-$HOME/.codex}\"; \
                  chmod -R u+w /workspace /tmp 2>/dev/null; \
                  rm -rf /workspace/* /workspace/.* /tmp/* 2>/dev/null; true";
    let mut cmd = Command::new("openshell");
    cmd.args(["sandbox", "exec", "-n", sandbox, "--"])
        .args(["sh", "-c", script]);
    match run_bounded(cmd, Duration::from_secs(120)) {
        Some(true) => Ok(()),
        Some(false) => Err(format!("reset {sandbox} failed")),
        None => Err(format!("reset {sandbox} timed out")),
    }
}

/// The revision of upstream OpenRCT2 this fork is based on. That tree is the
/// engine we build and score with, minus the harness, so it is what open note
/// hands to the agents.
fn upstream_merge_base(root: &Path) -> Result<String, String> {
    for upstream in ["upstream/develop", "origin/develop"] {
        let out = Command::new("git")
            .current_dir(root)
            .args(["merge-base", "HEAD", upstream])
            .output()
            .map_err(|e| format!("git merge-base: {e}"))?;
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !sha.is_empty() {
                return Ok(sha);
            }
        }
    }
    Err("no merge-base with upstream/develop or origin/develop".into())
}

/// Extracts upstream's tree with `git archive` (which cannot contain fork code:
/// that commit predates all of it) and uploads it into the sandbox. Cached per
/// revision on the host, so extra models and reruns skip the extract.
/// exec's stdin caps at 4 MiB, hence `sandbox upload`, which nests the local
/// directory under its destination: the staging directory is therefore named
/// after the sandbox path's last component and uploaded to its parent.
fn stage_open_note(root: &Path, sandbox: &str, sha: &str) -> Result<(), String> {
    let dest = Path::new(OPEN_NOTE_DIR);
    let (parent, name) = match (dest.parent(), dest.file_name()) {
        (Some(p), Some(n)) => (p, n),
        _ => return Err(format!("{OPEN_NOTE_DIR} is not a usable path")),
    };
    let stage = std::env::temp_dir()
        .join("coaster-open-note")
        .join(&sha[..12])
        .join(name);
    let probe = stage.join("src/openrct2/ride/RideRatings.cpp");
    if !probe.is_file() {
        let _ = std::fs::remove_dir_all(&stage);
        std::fs::create_dir_all(&stage).map_err(|e| format!("create {}: {e}", stage.display()))?;
        let mut archive = Command::new("git")
            .current_dir(root)
            .args(["archive", sha])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("git archive: {e}"))?;
        let source = archive
            .stdout
            .take()
            .ok_or_else(|| "git archive produced no output".to_string())?;
        let untar = Command::new("tar")
            .args([
                std::ffi::OsStr::new("x"),
                std::ffi::OsStr::new("-C"),
                stage.as_os_str(),
            ])
            .stdin(Stdio::from(source))
            .status()
            .map_err(|e| format!("tar: {e}"))?;
        let _ = archive.wait();
        if !untar.success() || !probe.is_file() {
            return Err(format!("extracting upstream {} failed", &sha[..12]));
        }
    }

    // An earlier run left the tree unwritable, which would also block deleting
    // it, so restore write permission before replacing it.
    let clear = format!("chmod -R u+w {OPEN_NOTE_DIR} 2>/dev/null; rm -rf {OPEN_NOTE_DIR}");
    let _ = Command::new("openshell")
        .args(["sandbox", "exec", "-n", sandbox, "--"])
        .args(["sh", "-c", &clear])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // --no-git-ignore: upstream's own .gitignore would otherwise silently drop
    // files from the tree the agent is told is the engine.
    let upload = Command::new("openshell")
        .args(["sandbox", "upload", "--no-git-ignore", sandbox])
        .arg(&stage)
        .arg(parent)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("openshell upload: {e}"))?;
    if !upload.success() {
        return Err(format!("upload into {sandbox} failed: {upload}"));
    }

    // Read-only, so one round cannot leave edits for the next. Verified by the
    // ratings source: without it the round would silently be a plain design
    // round with a misleading prompt.
    let finish = format!(
        "chmod -R a-w {OPEN_NOTE_DIR} && test -f {OPEN_NOTE_DIR}/src/openrct2/ride/RideRatings.cpp"
    );
    let ok = Command::new("openshell")
        .args(["sandbox", "exec", "-n", sandbox, "--"])
        .args(["sh", "-c", &finish])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("verify checkout: {e}"))?;
    if !ok.success() {
        return Err(format!(
            "{OPEN_NOTE_DIR} in {sandbox} has no ratings source"
        ));
    }
    Ok(())
}

/// Tools a session may call without a prompt. MCP only by default; open note
/// adds the file tools, without which the staged source is unreadable and the
/// prompt describing it is a lie (observed: every Read denied, turns burnt).
fn allowed_tools(args: &Args) -> String {
    let mut allowed = vec!["mcp__coaster__*".to_string()];
    if args.open_note {
        allowed.extend(["Read", "Grep", "Glob", "Bash"].map(str::to_string));
    }
    if let Some(extra) = &args.extra_tools {
        for tool in extra.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            if !allowed.iter().any(|a| a == tool) {
                allowed.push(tool.to_string());
            }
        }
    }
    allowed.join(",")
}

/// One agent session in the harness's sandbox. Success is judged by game
/// state, not the transcript; usage is captured best-effort either way.
/// Output streams to `session.log` / `session.err` in the round directory as
/// the session runs, so a round in progress can be tailed, and a round that
/// went wrong can be read afterwards. Files, not pipes: a session can outrun a
/// pipe buffer and deadlock while nobody is reading it.
fn run_agent_session(
    args: &Args,
    contender: &Contender,
    prompt: &str,
    round_dir: &Path,
    lease: &str,
    shutdown: &AtomicBool,
) -> SessionResult {
    let mut cmd = Command::new("openshell");
    let spend_before = contender
        .billed_by_openrouter()
        .then(openrouter_spend)
        .flatten();
    match contender.harness {
        Harness::ClaudeCode => {
            let mcp_config = json!({
                "mcpServers": {
                    "coaster": {"type": "http", "url": contender.mcp_url(args.port, lease, args.mode)}
                }
            });
            let claude = vec![
                "claude".to_string(),
                "--model".to_string(),
                contender
                    .claude_openrouter_model()
                    .unwrap_or(&contender.model)
                    .to_string(),
                "-p".to_string(),
                prompt.to_string(),
                "--mcp-config".to_string(),
                mcp_config.to_string(),
                "--allowedTools".to_string(),
                allowed_tools(args),
                "--max-turns".to_string(),
                args.max_turns.to_string(),
                // Event stream, not a summary: the round's trace is built from it.
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
            ];
            cmd.args(["sandbox", "exec", "-n", contender.sandbox(args), "--"]);
            match contender.claude_openrouter_model() {
                // OpenRouter answers the Anthropic Messages API, so Claude Code
                // drives a non-Anthropic model by base URL alone. The token
                // stays the gateway's placeholder, which the egress proxy
                // substitutes; the sandbox never holds the key. Telemetry is
                // off because this lane's policy grants OpenRouter and the game,
                // and nothing else, so the reporting hosts would only generate
                // denials.
                Some(_) => {
                    let script = concat!(
                        "export ANTHROPIC_BASE_URL=https://openrouter.ai/api\n",
                        "export ANTHROPIC_AUTH_TOKEN=\"$OPENROUTER_API_KEY\"\n",
                        "export CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1\n",
                        "export DISABLE_TELEMETRY=1 DISABLE_ERROR_REPORTING=1\n",
                        "exec \"$@\"",
                    );
                    cmd.args(["sh", "-c", script, "claude-session"])
                        .args(&claude);
                }
                None => {
                    cmd.args(&claude);
                }
            }
        }
        Harness::Opencode => {
            // opencode reads its MCP server list from a config file inside the
            // sandbox, so the per-contender endpoint has to be written there
            // before the session starts. Model comes fully qualified on the
            // command line (e.g. openrouter/poolside/...).
            if let Err(e) = write_opencode_config(args, contender, lease) {
                return SessionResult::failed(e);
            }
            cmd.args(["sandbox", "exec", "-n", &args.opencode_sandbox, "--"])
                .args(["opencode", "run", prompt, "-m", &contender.model])
                .args(["--format", "json"]);
        }
        Harness::Codex => {
            if let Err(e) = write_codex_config(args, contender, lease) {
                return SessionResult::failed(e);
            }
            let script = format!(
                "exec {} exec --json --skip-git-repo-check --cd \"$HOME\" \"$1\"",
                args.codex_bin
            );
            cmd.args(["sandbox", "exec", "-n", &args.codex_sandbox, "--"])
                .args(["sh", "-c", &script, "codex-session", prompt]);
        }
    }
    let log_path = round_dir.join("session.log");
    let err_path = round_dir.join("session.err");
    let (log, errlog) = match (File::create(&log_path), File::create(&err_path)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            return SessionResult::failed(format!(
                "cannot open session logs in {}",
                round_dir.display()
            ))
        }
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(errlog));

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return SessionResult::failed(format!("openshell exec: {e}")),
    };
    let budget = Duration::from_secs(args.session_timeout);
    let started = Instant::now();
    let mut timed_out = false;
    let mut interrupted = false;
    let mut hung = false;
    // Log length, not mtime: it only grows, so there is no clock to trust.
    let mut logged = 0u64;
    let mut last_write = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None)
                if started.elapsed() >= budget
                    || shutdown.load(Ordering::Relaxed)
                    || session_went_quiet(last_write.elapsed(), args.idle_timeout) =>
            {
                interrupted = shutdown.load(Ordering::Relaxed);
                timed_out = !interrupted && started.elapsed() >= budget;
                hung = !interrupted && !timed_out;
                let _ = child.kill();
                let status = child.wait().ok();
                // The local client is gone; the agent inside the sandbox is not.
                kill_stray_agents(contender.sandbox(args));
                break status;
            }
            Ok(None) => {
                std::thread::sleep(Duration::from_secs(1));
                let len = std::fs::metadata(&log_path)
                    .map(|m| m.len())
                    .unwrap_or(logged);
                if len != logged {
                    logged = len;
                    last_write = Instant::now();
                }
            }
            Err(e) => return SessionResult::failed(format!("waiting on agent session: {e}")),
        }
    };

    // Built even when the session failed or was killed: a partial event stream
    // still explains what the model was doing when it stopped.
    let stdout = std::fs::read_to_string(&log_path).unwrap_or_default();
    let trace = trace::parse(contender.harness, &stdout);
    if let Err(e) = std::fs::write(round_dir.join("trace.jsonl"), trace.to_jsonl()) {
        eprintln!("trace write failed: {e}");
    }
    let mut usage = trace.usage.clone();
    // opencode reports per-step cost, but only when the provider sends it back,
    // and Claude Code prices whatever it ran at Anthropic's rates (costBasis
    // "unknown" for anything else). The key's spend delta is the real number
    // for either, and is right for a run of one.
    let transcript_cost_is_wrong = contender.claude_openrouter_model().is_some();
    if spend_before.is_some()
        && (transcript_cost_is_wrong
            || usage.get("cost_usd").and_then(Value::as_f64).unwrap_or(0.0) == 0.0)
    {
        if let (Some(before), Some(after)) = (spend_before, openrouter_spend()) {
            if after >= before {
                usage["cost_usd"] = json!(after - before);
            }
        }
    }
    println!(
        "  {} trace events, {} tool calls",
        trace.events.len(),
        trace.tool_calls()
    );
    let error = if interrupted {
        Some("run interrupted by a signal; scoring whatever it built".to_string())
    } else if timed_out {
        Some(format!(
            "agent session hit the {}s timeout and was killed; scoring whatever it built",
            args.session_timeout
        ))
    } else if hung {
        Some(format!(
            "agent session wrote nothing for {}s and was killed as hung",
            args.idle_timeout
        ))
    } else if status.map(|s| s.success()).unwrap_or(false) {
        None
    } else {
        let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
        let stdout = stdout.as_str();
        let mut tail = format!("{} | {}", stderr.trim(), stdout.trim());
        tail.truncate(500);
        let code = status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "killed".into());
        Some(format!("agent session failed ({code}): {tail}"))
    };
    SessionResult { usage, error, hung }
}

/// Post-round artifact collection over MCP; agent may or may not have called
/// finish_and_test itself (calling it again only adds test ticks).
fn collect_round(
    client: &mut mcp::McpClient,
    control: &mut mcp::McpClient,
    args: &Args,
    round_dir: &Path,
) -> Result<Value, String> {
    std::fs::create_dir_all(round_dir).map_err(|e| e.to_string())?;

    // Both calls can legitimately fail (no ride built, open circuit); the
    // round record must exist either way so the site can show what happened.
    let state = client.call("get_state", json!({})).unwrap_or(json!({}));
    let placed = state.get("pieces_placed").cloned().unwrap_or(json!(0));
    // Test the current park: catches a round where the agent never called
    // finish_and_test itself, and updates the server's best-so-far.
    let final_report = client
        .call("finish_and_test", json!({"ticks": args.ticks}))
        .unwrap_or_else(|e| {
            json!({
                "program": {
                    "ok": false,
                    "pieces_placed": placed,
                    "pieces_total": placed,
                    "error": {"piece_index": null, "message": e},
                },
                "rides": [],
                "similarity": null,
            })
        });
    // Score the best tested coaster of the round, not whatever happens to be
    // standing now: a model that demolished a good ride to try a better one
    // should keep the good score. best_result errors when nothing rated, in
    // which case the final-park report (with its error) is what we record.
    let (report, scored_from_best) = match client.call("best_result", json!({})) {
        Ok(best) => (best, true),
        _ => (final_report, false),
    };

    // Reconstruct program.json for the piece listing. The scored report
    // carries its own placed_pieces/start (finish_and_test snapshots them), so
    // this describes the scored coaster even when it has since been rebuilt;
    // fall back to the live state for pre-snapshot reports.
    let pieces = report
        .get("placed_pieces")
        .or_else(|| state.get("placed_pieces"))
        .cloned()
        .unwrap_or(json!([]));
    let start = report
        .get("start")
        .or_else(|| state.get("start"))
        .cloned()
        .unwrap_or(Value::Null);
    // start already comes from cursor_json, which reports tile coordinates
    // (it divides raw game units by 32); the program format wants tiles, so
    // pass them straight through — dividing again put every start at (1, 1).
    let program = json!({
        "ride_type": args.ride_type,
        "start": {
            "x": start.get("x").and_then(Value::as_i64).unwrap_or(0),
            "y": start.get("y").and_then(Value::as_i64).unwrap_or(0),
            "dir": start.get("dir").and_then(Value::as_i64).unwrap_or(0),
        },
        "pieces": pieces,
    });
    write_json(&round_dir.join("program.json"), &program)?;
    write_json(&round_dir.join("report.json"), &report)?;

    // Server-side writes from here, hence absolute paths: the game is a
    // separate process with its own working directory. Control plane, not the
    // agents' MCP endpoint, because these write this filesystem.
    let dir = std::fs::canonicalize(round_dir)
        .map_err(|e| format!("resolve {}: {e}", round_dir.display()))?;
    // Picture the scored coaster, cropped to the track: `best` when the score
    // came from an earlier (possibly demolished) build, otherwise the park as
    // it stands. The agent's own screenshot tool renders the whole map, which
    // is right for building and wrong for a thumbnail.
    let shot_path = dir.join("park.png");
    control
        .call(
            "capture_park",
            json!({"path": shot_path, "best": scored_from_best}),
        )
        .map_err(|e| format!("capture scored park: {e}"))?;

    // Save the same candidate as the report and picture, so the result can be
    // reopened and checked rather than taken on the report's word.
    let park_path = dir.join("park.park");
    control
        .call(
            "save_park",
            json!({"path": park_path, "best": scored_from_best}),
        )
        .map_err(|e| format!("save scored park: {e}"))?;
    // Saved before the replay, because capturing one ticks the simulation on.
    if args.replay_seconds > 0 {
        if let Err(e) = capture_replay(
            control,
            round_dir,
            replay_seconds(&report, args),
            scored_from_best,
        ) {
            eprintln!("  replay failed: {e}");
        }
    }
    Ok(report)
}

/// Records the running ride as a video: frames from the game, encoded by
/// ffmpeg, frames thrown away.
///
/// The camera is the track's bounding box and never moves, so consecutive
/// frames differ only where the train is. That is what makes this cheap to
/// keep: measured on a test circuit, 20 seconds came to 255 KB, and ten times
/// the frames cost under four times the bytes. The PNGs in between are large
/// (13 MB for those 400 frames) and are deleted once encoded.
/// How long to record: the lap time the game itself measured during the test
/// ("Ride time" in the ride window, the sum of the stations' segment times),
/// plus a little so the train is seen arriving rather than cut off mid-air.
/// Falls back to the cap when the ride never completed a test and so has no
/// measured time.
fn replay_seconds(report: &Value, args: &Args) -> u32 {
    const TAIL: i64 = 3;
    let lap = report
        .get("rides")
        .and_then(Value::as_array)
        .and_then(|rides| {
            rides
                .iter()
                .filter_map(|r| r.get("ride_time")?.as_i64())
                .max()
        })
        .unwrap_or(0);
    if lap <= 0 {
        return args.replay_seconds;
    }
    u32::try_from(lap + TAIL)
        .unwrap_or(args.replay_seconds)
        .min(args.replay_seconds)
}

fn capture_replay(
    control: &mut mcp::McpClient,
    round_dir: &Path,
    seconds: u32,
    best: bool,
) -> Result<(), String> {
    const FPS: u32 = 20;
    // 40 game ticks make a second, so every other tick is 20fps.
    let frames = seconds * FPS;
    let out = std::fs::canonicalize(round_dir)
        .map(|d| d.join("replay.mp4"))
        .map_err(|e| format!("resolve {}: {e}", round_dir.display()))?;
    // The game renders straight into ffmpeg, so there are no frames on disk to
    // clean up and nothing is PNG-encoded on the way.
    control.call(
        "capture_replay",
        json!({"out": out, "frames": frames, "every_ticks": 2, "zoom": 0, "best": best}),
    )?;
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use std::io::Read;

    let mut file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn command_stdout(program: &str, args: &[&str], cwd: &Path) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_provenance(root: &Path) -> Value {
    let commit = command_stdout("git", &["rev-parse", "HEAD"], root);
    let diff = Command::new("git")
        .args([
            "diff",
            "--binary",
            "HEAD",
            "--",
            "rust/coaster-bench",
            "rust/orct2-agent",
            "src/openrct2/rustbridge",
            "src/openrct2/command_line/EvalCommands.cpp",
            "src/openrct2/interface/Screenshot.cpp",
        ])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
        .unwrap_or_default();
    json!({
        "commit": commit,
        "dirty": !diff.is_empty(),
        "dirty_diff_sha256": (!diff.is_empty()).then(|| sha256_bytes(&diff)),
    })
}

fn image_id(image: &str, root: &Path) -> Option<String> {
    ["podman", "docker"].iter().find_map(|runtime| {
        command_stdout(
            runtime,
            &["image", "inspect", "--format", "{{.Id}}", image],
            root,
        )
        .filter(|id| !id.is_empty())
    })
}

fn bounded_output(mut command: Command, budget: Duration) -> Option<std::process::Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return None,
        }
    }
}

fn agent_version(args: &Args, contender: &Contender) -> Option<String> {
    // Read Node package metadata instead of invoking those agents: notably,
    // `opencode --version` performs network startup and can hang behind a
    // restricted policy.
    let (binary, version_args): (&str, Vec<&str>) = match contender.harness {
        Harness::ClaudeCode => (
            "/usr/local/bin/node",
            vec![
                "-p",
                "require('/usr/local/lib/node_modules/@anthropic-ai/claude-code/package.json').version",
            ],
        ),
        Harness::Opencode => (
            "/usr/local/bin/node",
            vec![
                "-p",
                "require('/usr/local/lib/node_modules/opencode-ai/package.json').version",
            ],
        ),
        Harness::Codex => (args.codex_bin.as_str(), vec!["--version"]),
    };
    let mut command = Command::new("openshell");
    command
        .args(["sandbox", "exec", "-n", contender.sandbox(args), "--"])
        .arg(binary)
        .args(version_args);
    let output = bounded_output(command, Duration::from_secs(15))?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Some(if stdout.is_empty() { stderr } else { stdout }).filter(|v| !v.is_empty())
}

fn best_excitement(report: &Value) -> Option<f64> {
    // Interactive reports carry the scorer's authoritative value. Historical
    // and whole-program driver reports predate it, so retain the exact fallback
    // formula for those artifacts.
    if let Some(score) = report.get("score").and_then(Value::as_f64) {
        return Some(score);
    }
    let rides = report.get("rides")?.as_array()?;
    let raw = rides
        .iter()
        .filter_map(|r| r.get("excitement").and_then(Value::as_f64))
        .reduce(f64::max)?;
    // Same penalty math as driver.py (SIMILARITY_GRACE = 0.5).
    let similarity = report
        .pointer("/similarity/similarity")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let multiplier = if similarity <= 0.5 {
        1.0
    } else {
        ((1.0 - similarity) / 0.5).max(0.0)
    };
    Some(raw * multiplier)
}

/// Wall-clock seconds, used to make each round's lease unique across restarts.
fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn today() -> String {
    // No chrono dep for one date string: days since epoch -> civil date.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}{m:02}{d:02}")
}

fn read_json(path: &Path) -> Result<Value, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
}

fn archived_contender(round_dir: &Path) -> Result<Contender, String> {
    let usage = read_json(&round_dir.join("usage.json"))?;
    let model = usage
        .get("model")
        .and_then(Value::as_str)
        .ok_or("archived usage.json has no model")?;
    let harness = usage
        .get("harness")
        .and_then(Value::as_str)
        .ok_or("archived usage.json has no harness")?;
    let spec = match harness {
        "codex" => format!("codex:{model}"),
        "opencode" => format!("opencode:{model}"),
        "claude-code" => model.to_string(),
        other => return Err(format!("unsupported archived harness: {other}")),
    };
    Ok(Contender::parse(&spec))
}

fn archived_run_path(round_dir: &Path) -> Result<PathBuf, String> {
    let model_dir = round_dir
        .parent()
        .ok_or("round directory has no model parent")?;
    let run_dir = model_dir
        .parent()
        .ok_or("round directory has no run parent")?;
    let path = run_dir.join("run.json");
    path.is_file()
        .then_some(path)
        .ok_or_else(|| "archived run.json not found two levels above round".to_string())
}

fn use_archived_settings(args: &mut Args, run: &Value) {
    if let Some(scenario) = run.pointer("/scenario/path").and_then(Value::as_str) {
        args.scenario = scenario.to_string();
    }
    if let Some(ticks) = run.get("ticks").and_then(Value::as_u64) {
        args.ticks = ticks.min(u32::MAX as u64) as u32;
    }
    if let Some(seconds) = run
        .pointer("/budgets/replay_cap_seconds")
        .and_then(Value::as_u64)
    {
        args.replay_seconds = seconds.min(u32::MAX as u64) as u32;
    }
    if let Some(effort) = run.get("codex_reasoning_effort").and_then(Value::as_str) {
        args.codex_reasoning_effort = match effort {
            "low" => CodexReasoningEffort::Low,
            "high" => CodexReasoningEffort::High,
            "xhigh" => CodexReasoningEffort::Xhigh,
            "max" => CodexReasoningEffort::Max,
            "ultra" => CodexReasoningEffort::Ultra,
            _ => CodexReasoningEffort::Medium,
        };
    }
}

fn presentation_prompt(model: &str, source_report: &Value) -> String {
    let ride = source_report
        .get("rides")
        .and_then(Value::as_array)
        .and_then(|rides| rides.first())
        .cloned()
        .unwrap_or(Value::Null);
    format!(
        "This is a presentation-only epilogue for the roller coaster you already designed in a \
         completed CoasterBench run. You are {model}. The layout and its score are frozen. Only \
         three coaster tools are available: inspect best_result, inspect best_screenshot, then call \
         style_best_ride exactly once with a distinctive final name (32 characters maximum) and a \
         cohesive track, rail, and support colour scheme that suits what you built. Do not ask for \
         confirmation; make the creative decision yourself. End with one sentence explaining the \
         name and palette.\n\nOriginal ride measurements: {ride}"
    )
}

fn write_presentation_usage(
    path: &Path,
    contender: &Contender,
    session: &SessionResult,
) -> Result<(), String> {
    write_json(
        path,
        &json!({
            "harness": contender.harness.name(),
            "model": contender.model,
            "input_tokens": session.usage.get("input_tokens"),
            "output_tokens": session.usage.get("output_tokens"),
            "cache_read_tokens": session.usage.get("cache_read_tokens"),
            "cost_usd": session.usage.get("cost_usd"),
            "num_turns": session.usage.get("num_turns"),
        }),
    )
}

struct PresentationStage(PathBuf);

impl Drop for PresentationStage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Reconstructs an archived winner, proves it still has the recorded score,
/// then gives the original model a capability-limited naming/colour pass.
/// Artifacts are staged and replace the published set only after every capture
/// succeeds, so a failed model turn cannot damage the scored result.
fn present_archived_round(
    args: &mut Args,
    root: &Path,
    round_arg: &Path,
    shutdown: &AtomicBool,
) -> Result<(), String> {
    let round_dir = std::fs::canonicalize(round_arg)
        .map_err(|e| format!("resolve {}: {e}", round_arg.display()))?;
    let program = read_json(&round_dir.join("program.json"))?;
    let source_report_path = round_dir.join("report.json");
    let source_report = read_json(&source_report_path)?;
    let source_score =
        best_excitement(&source_report).ok_or("archived report has no scored ride")?;
    let source_report_sha256 = sha256_file(&source_report_path)?;
    let source_park_sha256 = sha256_file(&round_dir.join("park.park")).ok();
    let run = read_json(&archived_run_path(&round_dir)?)?;
    use_archived_settings(args, &run);
    if let Some(expected) = run.pointer("/scenario/sha256").and_then(Value::as_str) {
        let scenario = expand_home(&args.scenario);
        let actual = sha256_file(&scenario)?;
        if actual != expected {
            return Err(format!(
                "scenario hash mismatch for {}: archived {expected}, current {actual}",
                scenario.display()
            ));
        }
    }
    args.mode = BenchmarkMode::Presentation;
    args.open_note = false;
    args.extra_tools = None;

    let contender = archived_contender(&round_dir)?;
    let model = contender.display();
    println!(
        "presentation pass: {model}, archived score {source_score:.2}, {}",
        round_dir.display()
    );

    // Same ephemeral-sandbox contract as a run: the guard deletes it on the
    // way out, and the lane's base name is swapped for the fresh one.
    let mut _ephemeral: Option<EphemeralSandbox> = None;
    if args.fresh_sandbox {
        let codex_providers = if contender.codex_openrouter_model().is_some() {
            format!("{},openrouter", args.codex_sandbox_provider)
        } else {
            args.codex_sandbox_provider.clone()
        };
        let (image, policy, provider, base) = match contender.lane() {
            Lane::ClaudeCode => (
                args.sandbox_image.clone(),
                args.sandbox_policy.clone(),
                args.sandbox_provider.clone(),
                args.sandbox.clone(),
            ),
            Lane::ClaudeCodeOpenRouter => (
                args.claude_openrouter_sandbox_image.clone(),
                args.claude_openrouter_sandbox_policy.clone(),
                "openrouter".to_string(),
                args.claude_openrouter_sandbox.clone(),
            ),
            Lane::Opencode => (
                args.opencode_sandbox_image.clone(),
                args.opencode_sandbox_policy.clone(),
                args.opencode_sandbox_provider.clone(),
                args.opencode_sandbox.clone(),
            ),
            Lane::Codex => (
                args.codex_sandbox_image.clone(),
                args.codex_sandbox_policy.clone(),
                codex_providers,
                args.codex_sandbox.clone(),
            ),
        };
        let recipe = SandboxRecipe {
            image: &image,
            policy: &policy,
            provider: &provider,
        };
        let name = ephemeral_sandbox_name(&base, epoch_secs());
        println!("creating sandbox {name} from {image}");
        _ephemeral = Some(create_sandbox(&recipe, args.port, root, &name)?);
        match contender.harness {
            Harness::ClaudeCode => args.sandbox = name,
            Harness::Opencode => args.opencode_sandbox = name,
            Harness::Codex => args.codex_sandbox = name,
        }
    }

    let _server = GameServer::spawn(args, root)?;
    let mut client = mcp::McpClient::new("127.0.0.1", args.port);
    wait_for_server(&mut client, Duration::from_secs(120))?;
    let mut control = mcp::McpClient::new("127.0.0.1", args.control_port);
    wait_for_server(&mut control, Duration::from_secs(120))?;

    let lease = format!("present-{model}-{}", epoch_secs());
    client.claim("127.0.0.1", args.port, &lease);
    let start = program
        .get("start")
        .ok_or("archived program has no start")?;
    client.call(
        "new_ride",
        json!({
            "ride_type": program.get("ride_type").and_then(Value::as_u64)
                .ok_or("archived program has no ride_type")?,
            "x": start.get("x").and_then(Value::as_i64).ok_or("program start has no x")?,
            "y": start.get("y").and_then(Value::as_i64).ok_or("program start has no y")?,
            "dir": start.get("dir").and_then(Value::as_i64).ok_or("program start has no dir")?,
        }),
    )?;
    let pieces = program
        .get("pieces")
        .and_then(Value::as_array)
        .ok_or("archived program has no piece array")?;
    // place_pieces caps a batch at 200; archived programs run longer.
    let mut placed_count = 0usize;
    let mut expected = 0usize;
    for chunk in pieces.chunks(200) {
        let placed = client.call("place_pieces", json!({"pieces": chunk}))?;
        placed_count += placed
            .get("placed_this_call")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        expected += chunk.len();
        if placed_count != expected {
            return Err(format!(
                "archived reconstruction placed {placed_count}/{} pieces: {placed}",
                pieces.len()
            ));
        }
    }
    let rebuilt = client.call("finish_and_test", json!({"ticks": args.ticks}))?;
    let rebuilt_score =
        best_excitement(&rebuilt).ok_or("archived reconstruction did not produce a score")?;
    if (rebuilt_score - source_score).abs() > 0.005 {
        return Err(format!(
            "archived score mismatch: recorded {source_score:.2}, rebuilt {rebuilt_score:.2}; \
             refusing to restyle different ride physics"
        ));
    }
    println!("reconstruction verified at {rebuilt_score:.2}");

    let presentation_dir = round_dir.join("presentation");
    std::fs::create_dir_all(&presentation_dir)
        .map_err(|e| format!("create {}: {e}", presentation_dir.display()))?;
    reset_agent_state(contender.sandbox(args))?;
    let version = agent_version(args, &contender);
    let prompt = presentation_prompt(&model, &source_report);
    let session = run_agent_session(
        args,
        &contender,
        &prompt,
        &presentation_dir,
        &lease,
        shutdown,
    );
    write_presentation_usage(&presentation_dir.join("usage.json"), &contender, &session)?;

    let styled_report = client.call("best_result", json!({}))?;
    let presentation = styled_report.get("presentation").cloned().ok_or_else(|| {
        format!(
            "model did not style the ride{}",
            session
                .error
                .as_deref()
                .map(|e| format!(": {e}"))
                .unwrap_or_default()
        )
    })?;
    let styled_score = best_excitement(&styled_report).ok_or("styled report lost its score")?;
    if (styled_score - source_score).abs() > f64::EPSILON {
        return Err(format!(
            "presentation changed the score from {source_score} to {styled_score}"
        ));
    }
    if args.replay_seconds == 0 {
        return Err("presentation pass requires replay capture to refresh coaster colours".into());
    }

    let stage = round_dir.join(format!(
        ".presentation-stage-{}-{}",
        std::process::id(),
        epoch_secs()
    ));
    std::fs::create_dir(&stage).map_err(|e| format!("create {}: {e}", stage.display()))?;
    let _stage_cleanup = PresentationStage(stage.clone());
    write_json(&stage.join("report.json"), &styled_report)?;
    control.call(
        "capture_park",
        json!({"path": stage.join("park.png"), "best": true}),
    )?;
    control.call(
        "save_park",
        json!({"path": stage.join("park.park"), "best": true}),
    )?;
    capture_replay(
        &mut control,
        &stage,
        replay_seconds(&styled_report, args),
        true,
    )?;
    for name in [
        "report.json",
        "park.png",
        "park.park",
        "replay.mp4",
        "replay.png",
        "replay.json",
    ] {
        let staged = stage.join(name);
        if !staged.is_file() {
            return Err(format!(
                "presentation artifact missing: {}",
                staged.display()
            ));
        }
    }
    for name in [
        "report.json",
        "park.png",
        "park.park",
        "replay.mp4",
        "replay.png",
        "replay.json",
    ] {
        std::fs::rename(stage.join(name), round_dir.join(name))
            .map_err(|e| format!("install presentation artifact {name}: {e}"))?;
    }
    std::fs::remove_dir(&stage).map_err(|e| format!("remove {}: {e}", stage.display()))?;

    let provenance = json!({
        "kind": "posthoc-presentation",
        "model": contender.model,
        "harness": contender.harness.name(),
        "agent_version": version,
        "reasoning_effort": (contender.harness == Harness::Codex)
            .then(|| args.codex_reasoning_effort.as_str()),
        "score_before": source_score,
        "score_after": styled_score,
        "source_report_sha256": source_report_sha256,
        "source_park_sha256": source_park_sha256,
        "styled_report_sha256": sha256_file(&round_dir.join("report.json"))?,
        "styled_park_sha256": sha256_file(&round_dir.join("park.park"))?,
        "presentation": presentation,
        "session_error": session.error,
        "completed_at": rfc3339_now(),
    });
    write_json(&presentation_dir.join("pass.json"), &provenance)?;
    println!("model-authored presentation committed locally: {presentation}");
    Ok(())
}

/// Howard Hinnant's days-from-civil inverse, public domain algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn main() -> Result<(), String> {
    let mut args = Args::parse();
    if args.control_port == 0 {
        return Err(
            "--control-port must be non-zero: round reset and artifact collection require it"
                .into(),
        );
    }
    if args.control_port == args.port {
        return Err("agent and control ports must be different".into());
    }
    if args.open_note {
        if args.mode == BenchmarkMode::Library {
            return Err(
                "--open-note cannot be combined with --mode library; use one benchmark condition"
                    .into(),
            );
        }
        args.mode = BenchmarkMode::OpenNote;
    }
    args.open_note = args.mode == BenchmarkMode::OpenNote;
    let root = repo_root();
    let shutdown = install_shutdown_flag()?;
    if let Some(round_dir) = args.present_round.clone() {
        return present_archived_round(&mut args, &root, &round_dir, &shutdown);
    }

    // Debug aid: print the round-1 prompt verbatim and exit. Built from the
    // first contender: harness and modality are what vary, and printing it
    // with fixed defaults hid a missing one-shot warning for six rounds.
    if std::env::var_os("COASTER_BENCH_PRINT_PROMPT").is_some() {
        let contender = args
            .models
            .first()
            .map(|s| Contender::parse(s))
            .ok_or("no model to print a prompt for")?;
        print!(
            "{}",
            prompt::round_prompt(&prompt::Round {
                ride_type: args.ride_type,
                round: 1,
                rounds: args.rounds,
                previous_feedback: None,
                modalities: contender.modalities,
                budget_secs: args.session_timeout,
                open_note_dir: args.open_note.then_some(OPEN_NOTE_DIR),
                single_turn: contender.harness.single_turn(),
            })
        );
        return Ok(());
    }

    // A resumed run keeps its original directory, and with it the rounds
    // already in the archive; a fresh one is always dated today. Reusing
    // --name on the same day would land in an existing run and overwrite its
    // rounds one at a time, so resuming is deliberate rather than implied.
    let (run_dir, resuming) = match &args.resume {
        Some(dir) => {
            let dir =
                std::fs::canonicalize(dir).map_err(|e| format!("resume {}: {e}", dir.display()))?;
            if !dir.join("run.json").is_file() {
                return Err(format!(
                    "resume {}: no run.json, so this is not a run directory",
                    dir.display()
                ));
            }
            (dir, true)
        }
        None => {
            let suffix = args.name.clone().unwrap_or_else(|| "bench".into());
            let dir = root
                .join("evals/runs")
                .join(format!("{}-{}", today(), suffix));
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            (dir, false)
        }
    };
    println!(
        "run dir: {} (ride_type {}){}",
        run_dir.display(),
        args.ride_type,
        if resuming { ", resuming" } else { "" }
    );

    let _server = if args.attach {
        None
    } else {
        Some(GameServer::spawn(&args, &root)?)
    };
    let mut client = mcp::McpClient::new("127.0.0.1", args.port);
    wait_for_server(&mut client, Duration::from_secs(120))?;
    let mut control = mcp::McpClient::new("127.0.0.1", args.control_port);
    wait_for_server(&mut control, Duration::from_secs(120))?;
    println!("game server ready on port {}", args.port);

    let contenders: Vec<Contender> = args.models.iter().map(|s| Contender::parse(s)).collect();
    let mut _ephemeral: Vec<EphemeralSandbox> = Vec::new();
    let codex_providers = if contenders
        .iter()
        .any(|c| c.codex_openrouter_model().is_some())
    {
        format!("{},openrouter", args.codex_sandbox_provider)
    } else {
        args.codex_sandbox_provider.clone()
    };
    if args.fresh_sandbox {
        let lanes = [
            (
                Lane::ClaudeCode,
                SandboxRecipe {
                    image: &args.sandbox_image,
                    policy: &args.sandbox_policy,
                    provider: &args.sandbox_provider,
                },
                args.sandbox.clone(),
            ),
            (
                Lane::ClaudeCodeOpenRouter,
                SandboxRecipe {
                    image: &args.claude_openrouter_sandbox_image,
                    policy: &args.claude_openrouter_sandbox_policy,
                    provider: "openrouter",
                },
                args.claude_openrouter_sandbox.clone(),
            ),
            (
                Lane::Opencode,
                SandboxRecipe {
                    image: &args.opencode_sandbox_image,
                    policy: &args.opencode_sandbox_policy,
                    provider: &args.opencode_sandbox_provider,
                },
                args.opencode_sandbox.clone(),
            ),
            (
                Lane::Codex,
                SandboxRecipe {
                    image: &args.codex_sandbox_image,
                    policy: &args.codex_sandbox_policy,
                    provider: &codex_providers,
                },
                args.codex_sandbox.clone(),
            ),
        ];
        let mut renamed: Vec<(Lane, String)> = Vec::new();
        for (lane, recipe, base) in &lanes {
            if !contenders.iter().any(|c| c.lane() == *lane) {
                continue;
            }
            let name = ephemeral_sandbox_name(base, epoch_secs());
            println!("creating sandbox {name} from {}", recipe.image);
            _ephemeral.push(create_sandbox(recipe, args.port, &root, &name)?);
            renamed.push((*lane, name));
        }
        for (lane, name) in renamed {
            match lane {
                Lane::ClaudeCode => args.sandbox = name,
                Lane::ClaudeCodeOpenRouter => args.claude_openrouter_sandbox = name,
                Lane::Opencode => args.opencode_sandbox = name,
                Lane::Codex => args.codex_sandbox = name,
            }
        }
    }
    // Contenders share sandboxes, so per-sandbox setup runs once each.
    let sandboxes: BTreeSet<&str> = contenders.iter().map(|c| c.sandbox(&args)).collect();
    // A previous run killed from the outside can leave an agent alive in its
    // sandbox, still building in the park this run is about to use.
    for sandbox in &sandboxes {
        kill_stray_agents(sandbox);
    }
    for sandbox in contenders
        .iter()
        .map(|c| c.sandbox(&args))
        .collect::<std::collections::BTreeSet<_>>()
    {
        reset_agent_state(sandbox)?;
        println!("reset agent state in {sandbox}");
    }
    let open_note_sha = if args.open_note {
        let sha = upstream_merge_base(&root)?;
        for sandbox in &sandboxes {
            stage_open_note(&root, sandbox, &sha)?;
            println!(
                "open note: {OPEN_NOTE_DIR} in {sandbox} at upstream {}",
                &sha[..12]
            );
        }
        Some(sha)
    } else {
        None
    };
    for c in &contenders {
        if !c.modalities.contains(Modalities::IMAGE) {
            println!(
                "[{}] modalities {}: screenshot tool hidden",
                c.display(),
                c.modalities.as_query_value()
            );
        }
    }
    let modalities: Value = contenders
        .iter()
        .map(|c| (c.display(), json!(c.modalities.as_query_value())))
        .collect::<serde_json::Map<_, _>>()
        .into();
    let harnesses: Value = contenders
        .iter()
        .map(|c| (c.display(), json!(c.harness.name())))
        .collect::<serde_json::Map<_, _>>()
        .into();
    let run_harness = if contenders
        .iter()
        .all(|c| c.harness == contenders[0].harness)
    {
        contenders[0].harness.name().to_string()
    } else {
        "mixed".to_string()
    };
    let mut capabilities = Vec::new();
    if args.mode == BenchmarkMode::Library {
        capabilities.push("track_library");
    }
    if args.open_note {
        capabilities.extend(["engine_source", "file_tools", "python3", "shell"]);
    }
    // Recorded even when no condition granted it, because the codex lane has a
    // shell regardless and a run that claims none would be describing the
    // opencode lane's constraints for everyone.
    if !capabilities.contains(&"shell") && contenders.iter().any(|c| has_shell(&args, c)) {
        capabilities.push("shell");
    }
    let model_specs: Value = contenders
        .iter()
        .map(|contender| {
            (
                contender.display(),
                json!({
                    "id": contender.model,
                    "harness": contender.harness.name(),
                    "provider": contender.provider(&args),
                    "modalities": contender.modalities.as_query_value(),
                    "agent_version": agent_version(&args, contender),
                    "reasoning_effort": (contender.harness == Harness::Codex)
                        .then(|| args.codex_reasoning_effort.as_str()),
                    "shell": has_shell(&args, contender),
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into();
    let scenario_path = expand_home(&args.scenario);
    let scenario_sha256 = sha256_file(&scenario_path)?;
    let sandbox_images = json!({
        "claude_code": {
            "reference": args.sandbox_image,
            "image_id": image_id(&args.sandbox_image, &root),
        },
        "claude_code_openrouter": {
            "reference": args.claude_openrouter_sandbox_image,
            "image_id": image_id(&args.claude_openrouter_sandbox_image, &root),
        },
        "opencode": {
            "reference": args.opencode_sandbox_image,
            "image_id": image_id(&args.opencode_sandbox_image, &root),
        },
        "codex": {
            "reference": args.codex_sandbox_image,
            "image_id": image_id(&args.codex_sandbox_image, &root),
        },
    });
    let environment = json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "rustc": command_stdout("rustc", &["--version"], &root),
        "sandbox_images": sandbox_images,
    });
    // Where each model picks up. Read before run.json is rewritten, because on
    // a resume that file is the thing being extended.
    let resume_points: Vec<(String, u32)> = contenders
        .iter()
        .map(|c| {
            let model = c.display();
            let done = if resuming {
                completed_rounds(&run_dir, &model)
            } else {
                0
            };
            (model, done)
        })
        .collect();
    let existing_run = resuming
        .then(|| read_json(&run_dir.join("run.json")))
        .transpose()?;
    if let Some(existing) = &existing_run {
        check_resume_compatible(existing, &args, &contenders)?;
        for (model, done) in &resume_points {
            println!(
                "[{model}] resuming: {done} rounds archived, building {done_plus}..={total}",
                done_plus = done + 1,
                total = args.rounds
            );
        }
    }
    // One entry per time someone pressed go. A continued run is built by more
    // than one harness build, and averaging that away in a single source field
    // would make the record claim rounds it did not produce.
    let segment = json!({
        "started_at": rfc3339_now(),
        "first_round": resume_points.iter().map(|(m, done)| (m.clone(), json!(done + 1)))
            .collect::<serde_json::Map<_, _>>(),
        "rounds": args.rounds,
        "harness_version": env!("CARGO_PKG_VERSION"),
        "source": git_provenance(&root),
        "environment": environment.clone(),
        "openrouter_provider": args.openrouter_provider,
    });
    let mut run_json = json!({
            "schema_version": RUN_SCHEMA_VERSION,
            "condition": args.mode.condition_name(),
            "mode": args.mode.base_mode(),
            "orchestrator": "coaster-bench",
            "harness_version": env!("CARGO_PKG_VERSION"),
            "scorer": {
                "name": "adjusted_excitement",
                "version": SCORER_VERSION,
                "similarity_grace": 0.5,
            },
            "source": git_provenance(&root),
            "harness": run_harness,
            "harnesses": harnesses,
            "models": contenders.iter().map(Contender::display).collect::<Vec<_>>(),
            "model_specs": model_specs,
            "modalities": modalities,
            "rounds": args.rounds,
            "ticks": args.ticks,
            "ride_type": args.ride_type,
            "similarity_grace": 0.5,
            "scenario": {
                "path": args.scenario,
                "sha256": scenario_sha256,
            },
            "budgets": {
                "rounds": args.rounds,
                "simulation_ticks": args.ticks,
                "session_timeout_seconds": args.session_timeout,
                "idle_timeout_seconds": args.idle_timeout,
                "idle_retries": args.idle_retries,
                "max_agent_turns": args.max_turns,
                "replay_cap_seconds": args.replay_seconds,
                "replay_fps": 20,
            },
            "environment": environment,
            "open_note": args.open_note,
            // What the condition actually granted, so a run explains itself
            // without cross-referencing the harness version.
            "capabilities": capabilities,
            "agent_state": if args.fresh_sandbox { "fresh-sandbox" } else { "reset-per-run" },
            "round_reset": "baseline-park-snapshot",
            "artifact_integrity": "sha256",
            "sandbox": args.sandbox,
            "claude_openrouter_sandbox": args.claude_openrouter_sandbox,
            "opencode_sandbox": args.opencode_sandbox,
            "codex_sandbox": args.codex_sandbox,
            "allowed_tools": allowed_tools(&args),
            "opencode_permission": opencode_permissions(&args),
            "openrouter_provider": args.openrouter_provider,
            "codex_sandbox_mode": codex_sandbox_mode(&args),
            "codex_reasoning_effort": args.codex_reasoning_effort.as_str(),
            "open_note_source": open_note_sha,
    });
    if let Some(mut existing) = existing_run {
        // Keep the original record and extend it. Rewriting it from today's
        // flags would restamp rounds this build never produced with this
        // build's commit, images and version.
        let mut segments = existing
            .get("segments")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![original_segment(&existing)]);
        segments.push(segment);
        existing["segments"] = Value::Array(segments);
        existing["rounds"] = json!(args.rounds);
        existing["budgets"]["rounds"] = json!(args.rounds);
        run_json = existing;
    } else {
        run_json["segments"] = json!([segment]);
    }
    write_json(&run_dir.join("run.json"), &run_json)?;

    let mut standings: Vec<Value> = Vec::new();
    'contenders: for (contender, (_, done)) in contenders.iter().zip(&resume_points) {
        let model = contender.display();
        // Carried across the gap, so a continued run keeps its winner and the
        // next prompt still quotes the round before it.
        let (mut best, mut feedback) = if *done > 0 {
            resumed_state(&run_dir, &model, *done)
        } else {
            (None, None)
        };
        for round in (done + 1)..=args.rounds {
            if shutdown.load(Ordering::Relaxed) {
                eprintln!("interrupted: stopping before {model} round {round}");
                break 'contenders;
            }
            let prompt = prompt::round_prompt(&prompt::Round {
                ride_type: args.ride_type,
                round,
                rounds: args.rounds,
                previous_feedback: feedback.as_deref(),
                modalities: contender.modalities,
                budget_secs: args.session_timeout,
                open_note_dir: open_note_sha.as_ref().map(|_| OPEN_NOTE_DIR),
                single_turn: contender.harness.single_turn(),
            });
            // Created up front: the session streams its logs in here while it
            // runs, so `tail -f` works on a round in progress.
            let round_dir = run_dir.join(&model).join(format!("round_{round}"));
            if let Err(e) = std::fs::create_dir_all(&round_dir) {
                return Err(format!("create {}: {e}", round_dir.display()));
            }
            println!(
                "[{model}] round {round}: agent session starting (log: {})",
                round_dir.join("session.log").display()
            );
            // A quiet upstream says nothing about the model, so the round is
            // rebuilt from the pristine park rather than banked.
            let mut hung_logs: Vec<String> = Vec::new();
            let mut hung_sessions = 0u32;
            let session = loop {
                // Restore the exact pre-round-one scenario before taking the
                // new lease, so clock, RNG, guests, and weather do not depend
                // on contender order.
                control
                    .call("reset_park", json!({}))
                    .map_err(|e| format!("reset park before {model} round {round}: {e}"))?;
                // One lease per round: claiming it evicts any agent still alive
                // from an earlier round, which would otherwise build in this park.
                let lease = format!("{}-r{round}-{}", model, epoch_secs());
                client.claim("127.0.0.1", args.port, &lease);
                let session =
                    run_agent_session(&args, contender, &prompt, &round_dir, &lease, &shutdown);
                if !session.hung || shutdown.load(Ordering::Relaxed) {
                    break session;
                }
                hung_sessions += 1;
                if !retry_hung_round(hung_sessions, args.idle_retries) {
                    break session;
                }
                eprintln!(
                    "[{model}] round {round}: session went quiet for {}s; retrying ({} of {})",
                    args.idle_timeout, hung_sessions, args.idle_retries
                );
                hung_logs.extend(keep_hung_logs(&round_dir, hung_sessions));
            };
            if hung_sessions > 0 {
                // The round's own record of the watchdog firing. Nothing here
                // is named report.json or park.png, so a resume still sees an
                // unfinished round and builds it again.
                let record = json!({
                    "idle_timeout_seconds": args.idle_timeout,
                    "idle_retries": args.idle_retries,
                    "hung_sessions": hung_sessions,
                    "kept_logs": hung_logs,
                });
                if let Err(e) = write_json(&round_dir.join("session_retries.json"), &record) {
                    eprintln!("[{model}] round {round}: retry record write failed: {e}");
                }
            }
            if let Some(e) = &session.error {
                eprintln!("[{model}] round {round}: {e}");
            }
            if !session.usage.is_null() {
                let usage = json!({
                    "harness": contender.harness.name(),
                    "model": contender.model,
                    "input_tokens": session.usage.get("input_tokens"),
                    "output_tokens": session.usage.get("output_tokens"),
                    "cache_read_tokens": session.usage.get("cache_read_tokens"),
                    "cost_usd": session.usage.get("cost_usd"),
                    "num_turns": session.usage.get("num_turns"),
                });
                let _ = std::fs::create_dir_all(&round_dir);
                if let Err(e) = write_json(&round_dir.join("usage.json"), &usage) {
                    eprintln!("[{model}] round {round}: usage write failed: {e}");
                }
            }
            match collect_round(&mut client, &mut control, &args, &round_dir) {
                Ok(report) => {
                    let score = best_excitement(&report);
                    println!(
                        "[{model}] round {round}: excitement {}",
                        score.map_or("unrated".into(), |s| format!("{s:.2}"))
                    );
                    if let Some(s) = score {
                        if best.is_none_or(|(_, b)| s > b) {
                            best = Some((round, s));
                        }
                    }
                    feedback = serde_json::to_string(&report).ok();
                }
                Err(e) => {
                    eprintln!("[{model}] round {round}: collect failed: {e}");
                    feedback = Some(format!(
                        "{{\"error\": \"round produced no rated ride: {e}\"}}"
                    ));
                }
            }
        }
        standings.push(json!({
            "model": model,
            "harness": contender.harness.name(),
            "best_excitement": best.map(|(_, s)| s),
            "best_round": best.map(|(r, _)| r),
        }));
    }

    standings.sort_by(|a, b| {
        let score = |v: &Value| {
            v.get("best_excitement")
                .and_then(Value::as_f64)
                .unwrap_or(-1.0)
        };
        score(b)
            .partial_cmp(&score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    write_json(
        &run_dir.join("standings.json"),
        &json!({"standings": standings}),
    )?;

    println!("\n=== FINAL STANDINGS ===");
    for (place, entry) in standings.iter().enumerate() {
        let model = entry.get("model").and_then(Value::as_str).unwrap_or("?");
        match entry.get("best_excitement").and_then(Value::as_f64) {
            Some(s) => println!("{}. {model}: excitement {s:.2}", place + 1),
            None => println!("{}. {model}: no successful coaster", place + 1),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contenders_advertise_their_modalities_to_the_mcp_server() {
        let text_only = Contender {
            harness: Harness::Opencode,
            model: "openrouter/poolside/laguna-s-2.1".into(),
            modalities: Modalities::TEXT,
        };
        let multimodal = Contender {
            harness: Harness::ClaudeCode,
            model: "claude-sonnet-5".into(),
            modalities: Modalities::TEXT | Modalities::IMAGE,
        };
        assert!(text_only
            .mcp_url(8791, "lease-1", BenchmarkMode::Design)
            .ends_with("/mcp?modalities=text&condition=design&lease=lease-1"));
        assert!(multimodal
            .mcp_url(8791, "lease-1", BenchmarkMode::Library)
            .ends_with("/mcp?modalities=text,image&condition=library&lease=lease-1"));
    }

    #[test]
    fn each_harness_resolves_to_its_own_sandbox() {
        let args = Args::parse_from(["coaster-bench"]);
        let claude = Contender::parse("claude-sonnet-5");
        let opencode = Contender {
            harness: Harness::Opencode,
            model: "openrouter/moonshotai/kimi-k3".into(),
            modalities: Modalities::TEXT | Modalities::IMAGE,
        };
        assert_eq!(claude.sandbox(&args), "coaster-sub");
        assert_eq!(opencode.sandbox(&args), "coaster-or");
    }

    #[test]
    fn replay_length_follows_the_measured_lap() {
        let args = Args::parse_from(["coaster-bench"]);
        // The game measured the lap, so record that plus a tail rather than a
        // guess: a fixed 20s cut the test oval's 56s circuit off two thirds in.
        let report = json!({"rides": [{"ride_time": 56}]});
        assert_eq!(replay_seconds(&report, &args), 59);

        // Several rides in the park: the longest lap is the one that has to fit.
        let two = json!({"rides": [{"ride_time": 12}, {"ride_time": 40}]});
        assert_eq!(replay_seconds(&two, &args), 43);

        // A ride that never completed a test has no measured time; fall back to
        // the cap rather than recording nothing.
        let untested = json!({"rides": [{"ride_time": 0}]});
        assert_eq!(replay_seconds(&untested, &args), args.replay_seconds);
        assert_eq!(
            replay_seconds(&json!({"rides": []}), &args),
            args.replay_seconds
        );

        // A pathological lap is bounded by the cap.
        let epic = json!({"rides": [{"ride_time": 9999}]});
        assert_eq!(replay_seconds(&epic, &args), args.replay_seconds);
    }

    #[test]
    fn codex_specs_pick_their_backend_from_the_model_id() {
        let oauth = Contender::parse("codex:gpt-5.6-sol");
        assert_eq!(oauth.harness, Harness::Codex);
        assert_eq!(oauth.model, "gpt-5.6-sol");
        assert_eq!(
            oauth.codex_openrouter_model(),
            None,
            "the subscription backend uses its own login"
        );

        let via_openrouter = Contender::parse("codex:openrouter/openai/gpt-5.1");
        assert_eq!(via_openrouter.harness, Harness::Codex);
        assert_eq!(
            via_openrouter.codex_openrouter_model(),
            Some("openai/gpt-5.1"),
            "codex is pointed at OpenRouter with the bare model id"
        );

        assert_eq!(
            Contender::parse("claude-sonnet-5").codex_openrouter_model(),
            None,
            "other harnesses are never the codex backend"
        );
    }

    #[test]
    fn codex_contenders_run_in_the_codex_sandbox() {
        let args = Args::parse_from(["coaster-bench"]);
        assert_eq!(
            Contender::parse("codex:gpt-5.6-sol").sandbox(&args),
            "codex-arena"
        );
    }

    #[test]
    fn claude_code_runs_openrouter_models_in_their_own_lane() {
        let args = Args::parse_from(["coaster-bench"]);
        let sub = Contender::parse("claude-opus-5");
        let via_openrouter = Contender::parse("claude-code:openrouter/z-ai/glm-5.3-flash");

        assert_eq!(sub.harness, Harness::ClaudeCode);
        assert_eq!(via_openrouter.harness, Harness::ClaudeCode);
        assert_eq!(
            via_openrouter.claude_openrouter_model(),
            Some("z-ai/glm-5.3-flash"),
            "Claude Code takes the bare slug; the prefix is the base URL's job"
        );
        assert_eq!(sub.claude_openrouter_model(), None);
        assert_eq!(
            Contender::parse("opencode:openrouter/z-ai/glm-5.3-flash").claude_openrouter_model(),
            None,
            "other harnesses are never the Claude Code backend"
        );

        // Same harness, different backend, and therefore a different sandbox:
        // the credential and the policy are not the same.
        assert_eq!(sub.lane(), Lane::ClaudeCode);
        assert_eq!(via_openrouter.lane(), Lane::ClaudeCodeOpenRouter);
        assert_eq!(sub.sandbox(&args), "coaster-sub");
        assert_eq!(via_openrouter.sandbox(&args), "coaster-cc-or");
        assert_eq!(sub.provider(&args), "claude-sub");
        assert_eq!(via_openrouter.provider(&args), "openrouter");

        // Claude Code prices every model at Anthropic's rates, so its
        // transcript cost is fiction here and the spend delta is the record.
        assert!(via_openrouter.billed_by_openrouter());
        assert!(!sub.billed_by_openrouter());
    }

    #[test]
    fn archived_presentation_is_a_distinct_capability_condition() {
        let args = Args::try_parse_from([
            "coaster-bench",
            "--present-round",
            "evals/runs/example/model/round_6",
        ])
        .expect("presentation args");
        assert_eq!(
            args.present_round.as_deref(),
            Some(Path::new("evals/runs/example/model/round_6"))
        );
        assert_eq!(BenchmarkMode::Presentation.mcp_condition(), "presentation");
    }

    #[test]
    fn presentation_prompt_freezes_the_layout_and_requires_one_style() {
        let prompt = presentation_prompt(
            "gpt-5.6-sol",
            &json!({"rides": [{"excitement": 7.81, "num_inversions": 5}]}),
        );
        assert!(prompt.contains("layout and its score are frozen"));
        assert!(prompt.contains("style_best_ride exactly once"));
        assert!(prompt.contains("\"excitement\":7.81"));
        assert!(prompt.contains("\"num_inversions\":5"));
    }

    /// A blind codex round used to keep a shell the other lanes never had, so
    /// the design board compared models on different tool tables.
    #[test]
    fn codex_config_grants_a_shell_only_when_the_run_does() {
        let mut args = Args::parse_from(["coaster-bench"]);
        let codex = Contender::parse("codex:gpt-5.6-sol");
        let blind = codex_config(&args, &codex, "lease");
        assert!(blind.contains("shell_tool = false"));
        assert!(
            blind.contains("unified_exec = false"),
            "exec survives shell_tool on its own"
        );
        assert!(!has_shell(&args, &codex));

        args.open_note = true;
        let open = codex_config(&args, &codex, "lease");
        assert!(!open.contains("shell_tool = false"));
        assert!(has_shell(&args, &codex));

        args.open_note = false;
        args.extra_tools = Some("Bash".into());
        assert!(has_shell(&args, &codex), "asking for Bash still grants it");
        assert!(!codex_config(&args, &codex, "lease").contains("shell_tool = false"));
    }

    #[test]
    fn codex_config_keeps_top_level_keys_out_of_the_tables() {
        let mut args = Args::parse_from(["coaster-bench"]);
        args.open_note = true;
        let config = codex_config(
            &args,
            &Contender::parse("codex:openrouter/openai/gpt-5.1"),
            "l1",
        );

        let first_table = config.find('[').expect("config has tables");
        for key in [
            "model =",
            "model_provider =",
            "approval_policy =",
            "sandbox_mode =",
            "model_reasoning_effort =",
            "model_reasoning_summary =",
        ] {
            let at = config.find(key).unwrap_or_else(|| panic!("{key} missing"));
            assert!(at < first_table, "{key} fell inside a table:\n{config}");
        }
        assert!(config.contains("sandbox_mode = \"danger-full-access\""));
        assert!(config.contains("wire_api = \"responses\""), "chat is gone");
        assert!(config.contains("[mcp_servers.coaster]"));
        assert!(config.contains("lease=l1"), "per-round lease reaches codex");
        assert!(
            config.contains("default_tools_approval_mode = \"approve\""),
            "headless codex must not cancel coaster calls for lack of a user-input handler"
        );
    }

    #[test]
    fn codex_subscription_config_has_no_provider_block() {
        let args = Args::parse_from(["coaster-bench"]);
        let config = codex_config(&args, &Contender::parse("codex:gpt-5.6-sol"), "l1");
        assert!(config.starts_with("model = \"gpt-5.6-sol\""));
        assert!(
            config.contains("model_reasoning_effort = \"medium\""),
            "published runs pin the declared effort instead of inheriting low"
        );
        assert!(
            config.contains("model_reasoning_summary = \"auto\""),
            "codex's fallback for an unknown slug assumes no reasoning, \
             which traced zero reasoning items for the whole sol run"
        );
        assert!(
            !config.contains("model_provider"),
            "own login, not OpenRouter"
        );
        assert!(
            config.contains("sandbox_mode = \"read-only\""),
            "a blind round gets the tighter mode"
        );
    }

    #[test]
    fn codex_auth_timestamp_is_the_shape_codex_writes() {
        let stamped = rfc3339_now();
        assert_eq!(stamped.len(), 24, "{stamped}");
        assert!(stamped.ends_with(".000Z"), "{stamped}");
        assert_eq!(stamped.as_bytes()[10], b'T', "{stamped}");
    }

    #[test]
    fn modality_sets_render_in_catalogue_order() {
        assert_eq!(Modalities::TEXT.as_query_value(), "text");
        assert_eq!(
            (Modalities::FILE | Modalities::TEXT).as_query_value(),
            "text,file"
        );
        assert!(!Modalities::TEXT.contains(Modalities::IMAGE));
    }

    #[test]
    fn civil_date_matches_known_epochs() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_292), (2025, 7, 23));
    }

    #[test]
    fn penalty_math_matches_driver() {
        let report = json!({"rides": [{"excitement": 8.0}], "similarity": {"similarity": 0.75}});
        let score = best_excitement(&report).expect("scored");
        assert!((score - 4.0).abs() < 1e-9);
        let free = json!({"rides": [{"excitement": 8.0}], "similarity": {"similarity": 0.4}});
        assert!((best_excitement(&free).expect("scored") - 8.0).abs() < 1e-9);
        let authoritative = json!({
            "score": 3.5,
            "rides": [{"excitement": 8.0}],
            "similarity": {"similarity": 0.0}
        });
        assert_eq!(best_excitement(&authoritative), Some(3.5));
    }

    #[test]
    fn unrated_ride_scores_none() {
        let report = json!({"rides": [{"excitement": null}]});
        assert!(best_excitement(&report).is_none());
    }

    #[test]
    fn every_shutdown_signal_raises_the_flag() {
        let flag = install_shutdown_flag().expect("register");
        assert!(!flag.load(Ordering::Relaxed));
        for signal in [
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
        ] {
            flag.store(false, Ordering::Relaxed);
            signal_hook::low_level::raise(signal).expect("raise");
            // Delivery is asynchronous, but to this same process, so it lands
            // almost immediately; poll briefly rather than sleeping blind.
            let mut seen = false;
            for _ in 0..100 {
                if flag.load(Ordering::Relaxed) {
                    seen = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(seen, "signal {signal} did not set the shutdown flag");
        }
    }

    #[test]
    fn open_note_grants_the_file_tools_and_plain_runs_do_not() {
        let mut args = Args::parse_from(["coaster-bench"]);
        assert_eq!(allowed_tools(&args), "mcp__coaster__*");
        args.open_note = true;
        let allowed = allowed_tools(&args);
        for tool in ["Read", "Grep", "Glob", "Bash", "mcp__coaster__*"] {
            assert!(
                allowed.contains(tool),
                "{tool} must be allowed under open note"
            );
        }
    }

    #[test]
    fn policy_port_is_rewritten_to_the_running_port() {
        let policy = "network_policies:\n  coaster_game:\n    endpoints:\n      - host: host.containers.internal\n        port: 8791\n        protocol: rest\n";
        let out = policy_with_port(policy, 8899).expect("valid policy");
        assert!(out.contains("port: 8899"), "port rewritten in place: {out}");
        assert!(
            out.contains("host: host.containers.internal"),
            "rest untouched"
        );
        assert!(!out.contains("8791"));
    }

    #[test]
    fn policy_rewrite_does_not_touch_inference_ports() {
        let policy = "network_policies:\n  coaster_game:\n    endpoints:\n      - host: host.containers.internal\n        port: 8791\n  inference:\n    endpoints:\n      - host: api.openai.com\n        port: 443\n";
        let out = policy_with_port(policy, 1234).expect("valid policy");
        assert!(out.contains("port: 1234"), "{out}");
        assert!(out.contains("port: 443"), "{out}");
        assert!(out.contains("api.openai.com"), "{out}");
    }

    #[test]
    fn policy_rewrite_requires_the_named_game_endpoint() {
        let policy = "process:\n  run_as_user: sandbox\n";
        assert!(policy_with_port(policy, 1234).is_err());
    }

    #[test]
    fn ephemeral_names_fit_the_gateway_limit() {
        let name = ephemeral_sandbox_name("coaster-sub", 1_784_947_247);
        assert!(
            name.len() <= MAX_SANDBOX_NAME,
            "{name} is {} chars",
            name.len()
        );
        assert!(name.starts_with("coaster-sub-"));

        let long = ephemeral_sandbox_name("a-very-long-sandbox-name-indeed", 1_784_947_247);
        assert!(
            long.len() <= MAX_SANDBOX_NAME,
            "{long} is {} chars",
            long.len()
        );
        assert!(
            long.ends_with(name.rsplit('-').next().unwrap()),
            "id survives the trim"
        );
    }

    #[test]
    fn ephemeral_names_differ_between_runs() {
        let a = ephemeral_sandbox_name("coaster-sub", 1_784_947_247);
        let b = ephemeral_sandbox_name("coaster-sub", 1_784_947_248);
        assert_ne!(a, b);
    }

    #[test]
    fn extra_tools_are_added_without_open_note_and_never_duplicated() {
        let mut args = Args::parse_from(["coaster-bench"]);
        args.extra_tools = Some("Bash".into());
        assert_eq!(allowed_tools(&args), "mcp__coaster__*,Bash");

        args.open_note = true;
        let allowed = allowed_tools(&args);
        assert_eq!(
            allowed.matches("Bash").count(),
            1,
            "no duplicate: {allowed}"
        );
    }

    #[test]
    fn opencode_blind_runs_lose_the_file_tools_and_open_note_gets_them() {
        let mut args = Args::parse_from(["coaster-bench"]);
        let blind = opencode_permissions(&args);
        for tool in ["bash", "read", "grep", "glob", "edit", "external_directory"] {
            assert_eq!(blind[tool], "deny", "{tool} must be denied on a blind run");
        }

        args.open_note = true;
        let open = opencode_permissions(&args);
        for tool in ["bash", "read", "grep", "glob", "external_directory"] {
            assert_eq!(open[tool], "allow", "{tool} needed under open note");
        }
        assert_eq!(
            open["webfetch"], "deny",
            "network tools stay denied either way"
        );
    }

    /// Lays down a run directory the way a real run leaves one: a report per
    /// round, scored or not. Returns the run dir; the caller cleans it up.
    fn staged_run(tag: &str, model: &str, scores: &[Option<f64>]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coaster-resume-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (i, score) in scores.iter().enumerate() {
            let round = dir.join(model).join(format!("round_{}", i + 1));
            std::fs::create_dir_all(&round).expect("stage round");
            let report = match score {
                Some(s) => json!({"score": s, "rides": [{"excitement": s}]}),
                None => json!({"rides": []}),
            };
            write_json(&round.join("report.json"), &report).expect("stage report");
            // Written last by a real round, so it is what marks one finished.
            std::fs::write(round.join("park.png"), b"png").expect("stage capture");
        }
        dir
    }

    #[test]
    fn completed_rounds_counts_the_unbroken_prefix() {
        let model = "openrouter_moonshotai_kimi-k3";
        let dir = staged_run("prefix", model, &[Some(5.06), None, Some(6.18)]);
        assert_eq!(
            completed_rounds(&dir, model),
            3,
            "an unrated round still had its turn"
        );

        // Deleting a round is the documented way to ask for it again, so the
        // count must stop there rather than skipping over the hole.
        std::fs::remove_dir_all(dir.join(model).join("round_2")).expect("drop round 2");
        assert_eq!(completed_rounds(&dir, model), 1);
        assert_eq!(completed_rounds(&dir, "never-ran"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What actually happened to the kimi run: SIGTERM five minutes into round
    /// 6, leaving the report that collection writes before it captures. Counting
    /// that as a finished round would make resuming a no-op, which is precisely
    /// when someone is reaching for it.
    #[test]
    fn a_round_that_died_collecting_is_not_finished() {
        let model = "openrouter_moonshotai_kimi-k3";
        let dir = staged_run("halfway", model, &[Some(6.59), None]);
        std::fs::remove_file(dir.join(model).join("round_2").join("park.png"))
            .expect("drop the capture");
        assert_eq!(
            completed_rounds(&dir, model),
            1,
            "a report with no capture is a round that never finished"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A DeepSeek round sat untouched for twenty minutes with only the 1800s
    /// wall clock to end it. The bound must clear a model that is merely
    /// thinking: 949s is the longest quiet stretch in a healthy round.
    #[test]
    fn a_session_is_hung_only_once_it_passes_the_idle_bound() {
        let args = Args::parse_from(["coaster-bench"]);
        assert_eq!(args.idle_timeout, 1200);
        assert!(!session_went_quiet(
            Duration::from_secs(949),
            args.idle_timeout
        ));
        assert!(!session_went_quiet(
            Duration::from_secs(1199),
            args.idle_timeout
        ));
        assert!(session_went_quiet(
            Duration::from_secs(1200),
            args.idle_timeout
        ));
        // The off switch: a run that wants only the wall-clock bound gets it.
        assert!(!session_went_quiet(Duration::from_secs(99_999), 0));
    }

    #[test]
    fn hung_rounds_are_retried_a_bounded_number_of_times() {
        // One stall is bad luck and worth another go; two in a row is the
        // upstream, and the round is scored on whatever it built instead of
        // burning the wall clock again.
        assert!(retry_hung_round(1, 1));
        assert!(!retry_hung_round(2, 1));
        assert!(retry_hung_round(2, 2));
        assert!(!retry_hung_round(1, 0), "0 retries means score it as it is");
    }

    /// The retry must not leave anything behind that `completed_rounds` reads
    /// as a finished round, or a later --resume would skip the round the
    /// watchdog was trying to save. Only the logs are kept, under names of
    /// their own so the retry's fresh session.log does not overwrite the
    /// evidence of the stall.
    #[test]
    fn a_retried_round_still_looks_unfinished_to_resume() {
        let model = "openrouter_deepseek_deepseek-v4-flash";
        let dir = staged_run("hung", model, &[Some(4.21)]);
        let round = dir.join(model).join("round_2");
        std::fs::create_dir_all(&round).expect("stage round 2");
        std::fs::write(round.join("session.log"), b"{\"type\":\"step_start\"}")
            .expect("stage the stalled log");
        std::fs::write(round.join("session.err"), b"").expect("stage the stalled err");

        let kept = keep_hung_logs(&round, 1);
        assert_eq!(kept, vec!["session.hung_1.log", "session.hung_1.err"]);
        assert!(round.join("session.hung_1.log").is_file());
        assert!(
            !round.join("session.log").exists(),
            "the retry gets a fresh log"
        );
        assert_eq!(
            completed_rounds(&dir, model),
            1,
            "a round mid-retry is not a round that finished"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resuming_carries_the_best_round_and_the_last_report() {
        let model = "openrouter_moonshotai_kimi-k3";
        // Round 3 scores lower than round 1: a resumed run must not crown it.
        let dir = staged_run("carry", model, &[Some(6.18), None, Some(5.06)]);
        let (best, feedback) = resumed_state(&dir, model, 3);
        assert_eq!(best, Some((1, 6.18)), "the winner survives the gap");
        let feedback: Value =
            serde_json::from_str(&feedback.expect("last report is quoted back")).expect("json");
        assert_eq!(
            feedback["score"], 5.06,
            "feedback is the round before, not the best one"
        );

        let (best, feedback) = resumed_state(&dir, model, 0);
        assert_eq!(best, None, "nothing archived, nothing carried");
        assert!(feedback.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_may_grow_rounds_but_not_change_the_experiment() {
        let mut args = Args::parse_from(["coaster-bench", "--rounds", "8"]);
        let contenders = vec![Contender::parse("opencode:openrouter/moonshotai/kimi-k3")];
        let existing = json!({
            "condition": "design",
            "ride_type": 51,
            "models": ["openrouter_moonshotai_kimi-k3"],
            "rounds": 6,
        });
        check_resume_compatible(&existing, &args, &contenders).expect("more rounds is the point");

        args.ride_type = 52;
        let err = check_resume_compatible(&existing, &args, &contenders)
            .expect_err("two ride types in one standings table is not a run");
        assert!(err.contains("ride_type"), "{err}");

        args.ride_type = 51;
        args.mode = BenchmarkMode::Library;
        assert!(
            check_resume_compatible(&existing, &args, &contenders).is_err(),
            "blind rounds must not gain library rounds"
        );

        args.mode = BenchmarkMode::Design;
        let other = vec![Contender::parse("claude-opus-5")];
        assert!(
            check_resume_compatible(&existing, &args, &other).is_err(),
            "a different lineup is a different run"
        );
    }

    #[test]
    fn a_run_from_before_segments_becomes_its_own_first_segment() {
        let existing = json!({
            "rounds": 6,
            "harness_version": "0.1.0",
            "source": {"commit": "abc123", "dirty": false},
            "environment": {"arch": "aarch64"},
        });
        let segment = original_segment(&existing);
        assert_eq!(segment["harness_version"], "0.1.0");
        assert_eq!(segment["source"]["commit"], "abc123");
        assert_eq!(
            segment["started_at"],
            Value::Null,
            "we do not know when it ran, and must not invent it"
        );
    }

    /// Inkling Small ended all six rounds after ~2 minutes with nothing
    /// banked: opencode never re-prompts, and the warning was gated on codex
    /// alone. kimi and DeepSeek hid it by never choosing to stop.
    #[test]
    fn every_harness_that_never_reprompts_is_told_stopping_is_final() {
        assert!(Harness::Opencode.single_turn());
        assert!(Harness::Codex.single_turn());
        assert!(
            !Harness::ClaudeCode.single_turn(),
            "claude-code keeps going until the turn budget, so the warning would be a lie"
        );
    }

    /// Blind runs are MCP-only on every lane, and the record says so. Before
    /// this, codex kept a shell the other lanes never had and reported no
    /// capabilities at all.
    #[test]
    fn every_lane_is_mcp_only_when_the_run_is_blind() {
        let mut args = Args::parse_from(["coaster-bench"]);
        let codex = Contender::parse("codex:gpt-5.6-sol");
        let opencode = Contender::parse("opencode:openrouter/moonshotai/kimi-k3");
        let claude = Contender::parse("claude-opus-5");

        for contender in [&codex, &opencode, &claude] {
            assert!(!has_shell(&args, contender));
        }

        args.open_note = true;
        for contender in [&codex, &opencode, &claude] {
            assert!(
                has_shell(&args, contender),
                "open note grants it to every lane"
            );
        }
    }

    #[test]
    fn openrouter_upstream_is_pinned_per_model_and_only_when_asked() {
        let mut args = Args::parse_from(["coaster-bench"]);
        let contender = Contender::parse("opencode:openrouter/moonshotai/kimi-k3");
        let unpinned = opencode_provider_routing(&args, &contender)
            .expect("openrouter contenders always declare their model");
        let entry = &unpinned["openrouter"]["models"]["moonshotai/kimi-k3"];
        assert_eq!(
            entry["name"], "moonshotai/kimi-k3",
            "the model is declared so a stale models.dev snapshot cannot kill the run"
        );
        assert!(
            entry.get("options").is_none(),
            "unpinned runs must leave OpenRouter's routing alone"
        );

        args.openrouter_provider = Some("baseten".into());
        let routing = opencode_provider_routing(&args, &contender)
            .expect("a pinned run routes its opencode contender");
        let provider =
            &routing["openrouter"]["models"]["moonshotai/kimi-k3"]["options"]["provider"];
        assert_eq!(provider["order"], json!(["baseten"]));
        assert_eq!(
            provider["allow_fallbacks"], true,
            "a dead upstream costs a retry, not the round"
        );

        // Claude Code and codex contenders never touch OpenRouter's router.
        assert!(
            opencode_provider_routing(&args, &Contender::parse("claude-opus-5")).is_none(),
            "only openrouter-served models are keyed by an openrouter slug"
        );
    }

    /// A headless "ask" is an auto-reject, and opencode kills the session on a
    /// rejected call. Left at its default this cost kimi-k3 a whole round for
    /// undoing pieces in a row, which is ordinary coaster building.
    #[test]
    fn the_doom_loop_guard_never_ends_a_headless_round() {
        let mut args = Args::parse_from(["coaster-bench"]);
        assert_eq!(opencode_permissions(&args)["doom_loop"], "allow");
        args.open_note = true;
        assert_eq!(opencode_permissions(&args)["doom_loop"], "allow");
    }
}
