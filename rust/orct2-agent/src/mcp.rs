//! Minimal MCP (Model Context Protocol) server over streamable HTTP.
//!
//! Hand-rolled on purpose: the game API is strictly single-threaded, so the
//! server runs synchronously ON the game thread — a tool call executes host
//! functions directly, and `run_ticks` advances the simulation inline. No
//! async runtime, no cross-thread marshaling, fully deterministic. The
//! protocol subset (initialize, tools/list, tools/call) is small; if we ever
//! need SSE streaming or concurrent clients, swap in rmcp then.
//!
//! Transport: POST /mcp with a JSON-RPC message, single JSON response
//! (the "streamable HTTP" JSON response mode). GET returns 405.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use base64::Engine;
use serde_json::{json, Value};

use crate::host;
use crate::library::{self, LibraryDesign};
use crate::pieces;
use crate::program;
use crate::report;

/// One placed piece: what and where, so it can be undone.
struct Placed {
    name: &'static str,
    track_type: u16,
    origin: host::TrackCursor,
    cost: i64,
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
}

impl Session {
    fn library(&mut self) -> Result<&[LibraryDesign], String> {
        if self.library.is_none() {
            self.library = Some(library::load()?);
        }
        // The Option was just filled; unwrap_or_default keeps this panic-free.
        Ok(self.library.as_deref().unwrap_or_default())
    }
}

/// Runs the MCP server until the process is killed. Never panics; per-request
/// failures are reported to the client and the listener keeps accepting.
pub fn serve(bind: &str, port: u16) -> Result<(), String> {
    let listener =
        TcpListener::bind((bind, port)).map_err(|e| format!("bind {bind}:{port}: {e}"))?;
    host::log(&format!(
        "orct2-agent: MCP server listening on http://{bind}:{port}/mcp"
    ));

    let mut session = Session::default();
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        // One client at a time, requests handled in arrival order.
        if let Err(e) = handle_connection(stream, &mut session) {
            host::log(&format!("orct2-agent: mcp connection ended: {e}"));
        }
    }
    Ok(())
}

fn handle_connection(stream: TcpStream, session: &mut Session) -> Result<(), String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut stream = stream;
    loop {
        let (method, body) = match read_http_request(&mut reader) {
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
        let response = dispatch(&message, session);
        write_json(&mut stream, &response)?;
    }
}

fn read_http_request(
    reader: &mut BufReader<TcpStream>,
) -> Result<Option<(String, Vec<u8>)>, String> {
    let mut request_line = String::new();
    let n = reader
        .read_line(&mut request_line)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    let method = request_line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
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
    Ok(Some((method, body)))
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

fn dispatch(message: &Value, session: &mut Session) -> Value {
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
        "tools/list" => rpc_result(id, json!({"tools": tool_definitions()})),
        "tools/call" => {
            let name = message
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = message
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
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

fn tool_definitions() -> Value {
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
                "chain": {"type": "boolean", "description": "Chain lift on this piece (for climbing)"}
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
                    "description": "Array of pieces to place sequentially. Each element is either a string (piece name) or an object {\"piece\": \"name\", \"chain\": true/false}.",
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
            let build = session
                .build
                .as_mut()
                .ok_or("no active ride; call new_ride first")?;
            if build.finalized {
                return Err("ride already finalized; use demolish + new_ride to rebuild".into());
            }
            let piece = args
                .get("piece")
                .and_then(Value::as_str)
                .ok_or("piece required")?;
            let chain = args.get("chain").and_then(Value::as_bool).unwrap_or(false);
            let (name, track_type) = pieces::CATALOG
                .iter()
                .find(|(n, _)| *n == piece)
                .copied()
                .ok_or_else(|| format!("unknown piece: {piece}"))?;
            let origin = build.cursor;
            let cost = host::track_place(build.ride_id, track_type, chain, &mut build.cursor)
                .map_err(|e| {
                    format!(
                        "rejected: {e} (cursor unchanged: {})",
                        cursor_json(&build.cursor)
                    )
                })?;
            build.placed.push(Placed {
                name,
                track_type,
                origin,
                cost,
            });
            let closed = build.cursor == build.start;
            Ok(text_content(json!({
                "placed": piece,
                "cost": cost,
                "cursor": cursor_json(&build.cursor),
                "pieces_placed": build.placed.len(),
                "circuit_closed": closed,
            })))
        }
        "valid_next_pieces" => {
            let build = session
                .build
                .as_ref()
                .ok_or("no active ride; call new_ride first")?;
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
            let build = session
                .build
                .as_ref()
                .ok_or("no active ride; call new_ride first")?;
            let placed: Vec<&str> = build.placed.iter().map(|p| p.name).collect();
            Ok(text_content(json!({
                "ride_id": build.ride_id,
                "start": cursor_json(&build.start),
                "cursor": cursor_json(&build.cursor),
                "pieces_placed": build.placed.len(),
                "placed_pieces": placed,
                "total_cost": build.total_cost(),
                "circuit_closed": build.cursor == build.start,
                "finalized": build.finalized,
            })))
        }
        "finish_and_test" => {
            let ticks = arg_i64(args, "ticks").unwrap_or(25000).clamp(1, 200_000) as u32;
            let build = session
                .build
                .as_mut()
                .ok_or("no active ride; call new_ride first")?;
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
            let eval_report = report::build(None, Some(ride_id), Some(&placed));
            serde_json::to_value(&eval_report)
                .map(text_content)
                .map_err(|e| format!("report serialise: {e}"))
        }
        "screenshot" => {
            let path = std::env::temp_dir().join("orct2_mcp_capture.png");
            let path_str = path.to_string_lossy();
            if !host::capture(&path_str, 0, 0) {
                return Err("capture failed".into());
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("read capture: {e}"))?;
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            Ok(vec![
                json!({"type": "image", "data": data, "mimeType": "image/png"}),
            ])
        }
        "undo_piece" => {
            let build = session.build.as_mut().ok_or("no active ride")?;
            if build.finalized {
                return Err("ride already finalized; use demolish + new_ride".into());
            }
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
            let build = session
                .build
                .as_mut()
                .ok_or("no active ride; call new_ride first")?;
            if build.finalized {
                return Err("ride already finalized; use demolish + new_ride to rebuild".into());
            }
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
                let (piece_name, chain) = match piece_val {
                    Value::String(s) => (s.as_str(), false),
                    Value::Object(obj) => {
                        let name = obj
                            .get("piece")
                            .and_then(Value::as_str)
                            .ok_or_else(|| format!("piece[{index}]: missing 'piece' field"))?;
                        let chain = obj.get("chain").and_then(Value::as_bool).unwrap_or(false);
                        (name, chain)
                    }
                    _ => {
                        return Err(format!(
                            "piece[{index}]: expected string or object, got {}",
                            piece_val
                        ))
                    }
                };

                let (catalog_name, track_type) = pieces::CATALOG
                    .iter()
                    .find(|(n, _)| *n == piece_name)
                    .copied()
                    .ok_or_else(|| format!("piece[{index}]: unknown piece: {piece_name}"))?;

                let origin = build.cursor;
                match host::track_place(build.ride_id, track_type, chain, &mut build.cursor) {
                    Ok(cost) => {
                        build.placed.push(Placed {
                            name: catalog_name,
                            track_type,
                            origin,
                            cost,
                        });
                        placed_this_call += 1;
                    }
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

            let closed = build.cursor == build.start;
            let mut resp = json!({
                "placed_this_call": placed_this_call,
                "total_placed": build.placed.len(),
                "cursor": cursor_json(&build.cursor),
                "circuit_closed": closed,
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
        let resp = dispatch(&msg, &mut session);
        assert_eq!(
            resp.pointer("/result/protocolVersion")
                .and_then(Value::as_str),
            Some("2025-03-26")
        );
        assert!(resp.pointer("/result/capabilities/tools").is_some());
    }

    #[test]
    fn dispatch_unknown_method_is_rpc_error() {
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 2, "method": "bogus"});
        let resp = dispatch(&msg, &mut session);
        assert_eq!(
            resp.pointer("/error/code").and_then(Value::as_i64),
            Some(-32601)
        );
    }

    #[test]
    fn tools_list_contains_the_full_toolset() {
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"});
        let resp = dispatch(&msg, &mut session);
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
            "screenshot",
            "demolish",
            "search_track_designs",
            "get_track_design",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
    }

    #[test]
    fn search_without_a_library_errors_cleanly() {
        // The test stub host has no track library; the tool should report
        // that as a tool error, not panic.
        let mut session = Session::default();
        let msg = json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call",
                         "params": {"name": "search_track_designs", "arguments": {}}});
        let resp = dispatch(&msg, &mut session);
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
        let resp = dispatch(&msg, &mut session);
        assert_eq!(
            resp.pointer("/result/isError").and_then(Value::as_bool),
            Some(true)
        );
    }
}
