//! Minimal MCP (Model Context Protocol) server over streamable HTTP.
//!
//! Hand-rolled on purpose: the protocol subset (initialize, tools/list,
//! tools/call) is small, and no async runtime is needed for it.
//!
//! The game API is strictly single-threaded, but I/O is not: sockets are read
//! and written on a thread per connection, and every request crosses one
//! channel to the game thread, which owns the session and is the only thread
//! allowed to touch the game. Tool calls still execute one at a time in arrival
//! order, so the simulation stays deterministic. Nothing reachable from a
//! connection thread may call `host::*`, the logger included.
//!
//! Two listeners, and which one a request arrived on decides what it can ask
//! for: the MCP listener is reachable from the agent sandboxes, the control
//! listener binds loopback for the harness. They dispatch against separate tool
//! tables, so a control tool is absent from the agent's world rather than
//! merely refused to it.
//!
//! Transport: POST /mcp with a JSON-RPC message, single JSON response
//! (the "streamable HTTP" JSON response mode). GET returns 405.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};

use crate::host;
use crate::library::{self, LibraryDesign};
use crate::pieces;
use crate::program;
use crate::report;

/// Content kinds a client can accept, named after the OpenRouter model
/// catalogue's `input_modalities` so a harness can pass a model's declared
/// capabilities straight through. A client advertises its set in the request
/// target (`/mcp?modalities=text`), and tools that would answer with content
/// outside that set are hidden from tools/list and refused on call: a
/// text-only model handed an image content block fails the whole request
/// upstream, so it must never see the screenshot tool.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Modalities(u8);

impl Modalities {
    const TEXT: Modalities = Modalities(1 << 0);
    const IMAGE: Modalities = Modalities(1 << 1);
    const AUDIO: Modalities = Modalities(1 << 2);
    const FILE: Modalities = Modalities(1 << 3);
    /// What a client gets when it advertises nothing: everything the server has.
    const ALL: Modalities = Modalities(0b1111);

    fn contains(self, other: Modalities) -> bool {
        self.0 & other.0 == other.0
    }

    /// Parse a comma-separated list; unknown names are ignored.
    fn parse(list: &str) -> Modalities {
        list.split(',').fold(Modalities(0), |set, name| {
            set | match name.trim() {
                "text" => Modalities::TEXT,
                "image" => Modalities::IMAGE,
                "audio" => Modalities::AUDIO,
                "file" => Modalities::FILE,
                _ => Modalities(0),
            }
        })
    }

    /// Read the set out of an HTTP request target, e.g. "/mcp?modalities=text".
    fn from_request_target(target: &str) -> Modalities {
        let Some((_, query)) = target.split_once('?') else {
            return Modalities::ALL;
        };
        query
            .split('&')
            .find_map(|param| param.strip_prefix("modalities="))
            .map(Modalities::parse)
            .unwrap_or(Modalities::ALL)
    }
}

impl std::ops::BitOr for Modalities {
    type Output = Modalities;
    fn bitor(self, rhs: Modalities) -> Modalities {
        Modalities(self.0 | rhs.0)
    }
}

/// A request's claim on the park, read from the query string.
struct Claim {
    lease: Option<String>,
    /// `claim=1` takes ownership, replacing whatever lease was held.
    takes_ownership: bool,
}

impl Claim {
    fn from_request_target(target: &str) -> Claim {
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let param = |key: &str| {
            query
                .split('&')
                .find_map(|p| p.strip_prefix(key))
                .map(str::to_string)
        };
        Claim {
            lease: param("lease="),
            takes_ownership: param("claim=").as_deref() == Some("1"),
        }
    }

    /// Applies the claim to the session and reports whether the request may
    /// proceed. Taking ownership always wins; otherwise the lease must match.
    fn authorise(&self, session: &mut Session) -> bool {
        if self.takes_ownership {
            // A new round takes the park, so the previous round's best test
            // must not carry over into it.
            session.lease = self.lease.clone();
            session.best_test = None;
            session.best_shot = None;
            return true;
        }
        match (&session.lease, &self.lease) {
            (None, _) => true,
            (Some(held), Some(offered)) => held == offered,
            (Some(_), None) => false,
        }
    }
}

/// Render the whole park to a PNG and return the bytes. The full map is
/// intentional: an agent needs whole-park spatial context, not a viewport.
fn capture_park_png() -> Result<Vec<u8>, String> {
    let path = std::env::temp_dir().join("orct2_mcp_capture.png");
    let path_str = path.to_string_lossy();
    if !host::capture(&path_str, 0, 0, false, false) {
        return Err("capture failed".into());
    }
    std::fs::read(&path).map_err(|e| format!("read capture: {e}"))
}

fn image_content(bytes: Vec<u8>) -> Vec<Value> {
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    vec![json!({"type": "image", "data": data, "mimeType": "image/png"})]
}

/// The content kind a tool answers with; everything not listed here is text.
fn tool_modality(name: &str) -> Modalities {
    match name {
        "screenshot" | "best_screenshot" => Modalities::IMAGE,
        _ => Modalities::TEXT,
    }
}

/// One placed piece: what and where, so it can be undone.
struct Placed {
    name: &'static str,
    track_type: u16,
    origin: host::TrackCursor,
    cost: i64,
    /// Whether this piece carries the chain lift. Stored so placed_pieces (and
    /// the program.json rebuilt from it) can round-trip the lift hills — a
    /// name-only piece list rebuilds the shape but not a working ride.
    chain: bool,
    /// Brake/booster speed, for the pieces that have one. Same reason as
    /// `chain`: a replay without it is a different ride.
    speed: Option<u8>,
}

/// The piece list in program form: a bare name when the piece is plain, or a
/// {"t": name, "chain": true} object when it carries the lift (the format
/// program.rs already parses). Feeds get_state and the best-test snapshot, so a
/// program rebuilt from it replays to the same ride, not just the same shape.
fn placed_pieces_json(placed: &[Placed]) -> Vec<Value> {
    placed
        .iter()
        .map(|p| match (p.chain, p.speed) {
            (false, None) => json!(p.name),
            (true, None) => json!({"t": p.name, "chain": true}),
            (false, Some(speed)) => json!({"t": p.name, "speed": speed}),
            (true, Some(speed)) => json!({"t": p.name, "chain": true, "speed": speed}),
        })
        .collect()
}

/// Build state for the ride currently under construction.
struct Build {
    ride_id: u16,
    start: host::TrackCursor,
    cursor: host::TrackCursor,
    placed: Vec<Placed>,
    finalized: bool,
}

impl Build {
    /// Places one catalog piece at the cursor and records it, or hands back the
    /// game's rejection reason. The cursor only moves when the game accepts.
    ///
    /// `speed` is None unless the caller asked for one. A piece that reads a
    /// speed gets the game's own default rather than zero, which would leave a
    /// booster inert; a piece that ignores speed is refused an explicit one, so
    /// a setting that would do nothing fails loudly instead of silently.
    fn place(&mut self, piece: &str, chain: bool, speed: Option<u8>) -> Result<i64, String> {
        let (name, track_type) = pieces::CATALOG
            .iter()
            .find(|(n, _)| *n == piece)
            .copied()
            .ok_or_else(|| format!("unknown piece: {piece}"))?;
        let speed = pieces::resolve_speed(track_type, speed)?;
        let origin = self.cursor;
        let cost = host::track_place(self.ride_id, track_type, chain, speed, &mut self.cursor)?;
        self.placed.push(Placed {
            name,
            track_type,
            origin,
            cost,
            chain,
            speed: pieces::takes_speed(track_type).then_some(speed),
        });
        Ok(cost)
    }

    fn station_tiles(&self) -> Vec<(i32, i32)> {
        self.placed
            .iter()
            .filter(|p| (1..=3).contains(&p.track_type))
            .map(|p| (p.origin.x / 32, p.origin.y / 32))
            .collect()
    }

    fn total_cost(&self) -> i64 {
        self.placed.iter().map(|p| p.cost).sum()
    }
}

#[derive(Default)]
struct Session {
    build: Option<Build>,
    /// Stock design library, loaded from the host on first use.
    library: Option<Vec<LibraryDesign>>,
    /// Who currently owns the park, if anyone.
    ///
    /// There is one park and one build state, so two agents connected at once
    /// fight: each demolishes what the other builds, and neither can finish. A
    /// harness claims the park for a round (`/mcp?lease=<id>&claim=1`) and
    /// passes the same lease to its agent; anyone holding a stale lease is
    /// refused instead of silently sharing the park. Unleased servers stay
    /// open, so an interactive session needs no ceremony.
    lease: Option<String>,
    /// The highest-excitement finish_and_test report seen this lease. A model
    /// that tests a good coaster then demolishes it to chase a better one used
    /// to be scored on whatever it was mid-building at the timeout, which
    /// punished iterating; the round is scored on this instead. Reset when a
    /// new round claims the park.
    best_test: Option<Value>,
    /// Park PNG captured at the moment `best_test` was set, so the scored
    /// coaster can be pictured even after it was demolished. Reset with it.
    best_shot: Option<Vec<u8>>,
}

/// Best-ride excitement in a finish_and_test report, if any ride was rated.
fn report_excitement(report: &Value) -> Option<f64> {
    report
        .get("rides")?
        .as_array()?
        .iter()
        .filter_map(|r| r.get("excitement").and_then(Value::as_f64))
        .fold(None, |best: Option<f64>, e| {
            Some(best.map_or(e, |b| b.max(e)))
        })
}

const NO_RIDE: &str = "no active ride; call new_ride first";
const FINALIZED: &str = "ride already finalized; use demolish + new_ride to rebuild";

impl Session {
    fn active(&self) -> Result<&Build, String> {
        self.build.as_ref().ok_or_else(|| NO_RIDE.to_string())
    }

    /// The build, when it is still accepting pieces.
    fn open(&mut self) -> Result<&mut Build, String> {
        let build = self.build.as_mut().ok_or_else(|| NO_RIDE.to_string())?;
        if build.finalized {
            return Err(FINALIZED.to_string());
        }
        Ok(build)
    }

    fn library(&mut self) -> Result<&[LibraryDesign], String> {
        if self.library.is_none() {
            self.library = Some(library::load()?);
        }
        // The Option was just filled; unwrap_or_default keeps this panic-free.
        Ok(self.library.as_deref().unwrap_or_default())
    }
}

/// Which socket a request arrived on. This is the capability boundary: the MCP
/// listener is reachable from the sandboxes (it has to be), the control
/// listener binds loopback only. Tools are looked up in a table chosen by this,
/// so a control tool is not merely hidden from an agent, it does not exist in
/// the table its requests are dispatched against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Mcp,
    Control,
}

/// One request waiting on the game thread, with the channel its connection is
/// blocked on.
struct Request {
    source: Source,
    /// The HTTP request target, carrying the modality set and the park lease.
    target: String,
    message: Value,
    reply: mpsc::Sender<Value>,
}

/// How long a connection waits for the game thread before giving up. The game
/// thread is single-threaded and a tool call can run thousands of ticks, so
/// this is generous; it exists so a wedged game cannot leak connection threads
/// forever.
const REPLY_TIMEOUT: Duration = Duration::from_secs(600);

/// Runs the server until the process is killed. Never panics; per-request
/// failures are reported to the client and the listeners keep accepting.
///
/// Sockets are read and written on per-connection threads, and every request is
/// funnelled through one channel to this thread, which owns the session and is
/// the only thread that may touch the game: the game API is single-threaded,
/// but nothing about *I/O* has to happen there. Previously it did, so one
/// client that connected and vanished blocked the accept loop and hung every
/// other client. Requests still execute one at a time in arrival order, so the
/// simulation stays as deterministic as it was.
pub fn serve(bind: &str, port: u16, control_port: u16) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Request>();

    let listener =
        TcpListener::bind((bind, port)).map_err(|e| format!("bind {bind}:{port}: {e}"))?;
    host::log(&format!(
        "orct2-agent: MCP server listening on http://{bind}:{port}/mcp"
    ));
    spawn_listener(listener, Source::Mcp, tx.clone());

    if control_port > 0 {
        // Loopback only, and not configurable: the harness runs on this
        // machine, and a control port reachable from a sandbox would hand the
        // thing under test a write to the host filesystem.
        let control = TcpListener::bind(("127.0.0.1", control_port))
            .map_err(|e| format!("bind 127.0.0.1:{control_port}: {e}"))?;
        host::log(&format!(
            "orct2-agent: control server listening on http://127.0.0.1:{control_port}/mcp"
        ));
        spawn_listener(control, Source::Control, tx.clone());
    }
    // The listeners hold the senders they need; without this the loop below
    // could never see a disconnect.
    drop(tx);

    let mut session = Session::default();
    while let Ok(request) = rx.recv() {
        let response = handle_request(&request, &mut session);
        // A connection that gave up while we worked is not an error worth
        // reporting: its thread is already gone.
        let _ = request.reply.send(response);
    }
    Ok(())
}

fn spawn_listener(listener: TcpListener, source: Source, tx: mpsc::Sender<Request>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let tx = tx.clone();
            // Per connection, so a client that stalls mid-request stalls only
            // itself. These threads never call host::*, including the logger:
            // the game side of the bridge is not thread-safe.
            std::thread::spawn(move || {
                let _ = pump_connection(stream, source, &tx);
            });
        }
    });
}

/// Reads requests off one socket, hands each to the game thread, and writes
/// back what it answers. Bytes and JSON only: no game state is touched here.
fn pump_connection(
    stream: TcpStream,
    source: Source,
    tx: &mpsc::Sender<Request>,
) -> Result<(), String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut stream = stream;
    loop {
        let (method, target, body) = match read_http_request(&mut reader) {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(()), // clean EOF
            Err(e) => return Err(e),
        };
        if method != "POST" {
            write_http(&mut stream, 405, "text/plain", b"method not allowed")?;
            continue;
        }
        let message: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                let resp = rpc_error(Value::Null, -32700, &format!("parse error: {e}"));
                write_json(&mut stream, &resp)?;
                continue;
            }
        };
        // Notifications (no id) get 202 Accepted with no body.
        if message.get("id").is_none() {
            write_http(&mut stream, 202, "text/plain", b"")?;
            continue;
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        tx.send(Request {
            source,
            target,
            message,
            reply: reply_tx,
        })
        .map_err(|_| "game thread is gone".to_string())?;
        let response = reply_rx
            .recv_timeout(REPLY_TIMEOUT)
            .map_err(|e| format!("waiting on the game thread: {e}"))?;
        write_json(&mut stream, &response)?;
    }
}

/// Ticks until the ride's lead train is pulling out of the station, so a replay
/// opens on a departure instead of mid-circuit. Gives up after a lap's worth of
/// ticks: a train that never departs (a valleyed circuit) still gets filmed,
/// which is exactly when the footage is worth watching.
fn wait_for_departure(ride_id: u16, every: u32) -> u32 {
    const DEPARTING: u8 = 3;
    const MAX_WAIT_TICKS: u32 = 40 * 120;
    let mut waited = 0;
    while waited < MAX_WAIT_TICKS {
        if host::vehicle_status(ride_id) == Some(DEPARTING) {
            break;
        }
        host::run_ticks(every.max(1));
        waited += every.max(1);
    }
    waited
}

/// Game-thread side of one request: claim the park, then dispatch. Everything
/// reachable from here may call host functions.
fn handle_request(request: &Request, session: &mut Session) -> Value {
    if !Claim::from_request_target(&request.target).authorise(session) {
        // A stale agent from a finished round: refuse rather than let it
        // build in a park that now belongs to someone else.
        return rpc_error(
            request.message.get("id").cloned().unwrap_or(Value::Null),
            -32000,
            "this park is leased to another session; your lease is stale",
        );
    }
    let modalities = Modalities::from_request_target(&request.target);
    dispatch(&request.message, session, modalities, request.source)
}

fn read_http_request(
    reader: &mut BufReader<TcpStream>,
) -> Result<Option<(String, String, Vec<u8>)>, String> {
    let mut request_line = String::new();
    let n = reader
        .read_line(&mut request_line)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(Some((method, target, body)))
}

fn write_http(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nMcp-Session-Id: orct2\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(body).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}

fn write_json(stream: &mut TcpStream, value: &Value) -> Result<(), String> {
    let body = value.to_string();
    write_http(stream, 200, "application/json", body.as_bytes())
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn dispatch(
    message: &Value,
    session: &mut Session,
    modalities: Modalities,
    source: Source,
) -> Value {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => {
            let requested = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-06-18");
            rpc_result(
                id,
                json!({
                    "protocolVersion": requested,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "openrct2-coaster", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => {
            let tools = match source {
                Source::Mcp => tool_definitions(modalities),
                Source::Control => control_tool_definitions(),
            };
            rpc_result(id, json!({"tools": tools}))
        }
        "tools/call" => {
            let name = message
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = message
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            if source == Source::Control {
                return match call_control_tool(name, &args, session) {
                    Ok(content) => rpc_result(id, json!({"content": content, "isError": false})),
                    Err(text) => rpc_result(
                        id,
                        json!({"content": [{"type": "text", "text": text}], "isError": true}),
                    ),
                };
            }
            if !modalities.contains(tool_modality(name)) {
                return rpc_result(
                    id,
                    json!({
                        "content": [{"type": "text", "text": format!("{name} is unavailable: this client did not advertise the content kind it answers with")}],
                        "isError": true,
                    }),
                );
            }
            match call_tool(name, &args, session) {
                Ok(content) => rpc_result(id, json!({"content": content, "isError": false})),
                Err(text) => rpc_result(
                    id,
                    json!({"content": [{"type": "text", "text": text}], "isError": true}),
                ),
            }
        }
        _ => rpc_error(id, -32601, &format!("method not found: {method}")),
    }
}

fn tool_definitions(modalities: Modalities) -> Value {
    let mut tools = all_tool_definitions();
    if let Some(list) = tools.as_array_mut() {
        list.retain(|t| {
            let name = t.get("name").and_then(Value::as_str).unwrap_or_default();
            modalities.contains(tool_modality(name))
        });
    }
    tools
}

/// The harness's own tools, served only on the loopback control listener.
/// save_park writes a path on the machine running the game, which is precisely
/// what a sandboxed agent must not be able to ask for.
fn control_tool_definitions() -> Value {
    json!([
        {
            "name": "save_park",
            "description": "Write the park to a .park save on the machine running the game.",
            "inputSchema": {"type": "object", "required": ["path"], "properties": {
                "path": {"type": "string", "description": "Filesystem path to write"}
            }}
        },
        {
            "name": "capture_replay",
            "description": "Film the running ride straight into an mp4: ticks the simulation, renders frames, and pipes them to ffmpeg. Starts from the train leaving the station.",
            "inputSchema": {"type": "object", "required": ["out"], "properties": {
                "out": {"type": "string", "description": "Path of the .mp4 to write"},
                "frames": {"type": "integer", "description": "How many frames to capture (default 400)"},
                "every_ticks": {"type": "integer", "description": "Game ticks between frames; 40 ticks = 1 second (default 2, so 20fps)"},
                "zoom": {"type": "integer", "description": "Zoom level; higher is smaller and cheaper (default 0)"}
            }}
        }
    ])
}

fn call_control_tool(
    name: &str,
    args: &Value,
    session: &mut Session,
) -> Result<Vec<Value>, String> {
    match name {
        "save_park" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or("path is required")?;
            if host::save_park(path) {
                Ok(text_content(json!({"saved": path})))
            } else {
                Err("park save failed".to_string())
            }
        }
        "capture_replay" => {
            let out = args
                .get("out")
                .and_then(Value::as_str)
                .ok_or("out (an .mp4 path) is required")?;
            let frames = arg_i64(args, "frames").unwrap_or(400).clamp(1, 20_000) as usize;
            let every = arg_i64(args, "every_ticks").unwrap_or(2).clamp(1, 400) as u32;
            let zoom = arg_i64(args, "zoom").unwrap_or(0).clamp(0, 3) as i32;
            let fps = 40.0 / f64::from(every);

            // Framing is fit to the track's bounding box, which does not move
            // while a test runs, so every frame shares a camera. That is the
            // point: a still camera over a still park means consecutive frames
            // differ only where the train is, which is what makes the encode
            // small.
            let (width, height) = host::capture_size(zoom, 0, true).ok_or("no track to film")?;

            // Start where a viewer expects: the train pulling out of the
            // station, not wherever it happened to be when the test ended.
            let waited = session
                .build
                .as_ref()
                .map(|b| b.ride_id)
                .map(|ride| wait_for_departure(ride, every))
                .unwrap_or(0);

            let mut child = std::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error"])
                .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
                .args(["-s", &format!("{width}x{height}")])
                .args(["-framerate", &format!("{fps}")])
                .args(["-i", "-"])
                .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "28"])
                // One keyframe for the whole clip: the scene is static, so
                // extra keyframes are pure cost.
                .args(["-g", &frames.to_string(), "-movflags", "+faststart"])
                .arg(out)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("ffmpeg (brew install ffmpeg): {e}"))?;
            let mut sink = child.stdin.take().ok_or("ffmpeg stdin")?;

            // One buffer for the whole replay, handed to the renderer and then
            // straight to the encoder: no PNG, no temporary files.
            let mut buf = vec![0u8; width as usize * height as usize * 4];
            let mut written = 0usize;
            for _ in 0..frames {
                host::run_ticks(every);
                let n = host::capture_frame(zoom, 0, true, &mut buf);
                if n == 0 || sink.write_all(&buf[..n]).is_err() {
                    break;
                }
                written += 1;
            }
            drop(sink);
            let status = child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
            if !status.success() {
                return Err(format!("ffmpeg failed: {status}"));
            }
            Ok(text_content(json!({
                "out": out,
                "frames": written,
                "width": width,
                "height": height,
                "fps": fps,
                "ticks_waited_for_station": waited,
            })))
        }
        other => Err(format!("unknown control tool: {other}")),
    }
}

fn all_tool_definitions() -> Value {
    json!([
        {
            "name": "new_ride",
            "description": "Create a new coaster and set the build cursor at a start tile. Demolishes any ride previously built in this session. Station pieces must come first.",
            "inputSchema": {"type": "object", "required": ["ride_type", "x", "y", "dir"], "properties": {
                "ride_type": {"type": "integer", "description": "Game ride type, e.g. 52 = wooden roller coaster"},
                "x": {"type": "integer", "description": "Start tile x"},
                "y": {"type": "integer", "description": "Start tile y"},
                "dir": {"type": "integer", "minimum": 0, "maximum": 3, "description": "Facing: 0=-x, 1=+y, 2=+x, 3=-y"}
            }}
        },
        {
            "name": "place_piece",
            "description": "Place one track piece at the cursor and advance it. Returns the new cursor (including bank/slope state) and piece cost, or the game's rejection reason.",
            "inputSchema": {"type": "object", "required": ["piece"], "properties": {
                "piece": {"type": "string", "description": "Piece name from the catalog, e.g. begin_station, flat, up_25, left_turn_5"},
                "chain": {"type": "boolean", "description": "Chain lift on this piece (for climbing)"},
                "speed": {"type": "integer", "description": "brakes and booster only: brakes slow a train down to this speed, a booster accelerates it up to this speed. 0-30, default 8. Rejected on any other piece."}
            }}
        },
        {
            "name": "valid_next_pieces",
            "description": "Ask the game which catalog pieces would be accepted at the current cursor. Cheap and side-effect free; use it whenever unsure.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "get_state",
            "description": "Current cursor position/direction/bank/slope, start position, pieces placed, and cost so far.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "finish_and_test",
            "description": "Close out the build: auto-place entrance/exit, set the ride to testing, simulate, and return ratings + measurements. The track must form a closed circuit back to the start.",
            "inputSchema": {"type": "object", "properties": {
                "ticks": {"type": "integer", "description": "Simulation ticks (40/game-second); default 25000"}
            }}
        },
        {
            "name": "best_result",
            "description": "Return the highest-excitement finish_and_test result from this round, even if the park has since been demolished or rebuilt. Your round is scored on this, so demolishing a tested coaster to try something better never loses the earlier score.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "best_screenshot",
            "description": "Rendered image of the park as it stood for your best_result, even if you have since rebuilt.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "screenshot",
            "description": "Rendered image of the park including the track built so far.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "undo_piece",
            "description": "Remove the most recently placed piece and move the cursor back to where it was. Use this to fix mistakes without demolishing the whole ride.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "piece_geometry",
            "description": "Exact cursor displacement of every catalog piece for a given facing: dx/dy in tiles, dz in height units, and the resulting direction/bank/slope. Use this to plan a closed circuit on paper before placing. Displacements are facing-specific; query again after turns.",
            "inputSchema": {"type": "object", "properties": {
                "dir": {"type": "integer", "minimum": 0, "maximum": 3, "description": "Facing to compute deltas for (default 0)"}
            }}
        },
        {
            "name": "demolish",
            "description": "Remove the current ride and its track so you can start over with new_ride.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "search_track_designs",
            "description": "List the stock track design library (the coasters shipped with the game), optionally filtered by ride type. Returns name, ride type, and piece count per design. Study them for ideas, but note: the final score is penalized for similarity to any library design (mirrored copies included), so submitting a copy scores near zero.",
            "inputSchema": {"type": "object", "properties": {
                "ride_type": {"type": "integer", "description": "Only designs for this game ride type, e.g. 52 = wooden roller coaster"}
            }}
        },
        {
            "name": "get_track_design",
            "description": "Full piece sequence of one stock library design, in the same format place_pieces accepts (catalog names where known, raw numeric track types otherwise). Remember the similarity penalty: adapt, don't copy.",
            "inputSchema": {"type": "object", "required": ["name"], "properties": {
                "name": {"type": "string", "description": "Design name as returned by search_track_designs"}
            }}
        },
        {
            "name": "place_pieces",
            "description": "Batch placement: place a sequence of track pieces in one call. Stops at the FIRST rejection; all pieces placed BEFORE a rejection stay. Returns: number placed this call, total pieces placed, cursor state (with bank/slope), circuit_closed flag, and rejection details if any (index, piece, game error). Maximum 200 pieces per call. Still verify circuit closure arithmetic first; this just reduces round trips.",
            "inputSchema": {"type": "object", "required": ["pieces"], "properties": {
                "pieces": {
                    "type": "array",
                    "description": "Array of pieces to place sequentially. Each element is either a string (piece name) or an object {\"t\": \"name\", \"chain\": true/false, \"speed\": 0-30}; \"piece\" is accepted in place of \"t\". This is the same form get_state and the eval report hand back, so a piece list from either can be replayed as-is. speed applies to brakes and booster only (brakes slow to it, a booster accelerates up to it; default 8).",
                    "maxItems": 200
                }
            }}
        }
    ])
}

fn text_content(value: Value) -> Vec<Value> {
    vec![json!({"type": "text", "text": value.to_string()})]
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

fn call_tool(name: &str, args: &Value, session: &mut Session) -> Result<Vec<Value>, String> {
    match name {
        "new_ride" => {
            if let Some(build) = session.build.take() {
                let _ = host::ride_demolish(build.ride_id);
            }
            let ride_type = arg_i64(args, "ride_type").ok_or("ride_type required")? as u16;
            let x = arg_i64(args, "x").ok_or("x required")? as i32;
            let y = arg_i64(args, "y").ok_or("y required")? as i32;
            let dir = (arg_i64(args, "dir").ok_or("dir required")? as u8) & 3;
            if !host::enable_sandbox() {
                host::log("orct2-agent: warning: sandbox cheat failed");
            }
            let ride_id = host::ride_create(ride_type).map_err(|e| format!("ride_create: {e}"))?;
            let z = (host::surface_z(x, y) + 15) & !15;
            let cursor = host::TrackCursor {
                x: x * 32,
                y: y * 32,
                z,
                direction: dir,
                bank: 0,  // TrackRoll::none
                slope: 0, // TrackPitch::none
            };
            session.build = Some(Build {
                ride_id,
                start: cursor,
                cursor,
                placed: Vec::new(),
                finalized: false,
            });
            Ok(text_content(
                json!({"ride_id": ride_id, "cursor": cursor_json(&cursor)}),
            ))
        }
        "place_piece" => {
            let build = session.open()?;
            let piece = args
                .get("piece")
                .and_then(Value::as_str)
                .ok_or("piece required")?;
            let chain = args.get("chain").and_then(Value::as_bool).unwrap_or(false);
            let speed = arg_i64(args, "speed").map(|s| s.clamp(0, 255) as u8);
            let cost = build.place(piece, chain, speed).map_err(|e| {
                format!(
                    "'{piece}' rejected: {e} (cursor unchanged: {})",
                    cursor_json(&build.cursor)
                )
            })?;
            Ok(text_content(json!({
                "placed": piece,
                "cost": cost,
                "cursor": cursor_json(&build.cursor),
                "pieces_placed": build.placed.len(),
                "circuit_closed": build.cursor == build.start,
            })))
        }
        "valid_next_pieces" => {
            let build = session.active()?;
            let valid: Vec<&str> = pieces::CATALOG
                .iter()
                .filter(|(_, id)| host::track_query(build.ride_id, *id, &build.cursor).is_ok())
                .map(|&(name, _)| name)
                .collect();
            Ok(text_content(
                json!({"valid_pieces": valid, "cursor": cursor_json(&build.cursor)}),
            ))
        }
        "get_state" => {
            let build = session.active()?;
            Ok(text_content(json!({
                "ride_id": build.ride_id,
                "start": cursor_json(&build.start),
                "cursor": cursor_json(&build.cursor),
                "pieces_placed": build.placed.len(),
                "placed_pieces": placed_pieces_json(&build.placed),
                "total_cost": build.total_cost(),
                "circuit_closed": build.cursor == build.start,
                "finalized": build.finalized,
            })))
        }
        "finish_and_test" => {
            let ticks = arg_i64(args, "ticks").unwrap_or(25000).clamp(1, 200_000) as u32;
            // Disjoint field borrows: the cached library (shared with the
            // search tools) is read while the build is mutated, so every
            // finish_and_test doesn't re-import all 200+ .TD6 files.
            let Session { build, library, .. } = session;
            if library.is_none() {
                *library = library::load().ok();
            }
            let library = library.as_deref();
            let build = build.as_mut().ok_or_else(|| NO_RIDE.to_string())?;
            if !build.finalized {
                program::place_entrance_and_exit(build.ride_id, &build.station_tiles())?;
                host::ride_set_status(build.ride_id, 2).map_err(|e| {
                    format!(
                        "set_status(testing): {e} [track starts at {} and currently ends at {}; a closed circuit requires these to match]",
                        cursor_json(&build.start),
                        cursor_json(&build.cursor)
                    )
                })?;
                build.finalized = true;
            }
            host::run_ticks(ticks);
            host::update_ratings(build.ride_id);
            let ride_id = build.ride_id;
            let placed: Vec<u16> = build.placed.iter().map(|p| p.track_type).collect();
            let eval_report = report::build(None, Some(ride_id), Some(&placed), library);
            let mut report =
                serde_json::to_value(&eval_report).map_err(|e| format!("report serialise: {e}"))?;
            // Snapshot the piece list and start alongside the ratings, so the
            // harness can rebuild program.json for the scored coaster rather
            // than for whatever currently stands in the park.
            if let Some(obj) = report.as_object_mut() {
                obj.insert(
                    "placed_pieces".into(),
                    json!(placed_pieces_json(&build.placed)),
                );
                obj.insert("start".into(), cursor_json(&build.start));
            }
            // Remember the best rated test this round, so demolishing a good
            // coaster to chase a better one can't lose the good one. Snapshot
            // the park at the same moment so the scored ride can still be
            // pictured after a later rebuild replaces it.
            if let Some(excitement) = report_excitement(&report) {
                let prev = session.best_test.as_ref().and_then(report_excitement);
                if prev.is_none_or(|b| excitement > b) {
                    session.best_test = Some(report.clone());
                    session.best_shot = capture_park_png().ok();
                }
            }
            Ok(text_content(report))
        }
        "best_result" => {
            // The best rated test of the round, regardless of what currently
            // stands in the park. The harness scores this; a model may also
            // call it to see whether a rebuild actually beat its earlier work.
            match &session.best_test {
                Some(report) => Ok(text_content(report.clone())),
                None => Err("no rated test yet this round".into()),
            }
        }
        "best_screenshot" => {
            // The park as it stood when best_test was set; the harness saves
            // this as the round's artifact so the picture matches the score.
            match &session.best_shot {
                Some(bytes) => Ok(image_content(bytes.clone())),
                None => Err("no rated test yet this round".into()),
            }
        }
        "screenshot" => Ok(image_content(capture_park_png()?)),
        "undo_piece" => {
            let build = session.open()?;
            let last = build.placed.pop().ok_or("no pieces to undo")?;
            if let Err(e) = host::track_remove(build.ride_id, last.track_type, &last.origin) {
                // Put it back so state stays truthful.
                build.placed.push(last);
                return Err(format!("undo failed: {e}"));
            }
            build.cursor = last.origin;
            Ok(text_content(json!({
                "removed": last.name,
                "cursor": cursor_json(&build.cursor),
                "pieces_placed": build.placed.len(),
            })))
        }
        "piece_geometry" => {
            let dir = (arg_i64(args, "dir").unwrap_or(0) as u8) & 3;
            let table: Vec<Value> = pieces::CATALOG
                .iter()
                .filter_map(|&(name, id)| {
                    host::piece_delta(id, dir).map(|d| {
                        json!({
                            "piece": name,
                            "dx_tiles": d.x / 32,
                            "dy_tiles": d.y / 32,
                            "dz": d.z,
                            "dir_out": d.direction,
                            "bank_out": d.bank,
                            "slope_out": d.slope,
                        })
                    })
                })
                .collect();
            Ok(text_content(json!({"dir_in": dir, "deltas": table})))
        }
        "demolish" => {
            let build = session.build.take().ok_or("no active ride")?;
            host::ride_demolish(build.ride_id)?;
            Ok(text_content(json!({"demolished": build.ride_id})))
        }
        "search_track_designs" => {
            let ride_type = arg_i64(args, "ride_type").map(|t| t as u16);
            let designs: Vec<Value> = session
                .library()?
                .iter()
                .filter(|d| ride_type.is_none() || ride_type == Some(d.ride_type))
                .map(|d| {
                    json!({
                        "name": d.name,
                        "ride_type": d.ride_type,
                        "piece_count": d.pieces.len(),
                    })
                })
                .collect();
            Ok(text_content(json!({
                "designs": designs,
                "note": "final score is penalized for similarity to any of these designs",
            })))
        }
        "get_track_design" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or("name required")?;
            let design = session
                .library()?
                .iter()
                .find(|d| d.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no such design: {name}"))?;
            Ok(text_content(json!({
                "name": design.name,
                "ride_type": design.ride_type,
                "piece_count": design.pieces.len(),
                "pieces": library::program_pieces(design),
            })))
        }
        "place_pieces" => {
            let build = session.open()?;
            let pieces_arg = args
                .get("pieces")
                .and_then(Value::as_array)
                .ok_or("pieces array required")?;
            if pieces_arg.is_empty() {
                return Err("pieces array cannot be empty".into());
            }
            if pieces_arg.len() > 200 {
                return Err(format!(
                    "pieces array too large ({} > 200 max)",
                    pieces_arg.len()
                ));
            }

            let mut placed_this_call = 0usize;
            let mut rejection: Option<Value> = None;

            for (index, piece_val) in pieces_arg.iter().enumerate() {
                let spec: pieces::PieceSpec = serde_json::from_value(piece_val.clone())
                    .map_err(|e| format!("piece[{index}]: {e}"))?;
                let track_type = spec
                    .track_type()
                    .map_err(|e| format!("piece[{index}]: {e}"))?;
                let piece_name = pieces::name_of(track_type)
                    .ok_or_else(|| format!("piece[{index}]: unknown piece #{track_type}"))?;
                let (_, chain, speed) = spec.parts();
                match build.place(piece_name, chain, speed) {
                    Ok(_) => placed_this_call += 1,
                    Err(e) => {
                        rejection = Some(json!({
                            "index": index,
                            "piece": piece_name,
                            "error": e,
                        }));
                        break;
                    }
                }
            }

            let mut resp = json!({
                "placed_this_call": placed_this_call,
                "total_placed": build.placed.len(),
                "cursor": cursor_json(&build.cursor),
                "circuit_closed": build.cursor == build.start,
            });
            if let Some(rej) = rejection {
                resp["rejection"] = rej;
            }
            Ok(text_content(resp))
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn cursor_json(c: &host::TrackCursor) -> Value {
    json!({
        "x": c.x / 32,
        "y": c.y / 32,
        "z": c.z,
        "dir": c.direction,
        "bank": c.bank,
        "slope": c.slope
    })
}

#[cfg(test)]
mod tests {
    use crate::mcp::*;

    #[test]
    fn dispatch_initialize_echoes_protocol_version() {
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                         "params": {"protocolVersion": "2025-03-26"}});
        let resp = dispatch(&msg, &mut session, Modalities::ALL, Source::Mcp);
        assert_eq!(
            resp.pointer("/result/protocolVersion")
                .and_then(Value::as_str),
            Some("2025-03-26")
        );
        assert!(resp.pointer("/result/capabilities/tools").is_some());
    }

    #[test]
    fn placed_pieces_keep_chain_in_program_form() {
        let cursor = host::TrackCursor {
            x: 0,
            y: 0,
            z: 0,
            direction: 0,
            bank: 0,
            slope: 0,
        };
        let placed = vec![
            Placed {
                name: "begin_station",
                track_type: 1,
                origin: cursor,
                cost: 0,
                chain: false,
                speed: None,
            },
            Placed {
                name: "up_25",
                track_type: 5,
                origin: cursor,
                cost: 0,
                chain: true,
                speed: None,
            },
            Placed {
                name: "booster",
                track_type: 100,
                origin: cursor,
                cost: 0,
                chain: false,
                speed: Some(20),
            },
        ];
        let out = placed_pieces_json(&placed);
        // Plain pieces stay bare names; a chain piece becomes the {"t",chain}
        // object program.rs parses, so program.json round-trips the lift hill.
        assert_eq!(out[0], json!("begin_station"));
        assert_eq!(out[1], json!({"t": "up_25", "chain": true}));
        // Likewise the booster speed: replaying without it gives a booster that
        // does nothing, which is a different ride.
        assert_eq!(out[2], json!({"t": "booster", "speed": 20}));
    }

    #[test]
    fn dispatch_unknown_method_is_rpc_error() {
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 2, "method": "bogus"});
        let resp = dispatch(&msg, &mut session, Modalities::ALL, Source::Mcp);
        assert_eq!(
            resp.pointer("/error/code").and_then(Value::as_i64),
            Some(-32601)
        );
    }

    /// save_park writes a file on the host. The listener a request arrives on
    /// picks the table it is dispatched against, so an agent is not merely
    /// refused the tool: nothing in the table its requests reach has that name.
    #[test]
    fn control_tools_exist_only_on_the_control_listener() {
        let mut session = Session::default();
        let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        let names = |resp: &Value| -> Vec<String> {
            resp.pointer("/result/tools")
                .and_then(Value::as_array)
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|t| t.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };

        // The two tables are disjoint: everything the harness can ask for
        // writes this filesystem or drives the clock, and nothing an agent can
        // ask for does.
        let control = dispatch(&list, &mut session, Modalities::ALL, Source::Control);
        let agent_names = names(&dispatch(&list, &mut session, Modalities::ALL, Source::Mcp));
        assert!(names(&control).contains(&"save_park".to_string()));
        assert!(names(&control).contains(&"capture_replay".to_string()));
        assert!(
            names(&control).iter().all(|t| !agent_names.contains(t)),
            "control tools leaked into the agent table: {:?}",
            names(&control)
        );

        let agent = dispatch(&list, &mut session, Modalities::ALL, Source::Mcp);
        assert!(
            !names(&agent).contains(&"save_park".to_string()),
            "a sandboxed agent must not be offered a host filesystem write"
        );
        assert!(names(&agent).contains(&"place_piece".to_string()));

        // Naming it explicitly does not help either: the agent table has no
        // such tool to call.
        let call = json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "save_park", "arguments": {"path": "/tmp/should-not-happen.park"}}
        });
        let refused = dispatch(&call, &mut session, Modalities::ALL, Source::Mcp);
        assert_eq!(
            refused.pointer("/result/isError").and_then(Value::as_bool),
            Some(true)
        );

        // And the building tools are not reachable from the control plane.
        let build = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "get_state", "arguments": {}}
        });
        let wrong_plane = dispatch(&build, &mut session, Modalities::ALL, Source::Control);
        assert_eq!(
            wrong_plane
                .pointer("/result/isError")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn tools_list_contains_the_full_toolset() {
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"});
        let resp = dispatch(&msg, &mut session, Modalities::ALL, Source::Mcp);
        let tools = resp
            .pointer("/result/tools")
            .and_then(Value::as_array)
            .expect("tools array");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        for expected in [
            "new_ride",
            "place_piece",
            "place_pieces",
            "valid_next_pieces",
            "get_state",
            "finish_and_test",
            "best_result",
            "best_screenshot",
            "screenshot",
            "demolish",
            "search_track_designs",
            "get_track_design",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
    }

    #[test]
    fn text_only_clients_never_see_the_screenshot_tool() {
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"});
        let resp = dispatch(&msg, &mut session, Modalities::TEXT, Source::Mcp);
        let names: Vec<&str> = resp
            .pointer("/result/tools")
            .and_then(Value::as_array)
            .expect("tools array")
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        assert!(!names.contains(&"screenshot"));
        assert!(names.contains(&"place_piece"), "only screenshot is hidden");
    }

    #[test]
    fn screenshot_call_is_refused_for_text_only_clients() {
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call",
                         "params": {"name": "screenshot", "arguments": {}}});
        let resp = dispatch(&msg, &mut session, Modalities::TEXT, Source::Mcp);
        assert_eq!(
            resp.pointer("/result/isError").and_then(Value::as_bool),
            Some(true)
        );
        let text = resp
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(text.contains("unavailable"), "got {text}");
    }

    #[test]
    fn report_excitement_picks_the_best_rated_ride() {
        assert_eq!(report_excitement(&json!({"rides": []})), None);
        assert_eq!(
            report_excitement(&json!({"rides": [{"excitement": null}]})),
            None,
            "an unrated ride does not count"
        );
        assert_eq!(
            report_excitement(&json!({"rides": [{"excitement": 5.89}, {"excitement": 6.08}]})),
            Some(6.08)
        );
    }

    #[test]
    fn claiming_the_park_clears_the_previous_rounds_best() {
        let mut session = Session {
            best_test: Some(json!({"rides": [{"excitement": 6.08}]})),
            ..Default::default()
        };
        // A same-lease request (no claim) leaves the best in place.
        Claim::from_request_target("/mcp").authorise(&mut session);
        assert!(session.best_test.is_some());
        // The next round claiming the park wipes it.
        Claim::from_request_target("/mcp?lease=r2&claim=1").authorise(&mut session);
        assert!(
            session.best_test.is_none(),
            "a fresh round must not inherit the last round's score"
        );
    }

    #[test]
    fn an_unleased_park_serves_everyone() {
        let mut session = Session::default();
        assert!(Claim::from_request_target("/mcp").authorise(&mut session));
        assert!(Claim::from_request_target("/mcp?lease=whatever").authorise(&mut session));
    }

    #[test]
    fn a_claim_locks_the_park_to_one_lease() {
        let mut session = Session::default();
        assert!(Claim::from_request_target("/mcp?lease=round1&claim=1").authorise(&mut session));
        assert_eq!(session.lease.as_deref(), Some("round1"));

        // The holder keeps working; everyone else is refused.
        assert!(Claim::from_request_target("/mcp?lease=round1").authorise(&mut session));
        assert!(!Claim::from_request_target("/mcp?lease=round0").authorise(&mut session));
        assert!(!Claim::from_request_target("/mcp").authorise(&mut session));
    }

    #[test]
    fn the_next_round_takes_the_park_from_the_last_one() {
        let mut session = Session::default();
        Claim::from_request_target("/mcp?lease=round1&claim=1").authorise(&mut session);
        assert!(Claim::from_request_target("/mcp?lease=round2&claim=1").authorise(&mut session));
        assert_eq!(session.lease.as_deref(), Some("round2"));
        // The previous round's agent, still alive somewhere, is now locked out.
        assert!(!Claim::from_request_target("/mcp?lease=round1").authorise(&mut session));
    }

    #[test]
    fn lease_and_modalities_coexist_in_one_query() {
        let mut session = Session::default();
        let target = "/mcp?modalities=text&lease=r1&claim=1";
        assert!(Claim::from_request_target(target).authorise(&mut session));
        assert_eq!(session.lease.as_deref(), Some("r1"));
        assert_eq!(
            Modalities::from_request_target(target),
            Modalities::TEXT,
            "the lease param does not disturb modality parsing"
        );
    }

    #[test]
    fn modalities_parse_from_the_request_target() {
        assert_eq!(Modalities::from_request_target("/mcp"), Modalities::ALL);
        assert_eq!(
            Modalities::from_request_target("/mcp?modalities=text"),
            Modalities::TEXT
        );
        assert_eq!(
            Modalities::from_request_target("/mcp?foo=1&modalities=text,image"),
            Modalities::TEXT | Modalities::IMAGE
        );
        // Unknown names drop out, a known one still lands.
        assert_eq!(
            Modalities::from_request_target("/mcp?modalities=text,hologram"),
            Modalities::TEXT
        );
    }

    #[test]
    fn a_modality_set_contains_its_members_but_not_others() {
        let text_and_image = Modalities::TEXT | Modalities::IMAGE;
        assert!(text_and_image.contains(Modalities::TEXT));
        assert!(text_and_image.contains(Modalities::IMAGE));
        assert!(!text_and_image.contains(Modalities::AUDIO));
        assert!(Modalities::ALL.contains(text_and_image));
        assert!(!Modalities::TEXT.contains(text_and_image));
    }

    #[test]
    fn search_without_a_library_errors_cleanly() {
        // The test stub host has no track library; the tool should report
        // that as a tool error, not panic.
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call",
                         "params": {"name": "search_track_designs", "arguments": {}}});
        let resp = dispatch(&msg, &mut session, Modalities::ALL, Source::Mcp);
        assert_eq!(
            resp.pointer("/result/isError").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn tool_call_without_ride_errors_cleanly() {
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                         "params": {"name": "get_state", "arguments": {}}});
        let resp = dispatch(&msg, &mut session, Modalities::ALL, Source::Mcp);
        assert_eq!(
            resp.pointer("/result/isError").and_then(Value::as_bool),
            Some(true)
        );
    }
}
