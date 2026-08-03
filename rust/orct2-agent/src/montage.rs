//! Build montage: the track assembling itself, piece by piece.
//!
//! A replay shows the finished ride running; a montage shows how the agent got
//! there. Same encoder and the same crop as the replay, so a round's three
//! artifacts (park.png, replay.mp4, montage.mp4) all frame the coaster
//! identically. On the index hero the two clips play back to back, which only
//! works because the camera is shared: the montage's last frame and the
//! replay's first are the same picture.
//!
//! Three kinds, all filmed by the same reel:
//!
//! - `film_build`, one round assembling itself from its recorded program.
//! - `film_evolution`, a whole run: every round in order, each torn down for
//!   the next, ending on the champion. The camera is the union of all their
//!   bounding boxes, so a coaster that grew looks like it grew.
//! - `film_trace`, the agent's own session: the calls the game accepted, in the
//!   order it accepted them, undos and demolitions included. Where the other
//!   two replay a finished result, this replays the working. Only worth
//!   filming for a model that builds incrementally; one that submits whole
//!   programs looks exactly like its build montage.
//!
//! The camera is fixed in all three, and measured before anything is filmed.
//! Framing each frame to the track as it stands would zoom out on every piece
//! and turn the clip into one long dolly shot.

use crate::host;
use crate::pieces::{self, PieceSpec};
use crate::presentation::RidePresentation;
use crate::program;
use crate::replay;

/// Frames per second of the finished clip. Nothing is simulated while filming,
/// so this is purely how fast placements read.
const FPS: usize = 20;
/// A single round's build runs this long whatever the piece count: a 40-piece
/// coaster at one frame per piece is over before it registers, a 1299-piece one
/// runs a minute.
const MIN_BUILD_FRAMES: usize = FPS * 8;
const MAX_BUILD_FRAMES: usize = FPS * 15;
/// Ceiling on distinct renders. Rendering is the expensive half of filming, and
/// past this the extra stages are below a viewer's resolution anyway.
const MAX_STAGES: usize = MAX_BUILD_FRAMES;
/// Frames of finished track at the end, so the coaster reads before the cut.
const HOLD_FRAMES: usize = FPS * 2;
/// Frames every round of an evolution share. The per-round holds and the final
/// one land on top, which is what puts the whole clip in the 15-20s band.
const EVOLUTION_BUILD_FRAMES: usize = FPS * 15;
/// Beat on a finished round before it is torn down for the next.
const SEGMENT_HOLD_FRAMES: usize = FPS / 2;

pub struct Montage {
    pub frames: usize,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub segments: Vec<SegmentReport>,
    /// Every segment built in full. A program that fails partway still yields a
    /// montage; it just ends where the build ended.
    pub ok: bool,
    /// The camera every frame was shot on, so a clip meant to run straight on
    /// from this one can be shot from the same place.
    pub camera: host::TrackBounds,
    /// The ride left standing at the end: the last round built, still in
    /// testing. None if it never got that far.
    pub final_ride: Option<u16>,
}

impl Montage {
    pub fn pieces_placed(&self) -> usize {
        self.segments.iter().map(|s| s.pieces_placed).sum()
    }
}

pub struct SegmentReport {
    pub pieces_placed: usize,
    pub pieces_total: usize,
    pub pacing: Pacing,
}

/// One round of an evolution: its program, and the colours it banked.
pub struct Segment {
    pub program_json: String,
    pub style: Option<RidePresentation>,
}

/// One thing the agent did that changed the park, distilled from a round's
/// trace.jsonl. Only calls the game accepted appear: a rejected placement
/// changed nothing, so filming it would show a frame identical to the last.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TraceAction {
    NewRide {
        ride_type: u16,
        x: i32,
        y: i32,
        dir: u8,
    },
    /// `place_piece` and `place_pieces` both land here; a single placement is a
    /// batch of one. The trace's own arguments parse as PieceSpec unchanged.
    Place {
        pieces: Vec<PieceSpec>,
    },
    /// `undo_piece`: the last placement comes back off the map.
    Undo,
    Demolish,
    /// `finish_and_test`: entrance, exit and the flip to testing.
    Test,
}

/// How a piece list is spread over the frames it has been given.
///
/// Two independent knobs, because a short program and a long one fail the
/// target duration in opposite directions: `pieces_per_stage` thins out the
/// renders of a long program, `build_frames` (> `stages`) holds each render of
/// a short one on screen for more than one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pacing {
    /// Placements between renders. >1 once there are more pieces than frames.
    pub pieces_per_stage: usize,
    /// Distinct rendered stages of the build.
    pub stages: usize,
    /// Frames the build occupies.
    pub build_frames: usize,
}

impl Pacing {
    /// Spreads `pieces` over exactly `budget` frames, thinning the renders when
    /// there are more pieces than frames to show them in.
    fn for_budget(pieces: usize, budget: usize) -> Pacing {
        let budget = budget.max(1);
        let pieces_per_stage = pieces.div_ceil(budget).max(1);
        Pacing {
            pieces_per_stage,
            stages: pieces.div_ceil(pieces_per_stage),
            build_frames: budget,
        }
    }

    /// A single round's build, which owns the whole clip and so sets its own
    /// length.
    fn plan(pieces: usize) -> Pacing {
        let stages = pieces.div_ceil(pieces.div_ceil(MAX_STAGES).max(1));
        Pacing::for_budget(pieces, stages.clamp(MIN_BUILD_FRAMES, MAX_BUILD_FRAMES))
    }

    /// Frames the build has spent once `stage` stages are on screen. Frames per
    /// stage come out of the difference, so rounding never accumulates and the
    /// build lands on exactly `build_frames`.
    fn frames_through(&self, stage: usize) -> usize {
        if self.stages == 0 {
            return 0;
        }
        (stage * self.build_frames).div_ceil(self.stages)
    }
}

/// Splits `budget` frames between rounds in proportion to their piece counts,
/// so a round that grew takes longer to build on screen. Sums to `budget`.
fn shares(pieces: &[usize], budget: usize) -> Vec<usize> {
    let total: usize = pieces.iter().sum();
    if total == 0 {
        return vec![0; pieces.len()];
    }
    let mut cumulative = 0usize;
    let mut spent = 0usize;
    pieces
        .iter()
        .map(|&count| {
            cumulative += count;
            let through = cumulative * budget / total;
            let share = through - spent;
            spent = through;
            share
        })
        .collect()
}

/// The frames going to ffmpeg, and the camera they are all shot on.
struct Reel {
    sink: std::process::ChildStdin,
    buf: Vec<u8>,
    bounds: host::TrackBounds,
    zoom: i32,
    written: usize,
    /// Bytes of the last good frame, so a hold can repeat it.
    last_len: usize,
    /// The render or the pipe failed; everything after is skipped so the clip
    /// ends cleanly instead of half-written.
    broken: bool,
}

impl Reel {
    /// Renders the park once and writes it `copies` times. The encoder collapses
    /// the repeats to nothing, so holding a frame is free.
    fn render(&mut self, copies: usize) {
        if self.broken {
            return;
        }
        let n = host::capture_frame(self.zoom, 0, Some(&self.bounds), &mut self.buf);
        if n == 0 {
            self.broken = true;
            return;
        }
        self.last_len = n;
        self.repeat(n, copies);
    }

    /// Holds the last rendered frame for `frames` more frames.
    fn hold(&mut self, frames: usize) {
        if self.broken || self.last_len == 0 {
            return;
        }
        self.repeat(self.last_len, frames);
    }

    fn repeat(&mut self, len: usize, times: usize) {
        for _ in 0..times {
            if std::io::Write::write_all(&mut self.sink, &self.buf[..len]).is_err() {
                self.broken = true;
                return;
            }
            self.written += 1;
        }
    }

    /// Runs one program, filming it at `pacing`.
    fn build(
        &mut self,
        program_json: &str,
        style: Option<&RidePresentation>,
        pacing: Pacing,
    ) -> program::ProgramOutcome {
        let mut stage = 0usize;
        let mut styled = false;
        let mut emit = |ride_id: u16, index: usize| {
            // Render on the last piece of each batch, and always on the first,
            // so a segment opens on the station rather than on nothing.
            let step = pacing.pieces_per_stage;
            if self.broken || (index % step != step - 1 && index != 0) {
                return;
            }
            // Colour goes on once the ride exists, which is as soon as its
            // first piece is down, so every frame is the ride's own colour.
            if let (Some(style), false) = (style, styled) {
                styled = true;
                if let Err(e) = style.apply(ride_id) {
                    host::log(&format!("orct2-agent: montage style failed: {e}"));
                }
            }
            let copies = pacing.frames_through(stage + 1) - pacing.frames_through(stage);
            stage += 1;
            self.render(copies);
        };
        program::run_observed(program_json, &mut emit)
    }
}

/// A build in progress, driven one trace action at a time.
///
/// Holds what the MCP session held: the ride, the cursor, and enough of a
/// history to undo. Trace actions are replayed against the live game exactly as
/// the agent's tool calls were, which is the point of the clip.
#[derive(Default)]
struct TraceBuild {
    ride: Option<u16>,
    cursor: host::TrackCursor,
    /// Placed pieces, newest last: (type, the cursor it was placed from).
    /// `undo_piece` pops one and puts the cursor back where it was.
    placed: Vec<(u16, host::TrackCursor)>,
    station_tiles: Vec<(i32, i32)>,
}

impl TraceBuild {
    /// Applies one action; Err is reported and skipped rather than fatal. A
    /// trace records what the game accepted at the time, but a rerun happens on
    /// a later build of the game (see the drawability note in CLAUDE.md), so a
    /// call can be refused now that was accepted then.
    fn apply(&mut self, action: &TraceAction) -> Result<(), String> {
        match action {
            TraceAction::NewRide {
                ride_type,
                x,
                y,
                dir,
            } => {
                if !host::enable_sandbox() {
                    host::log("orct2-agent: warning: sandbox cheat failed");
                }
                // The server's new_ride demolishes the session's previous ride
                // (mcp.rs). Without doing the same, a second new_ride leaves
                // the first coaster standing and every placement after it is
                // refused with "Twister Roller Coaster 1 in the way".
                if let Some(previous) = self.ride.take() {
                    let _ = host::ride_demolish(previous);
                }
                self.ride = Some(host::ride_create(*ride_type)?);
                self.cursor = program::start_cursor(*x, *y, *dir);
                self.placed.clear();
                self.station_tiles.clear();
                Ok(())
            }
            TraceAction::Place { pieces } => {
                for piece in pieces {
                    self.place_one(piece)?;
                }
                Ok(())
            }
            TraceAction::Undo => {
                let ride = self.ride.ok_or("undo before new_ride")?;
                let (track_type, from) = self.placed.pop().ok_or("nothing to undo")?;
                host::track_remove(ride, track_type, &from)?;
                if (1..=3).contains(&track_type) {
                    self.station_tiles.pop();
                }
                self.cursor = from;
                Ok(())
            }
            TraceAction::Demolish => {
                let ride = self.ride.take().ok_or("demolish before new_ride")?;
                self.placed.clear();
                self.station_tiles.clear();
                host::ride_demolish(ride)
            }
            TraceAction::Test => {
                let ride = self.ride.ok_or("test before new_ride")?;
                program::place_entrance_and_exit(ride, &self.station_tiles)?;
                host::ride_set_status(ride, 2)
            }
        }
    }

    /// Places one piece and advances the cursor, remembering where it came
    /// from so `undo_piece` can put both back.
    fn place_one(&mut self, piece: &PieceSpec) -> Result<(), String> {
        let ride = self.ride.ok_or("place before new_ride")?;
        let (_, chain, speed) = piece.parts();
        let track_type = piece.track_type()?;
        let speed = pieces::resolve_speed(track_type, speed)?;
        let from = self.cursor;
        host::track_place(ride, track_type, chain, speed, &mut self.cursor)?;
        self.placed.push((track_type, from));
        if (1..=3).contains(&track_type) {
            self.station_tiles.push((from.x / 32, from.y / 32));
        }
        Ok(())
    }

    /// What is still standing, in the order it went down. The verification bar:
    /// replaying the trace has to leave the round's recorded program.
    fn surviving(&self) -> Vec<String> {
        self.placed
            .iter()
            .map(|(track_type, _)| {
                pieces::name_of(*track_type).map_or_else(|| format!("#{track_type}"), str::to_owned)
            })
            .collect()
    }
}

fn piece_count(program_json: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(program_json)
        .ok()
        .and_then(|v| v.get("pieces").and_then(|p| p.as_array()).map(Vec::len))
        .unwrap_or(0)
}

fn merge(a: host::TrackBounds, b: host::TrackBounds) -> host::TrackBounds {
    host::TrackBounds {
        min_tile_x: a.min_tile_x.min(b.min_tile_x),
        min_tile_y: a.min_tile_y.min(b.min_tile_y),
        max_tile_x: a.max_tile_x.max(b.max_tile_x),
        max_tile_y: a.max_tile_y.max(b.max_tile_y),
        min_z: a.min_z.min(b.min_z),
        max_z: a.max_z.max(b.max_z),
    }
}

/// Builds every segment once, unioning what each one occupies, and leaves the
/// park empty again. The camera has to be known before the first frame, and
/// only the game can say how much room a program takes.
fn union_bounds(segments: &[Segment]) -> Option<host::TrackBounds> {
    let mut union: Option<host::TrackBounds> = None;
    for (index, segment) in segments.iter().enumerate() {
        let outcome = program::run(&segment.program_json);
        if let Some(bounds) = host::track_bounds() {
            union = Some(union.map_or(bounds, |u| merge(u, bounds)));
        } else {
            host::log(&format!(
                "orct2-agent: evolution round {index} built no track"
            ));
        }
        if let Some(ride) = outcome.ride_id {
            if let Err(e) = host::ride_demolish(ride) {
                // A ride left standing would join the next round's bounds and
                // then show up in its frames, so this is fatal to the measure.
                host::log(&format!("orct2-agent: evolution measure demolish: {e}"));
                return None;
            }
        }
    }
    union
}

/// Opens the encoder and films `shoot` into it, then writes the poster and the
/// sidecar. Common tail of both montage kinds.
fn record<F>(
    out: &str,
    bounds: host::TrackBounds,
    zoom: i32,
    planned_frames: usize,
    kind: &str,
    extra: serde_json::Value,
    shoot: F,
) -> Result<Montage, String>
where
    F: FnOnce(&mut Reel) -> (Vec<SegmentReport>, Option<u16>),
{
    let (width, height) =
        host::capture_size(zoom, 0, Some(&bounds)).ok_or("montage camera has no size")?;
    let mut child = replay::spawn_encoder(out, width, height, FPS as f64, planned_frames)?;
    let sink = child.stdin.take().ok_or("ffmpeg stdin")?;
    let mut reel = Reel {
        sink,
        buf: vec![0u8; width as usize * height as usize * 4],
        bounds,
        zoom,
        written: 0,
        last_len: 0,
        broken: false,
    };

    let (segments, final_ride) = shoot(&mut reel);
    let Reel {
        sink,
        written,
        broken,
        ..
    } = reel;
    drop(sink);
    let status = child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg failed: {status}"));
    }
    if written == 0 {
        return Err("no frames rendered".into());
    }

    // Shot on the montage's own camera, not refitted to the track: the poster
    // is what a viewer sees until the first frame decodes, and a poster framed
    // differently jumps the moment the clip starts.
    let poster = replay::poster_path(out);
    if !host::capture_framed(&poster, zoom, 0, Some(&bounds), false) {
        host::log(&format!("orct2-agent: montage poster failed ({poster})"));
    }

    let fps = FPS as f64;
    let built_in_full = segments
        .iter()
        .all(|s| s.pieces_total > 0 && s.pieces_placed == s.pieces_total);
    let mut meta = serde_json::json!({
        "kind": kind,
        "frames": written,
        "fps": fps,
        "seconds": written as f64 / fps,
        "hold_frames": HOLD_FRAMES,
        "segments": segments.iter().map(|s| serde_json::json!({
            "pieces_placed": s.pieces_placed,
            "pieces_total": s.pieces_total,
            "pieces_per_stage": s.pacing.pieces_per_stage,
            "stages": s.pacing.stages,
            "build_frames": s.pacing.build_frames,
        })).collect::<Vec<_>>(),
    });
    if let (Some(meta), Some(extra)) = (meta.as_object_mut(), extra.as_object()) {
        meta.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    if let Ok(text) = serde_json::to_string_pretty(&meta) {
        let _ = std::fs::write(replay::meta_path(out), text);
    }

    Ok(Montage {
        frames: written,
        width,
        height,
        fps,
        segments,
        ok: built_in_full && !broken,
        camera: bounds,
        final_ride,
    })
}

/// Films `program_json` being built into `out` as an mp4.
///
/// Requires the program to have already been built, since `existing_ride` is
/// torn down to film the same build from an empty map and the finished track is
/// what fixes the camera.
///
/// `style` is the round's `style_best_ride` colours, applied to the rebuilt
/// ride before the first frame. Without it a styled round's montage is stock
/// gold and the cut to its replay changes the coaster's colour.
pub fn film_build(
    out: &str,
    program_json: &str,
    style: Option<&RidePresentation>,
    existing_ride: u16,
    zoom: i32,
) -> Result<Montage, String> {
    let bounds = host::track_bounds().ok_or("no track to frame the montage on")?;
    host::ride_demolish(existing_ride)
        .map_err(|e| format!("demolish {existing_ride} before refilming: {e}"))?;

    let pacing = Pacing::plan(piece_count(program_json));
    let planned = pacing.build_frames + 1 + HOLD_FRAMES;
    record(
        out,
        bounds,
        zoom,
        planned,
        "build-montage",
        serde_json::Value::Null,
        |reel| {
            let outcome = reel.build(program_json, style, pacing);
            // The last frame the build wrote predates the entrance, exit and the
            // flip to testing, so take one more of the finished ride to hold on.
            reel.render(1);
            reel.hold(HOLD_FRAMES);
            (
                vec![SegmentReport {
                    pieces_placed: outcome.pieces_placed,
                    pieces_total: outcome.pieces_total,
                    pacing,
                }],
                outcome.ride_id,
            )
        },
    )
}

/// Films a whole run into `out`: every round built in order, each torn down for
/// the next, ending held on the last one.
///
/// The camera is the union of every round's footprint, measured by building
/// them all once first, so the frame never moves and a round that grew looks
/// like it grew. Expects a park with no track of its own.
pub fn film_evolution(out: &str, segments: &[Segment], zoom: i32) -> Result<Montage, String> {
    if segments.is_empty() {
        return Err("evolution needs at least one round".into());
    }
    let bounds = union_bounds(segments).ok_or("no track in any round to frame the montage on")?;

    let counts: Vec<usize> = segments
        .iter()
        .map(|s| piece_count(&s.program_json))
        .collect();
    let budgets = shares(&counts, EVOLUTION_BUILD_FRAMES);
    // One livery for the whole clip: the last round to have banked a
    // presentation, which is the coaster the montage ends on. Styling each
    // round in its own colours means five stock-gold rounds and a blue finale,
    // and that flip reads as a rendering glitch to anyone who does not know
    // which round happened to call style_best_ride. The montage is
    // presentation; program.json remains the record.
    let livery = segments.iter().rev().find_map(|s| s.style.as_ref());
    let planned = EVOLUTION_BUILD_FRAMES
        + segments.len()
        + SEGMENT_HOLD_FRAMES * segments.len().saturating_sub(1)
        + HOLD_FRAMES;

    record(
        out,
        bounds,
        zoom,
        planned,
        "evolution-montage",
        serde_json::Value::Null,
        |reel| {
            let mut reports = Vec::with_capacity(segments.len());
            let mut final_ride = None;
            for (index, segment) in segments.iter().enumerate() {
                let pacing = Pacing::for_budget(counts[index], budgets[index]);
                let outcome = reel.build(&segment.program_json, livery, pacing);
                reel.render(1);
                let last = index + 1 == segments.len();
                reel.hold(if last {
                    HOLD_FRAMES
                } else {
                    SEGMENT_HOLD_FRAMES
                });
                reports.push(SegmentReport {
                    pieces_placed: outcome.pieces_placed,
                    pieces_total: outcome.pieces_total,
                    pacing,
                });
                // Clear the frame for the next round. The last one stays up: it is
                // the champion, and the hero cuts from it to its own lap.
                if last {
                    final_ride = outcome.ride_id;
                } else if let Some(ride) = outcome.ride_id {
                    if let Err(e) = host::ride_demolish(ride) {
                        host::log(&format!("orct2-agent: evolution demolish: {e}"));
                    }
                }
            }
            (reports, final_ride)
        },
    )
}

/// Visual steps in a trace: one per placement, plus one for each undo,
/// demolition and test. Rejected calls are not in the list at all.
fn trace_steps(actions: &[TraceAction]) -> usize {
    actions
        .iter()
        .map(|action| match action {
            TraceAction::Place { pieces } => pieces.len(),
            TraceAction::NewRide { .. } => 0,
            _ => 1,
        })
        .sum()
}

/// Films a round's recorded session into `out`: what the agent actually did,
/// in the order it did it, demolitions and all.
///
/// Where `film_build` replays the tidy program a round ended up with, this
/// replays the working: pieces going down one at a time, being taken back off,
/// a whole ride demolished and started again. Only worth filming for an agent
/// that builds incrementally; a model that submits whole programs looks
/// identical to its build montage.
///
/// A round that ends demolished ends on an empty park, which is the truth about
/// that round.
pub fn film_trace(out: &str, actions: &[TraceAction], zoom: i32) -> Result<Montage, String> {
    if actions.is_empty() {
        return Err("no accepted actions in the trace".into());
    }
    // Measure pass: the camera has to hold everything the session ever showed,
    // including track it later demolished, or the frame would move.
    let mut union: Option<host::TrackBounds> = None;
    {
        let mut build = TraceBuild::default();
        for action in actions {
            let _ = build.apply(action);
            if let Some(bounds) = host::track_bounds() {
                union = Some(union.map_or(bounds, |u| merge(u, bounds)));
            }
        }
        if let Some(ride) = build.ride {
            host::ride_demolish(ride)?;
        }
    }
    let bounds = union.ok_or("the trace never placed any track")?;

    let steps = trace_steps(actions);
    let pacing = Pacing::plan(steps);
    let demolitions = actions
        .iter()
        .filter(|a| matches!(a, TraceAction::Demolish))
        .count();
    let planned = pacing.build_frames + SEGMENT_HOLD_FRAMES * demolitions + HOLD_FRAMES;

    let mut placed = 0usize;
    let mut refused = 0usize;
    let mut surviving = Vec::new();
    let montage = record(
        out,
        bounds,
        zoom,
        planned,
        "trace-montage",
        serde_json::json!({ "steps": steps, "demolitions": demolitions }),
        |reel| {
            let mut build = TraceBuild::default();
            let mut stage = 0usize;
            let frames_for = |reel: &mut Reel, stage: &mut usize| {
                let copies = pacing.frames_through(*stage + 1) - pacing.frames_through(*stage);
                *stage += 1;
                reel.render(copies);
            };
            for action in actions {
                match action {
                    // Creating a ride changes nothing on screen; the first
                    // piece is the first thing to film.
                    TraceAction::NewRide { .. } => {
                        if let Err(e) = build.apply(action) {
                            refused += 1;
                            host::log(&format!("orct2-agent: trace new_ride refused: {e}"));
                        }
                    }
                    TraceAction::Place { pieces } => {
                        // One at a time even when the trace batched them: the
                        // point of this clip is watching the thing get built.
                        for piece in pieces {
                            match build.place_one(piece) {
                                Ok(()) => placed += 1,
                                Err(e) => {
                                    refused += 1;
                                    host::log(&format!("orct2-agent: trace place refused: {e}"));
                                }
                            }
                            frames_for(reel, &mut stage);
                        }
                    }
                    _ => {
                        if let Err(e) = build.apply(action) {
                            refused += 1;
                            host::log(&format!("orct2-agent: trace action refused: {e}"));
                        }
                        frames_for(reel, &mut stage);
                        // An empty park after a demolition is the story; give
                        // it long enough to register.
                        if matches!(action, TraceAction::Demolish) {
                            reel.hold(SEGMENT_HOLD_FRAMES);
                        }
                    }
                }
            }
            reel.hold(HOLD_FRAMES);
            surviving = build.surviving();
            (
                vec![SegmentReport {
                    pieces_placed: placed,
                    pieces_total: placed + refused,
                    pacing,
                }],
                build.ride,
            )
        },
    )?;

    // The verification bar lives in the sidecar: the surviving track is what a
    // harness checks against the round's recorded program.
    if let Ok(text) = std::fs::read_to_string(replay::meta_path(out)) {
        if let Ok(mut meta) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(object) = meta.as_object_mut() {
                object.insert("surviving_pieces".into(), serde_json::json!(surviving));
                object.insert("refused".into(), serde_json::json!(refused));
                if let Ok(text) = serde_json::to_string_pretty(&meta) {
                    let _ = std::fs::write(replay::meta_path(out), text);
                }
            }
        }
    }
    Ok(montage)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frame counts a plan actually emits, stage by stage.
    fn emitted(pacing: Pacing) -> Vec<usize> {
        (0..pacing.stages)
            .map(|s| pacing.frames_through(s + 1) - pacing.frames_through(s))
            .collect()
    }

    #[test]
    fn every_program_length_builds_for_eight_to_fifteen_seconds() {
        for pieces in [1usize, 16, 40, 139, 200, 299, 300, 301, 700, 1299, 5000] {
            let pacing = Pacing::plan(pieces);
            let seconds = pacing.build_frames as f64 / FPS as f64;
            assert!(
                (8.0..=15.0).contains(&seconds),
                "{pieces} pieces built for {seconds}s"
            );
            assert_eq!(emitted(pacing).iter().sum::<usize>(), pacing.build_frames);
            assert!(
                emitted(pacing).iter().all(|&n| n >= 1),
                "a stage got no frame"
            );
        }
    }

    #[test]
    fn short_programs_hold_each_placement_longer() {
        // 16 pieces, 16 renders, stretched over the 8s floor.
        let pacing = Pacing::plan(16);
        assert_eq!(pacing.pieces_per_stage, 1);
        assert_eq!(pacing.stages, 16);
        assert_eq!(pacing.build_frames, MIN_BUILD_FRAMES);
        assert_eq!(emitted(pacing), vec![10; 16]);
    }

    #[test]
    fn long_programs_batch_pieces_into_stages() {
        // Past MAX_STAGES the renders thin out rather than the clip growing.
        assert_eq!(Pacing::plan(MAX_STAGES).pieces_per_stage, 1);
        assert_eq!(Pacing::plan(MAX_STAGES + 1).pieces_per_stage, 2);
        let big = Pacing::plan(1299);
        assert_eq!(big.pieces_per_stage, 5);
        assert_eq!(big.stages, 260);
        assert_eq!(big.build_frames, 260);
    }

    #[test]
    fn an_empty_program_plans_no_stages_and_no_division_by_zero() {
        let pacing = Pacing::plan(0);
        assert_eq!(pacing.stages, 0);
        assert_eq!(pacing.frames_through(1), 0);
        assert!(emitted(pacing).is_empty());
    }

    #[test]
    fn an_evolution_splits_its_budget_by_piece_count_and_spends_all_of_it() {
        // The hero run: six rounds, the shortest 96 pieces and the longest 134.
        let rounds = [96usize, 122, 128, 134, 134, 134];
        let budgets = shares(&rounds, EVOLUTION_BUILD_FRAMES);
        assert_eq!(budgets.iter().sum::<usize>(), EVOLUTION_BUILD_FRAMES);
        assert!(budgets[0] < budgets[5], "a bigger round gets more frames");
        for (pieces, budget) in rounds.iter().zip(&budgets) {
            let pacing = Pacing::for_budget(*pieces, *budget);
            assert_eq!(pacing.build_frames, *budget);
            assert_eq!(emitted(pacing).iter().sum::<usize>(), *budget);
        }
    }

    #[test]
    fn an_evolution_of_one_round_spends_the_whole_budget_on_it() {
        assert_eq!(
            shares(&[134], EVOLUTION_BUILD_FRAMES),
            vec![EVOLUTION_BUILD_FRAMES]
        );
        // Rounds that built nothing must not divide by zero.
        assert_eq!(shares(&[0, 0], EVOLUTION_BUILD_FRAMES), vec![0, 0]);
        assert!(shares(&[], EVOLUTION_BUILD_FRAMES).is_empty());
    }

    #[test]
    fn the_whole_evolution_lands_in_the_fifteen_to_twenty_second_band() {
        let rounds = [96usize, 122, 128, 134, 134, 134];
        let frames = EVOLUTION_BUILD_FRAMES
            + rounds.len()
            + SEGMENT_HOLD_FRAMES * (rounds.len() - 1)
            + HOLD_FRAMES;
        let seconds = frames as f64 / FPS as f64;
        assert!((15.0..=20.0).contains(&seconds), "evolution ran {seconds}s");
    }

    #[test]
    fn montage_artifacts_sit_beside_the_video() {
        assert_eq!(
            replay::poster_path("round_6/montage.mp4"),
            "round_6/montage.png"
        );
        assert_eq!(
            replay::meta_path("round_6/montage.mp4"),
            "round_6/montage.json"
        );
    }
}
