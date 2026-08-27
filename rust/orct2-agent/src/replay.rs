//! Filming a running ride into an mp4: raw frames piped to ffmpeg. Two
//! callers, the `capture_replay` control tool and `eval --replay`. See
//! CLAUDE.md for the length rule.

use crate::host;

/// Game ticks per second, the game's own clock.
const TICKS_PER_SECOND: u32 = 40;
/// Ticks between frames: every other tick, so 20fps.
const DEFAULT_EVERY: u32 = 2;
/// Seconds of footage past the measured lap, so the train is seen arriving
/// rather than cut off mid-air.
const TAIL_SECONDS: i32 = 3;

pub struct Filmed {
    /// A whole station-to-station cycle, and so loops. False for an excerpt cut
    /// at the cap.
    pub looped: bool,
    pub frames: usize,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub ticks_waited_for_station: u32,
    /// Where the poster still went, when one could be written.
    pub poster: Option<String>,
}

/// The sidecar for `out`: same path, .json extension.
pub fn meta_path(out: &str) -> String {
    match out.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.json"),
        None => format!("{out}.json"),
    }
}

/// The poster still for `out`: same path, .png extension.
pub fn poster_path(out: &str) -> String {
    match out.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.png"),
        None => format!("{out}.png"),
    }
}

/// Widest or tallest a clip may be encoded. Hardware H.264 decoders refuse
/// much above 4096, and a browser that cannot decode shows the first frame and
/// never advances, which reads as a frozen video rather than an error. A track
/// big enough to need it (an evolution's camera is the union of every round)
/// therefore gets scaled at the encoder, leaving the camera itself alone.
const MAX_ENCODED_DIMENSION: u32 = 2560;

/// Encoded size for a frame of `width` x `height`, capped and kept even for
/// yuv420p.
fn encoded_dimensions(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= MAX_ENCODED_DIMENSION {
        return (width, height);
    }
    let scale = f64::from(MAX_ENCODED_DIMENSION) / f64::from(longest);
    let even = |v: u32| (((f64::from(v) * scale).round() as u32) & !1).max(2);
    (even(width), even(height))
}

/// Starts an ffmpeg that takes raw RGBA frames on stdin and writes `out`.
///
/// One keyframe for the whole clip (`-g gop_frames`): the camera never moves in
/// either kind of clip this crate produces, so consecutive frames differ only
/// where the train or the newest piece is, and extra keyframes are pure cost.
pub fn spawn_encoder(
    out: &str,
    width: u32,
    height: u32,
    fps: f64,
    gop_frames: usize,
) -> Result<std::process::Child, String> {
    let (out_width, out_height) = encoded_dimensions(width, height);
    let mut command = std::process::Command::new("ffmpeg");
    command
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{width}x{height}")])
        .args(["-framerate", &format!("{fps}")])
        .args(["-i", "-"]);
    if (out_width, out_height) != (width, height) {
        command.args(["-vf", &format!("scale={out_width}:{out_height}")]);
    }
    command
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "28"])
        .args([
            "-g",
            &gop_frames.max(1).to_string(),
            "-movflags",
            "+faststart",
        ])
        .arg(out)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg (brew install ffmpeg): {e}"))
}

/// `Vehicle::Status::departing`. Filming both starts and ends here, which is
/// what makes the clip loop: see `film`.
const DEPARTING: u8 = 3;

/// Ticks until the lead train pulls out of the station. Gives up after two
/// minutes: a valleyed train never departs, and that footage still matters.
fn wait_for_departure(ride_id: u16, every: u32) -> u32 {
    const MAX_WAIT_TICKS: u32 = TICKS_PER_SECOND * 120;
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

/// How many frames a clip of `seconds` runs to at `every` ticks per frame.
pub fn frames_for_seconds(seconds: u32, every: u32) -> usize {
    (seconds * TICKS_PER_SECOND / every.max(1)) as usize
}

/// The cap for a clip, from the game's measured lap plus a tail.
pub fn seconds_for_lap(ride_time: i32, cap: u32) -> u32 {
    if ride_time <= 0 {
        return cap;
    }
    u32::try_from(ride_time + TAIL_SECONDS)
        .unwrap_or(cap)
        .min(cap)
}

/// Films `frames` frames of the park into `out`, one every `every` ticks.
///
/// Waits for `ride_id`'s train to leave the station first; None rolls at once.
/// `camera` frames the shot; None fits it to the track that is standing. The
/// hero's evolution passes its own, because a clip that cuts to this one has to
/// be shot from the same place or the coaster jumps at the cut.
pub fn film(
    out: &str,
    frames: usize,
    every: u32,
    zoom: i32,
    ride_id: Option<u16>,
    camera: Option<host::TrackBounds>,
) -> Result<Filmed, String> {
    let every = every.max(1);
    let fps = f64::from(TICKS_PER_SECOND) / f64::from(every);

    // One camera for every frame: a still park means consecutive frames differ
    // only where the train is, which is what makes the encode small. Resolved
    // once rather than per frame, because framing the track rescans the map.
    let bounds = camera
        .or_else(host::track_bounds)
        .ok_or("no track to film")?;
    let (width, height) = host::capture_size(zoom, 0, Some(&bounds)).ok_or("no track to film")?;

    let waited = ride_id.map_or(0, |ride| wait_for_departure(ride, every));

    // A poster from the same camera; the round's park.png frames the whole map.
    let poster = poster_path(out);
    let poster = host::capture_framed(&poster, zoom, 0, Some(&bounds), false).then_some(poster);

    let mut child = spawn_encoder(out, width, height, fps, frames)?;
    let mut sink = child.stdin.take().ok_or("ffmpeg stdin")?;

    let mut buf = vec![0u8; width as usize * height as usize * 4];
    let mut written = 0usize;
    // One whole station-to-station cycle, so the last frame runs into the first
    // and the clip loops. `frames` bounds a train that never comes back round.
    let mut left_the_station = false;
    let mut looped = false;
    for _ in 0..frames {
        host::run_ticks(every);
        let n = host::capture_frame(zoom, 0, Some(&bounds), &mut buf);
        if n == 0 {
            break;
        }
        if std::io::Write::write_all(&mut sink, &buf[..n]).is_err() {
            break;
        }
        written += 1;
        if let Some(ride) = ride_id {
            match host::vehicle_status(ride) {
                Some(DEPARTING) if left_the_station => {
                    looped = true;
                    break;
                }
                Some(DEPARTING) => {}
                Some(_) => left_the_station = true,
                None => {}
            }
        }
    }
    drop(sink);
    let status = child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg failed: {status}"));
    }
    let meta = serde_json::json!({
        "looped": looped,
        "frames": written,
        "fps": fps,
        "seconds": written as f64 / fps,
    });
    if let Ok(text) = serde_json::to_string_pretty(&meta) {
        let _ = std::fs::write(meta_path(out), text);
    }

    Ok(Filmed {
        looped,
        frames: written,
        width,
        height,
        fps,
        ticks_waited_for_station: waited,
        poster,
    })
}

/// Films the park's ride, bounded by `max_seconds`. Used by `eval --replay` and
/// by the evolution, which hands over its own camera.
pub fn film_ride(
    out: &str,
    ride_id: u16,
    max_seconds: u32,
    zoom: i32,
    camera: Option<host::TrackBounds>,
) -> Result<Filmed, String> {
    let ride_time = host::ride_detail(ride_id).map_or(0, |d| d.ride_time);
    let seconds = seconds_for_lap(ride_time, max_seconds);
    film(
        out,
        frames_for_seconds(seconds, DEFAULT_EVERY),
        DEFAULT_EVERY,
        zoom,
        Some(ride_id),
        camera,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_length_follows_the_measured_lap() {
        assert_eq!(seconds_for_lap(56, 90), 59);
        assert_eq!(seconds_for_lap(0, 90), 90);
        assert_eq!(seconds_for_lap(9999, 90), 90);
    }

    #[test]
    fn poster_sits_beside_the_video() {
        assert_eq!(poster_path("round_6/replay.mp4"), "round_6/replay.png");
        assert_eq!(poster_path("clip"), "clip.png");
        assert_eq!(meta_path("round_6/replay.mp4"), "round_6/replay.json");
    }

    #[test]
    fn frame_count_is_the_clip_at_twenty_fps() {
        assert_eq!(frames_for_seconds(59, 2), 1180);
        // every=0 clamps to one tick per frame rather than dividing by zero.
        assert_eq!(frames_for_seconds(1, 0), 40);
    }

    #[test]
    fn oversized_frames_are_scaled_into_what_decoders_accept() {
        assert_eq!(encoded_dimensions(2560, 1600), (2560, 1600));
        assert_eq!(encoded_dimensions(1984, 1424), (1984, 1424));
        assert_eq!(encoded_dimensions(2944, 2096), (2560, 1822));

        // The two published clips that froze: an evolution's union camera.
        let (w, h) = encoded_dimensions(4288, 2784);
        assert_eq!((w, h), (2560, 1662));
        let (w, h) = encoded_dimensions(4960, 3840);
        assert_eq!((w, h), (2560, 1982));

        for (sw, sh) in [(4288, 2784), (4960, 3840), (9728, 5008), (1000, 8000)] {
            let (w, h) = encoded_dimensions(sw, sh);
            assert!(w.max(h) <= MAX_ENCODED_DIMENSION, "{sw}x{sh} -> {w}x{h}");
            assert_eq!((w % 2, h % 2), (0, 0), "yuv420p needs even dimensions");
        }
    }
}
