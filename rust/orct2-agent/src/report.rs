//! End-of-eval JSON report: the machine-readable artifact the driver script
//! feeds back to competing agents.

use serde::Serialize;

use crate::host;
use crate::program::ProgramOutcome;

#[derive(Debug, Serialize)]
pub struct EvalReport {
    /// Present when the eval ran a track program.
    pub program: Option<ProgramOutcome>,
    pub rides: Vec<RideReport>,
}

#[derive(Debug, Serialize)]
pub struct RideReport {
    pub id: u16,
    pub ride_type: u16,
    pub status: &'static str,
    pub tested: bool,
    pub crashed: bool,
    /// Ratings as displayed in game (e.g. 6.42), None until calculated.
    pub excitement: Option<f32>,
    pub intensity: Option<f32>,
    pub nausea: Option<f32>,
    pub max_speed: i32,
    pub average_speed: i32,
    pub ride_length: i32,
    pub max_positive_g: f32,
    pub max_negative_g: f32,
    pub max_lateral_g: f32,
    pub total_air_time: u16,
    pub num_drops: u8,
    pub highest_drop: u8,
    pub num_inversions: u8,
}

pub fn status_name(status: u8) -> &'static str {
    match status {
        0 => "closed",
        1 => "open",
        2 => "testing",
        3 => "simulating",
        _ => "unknown",
    }
}

fn fixed2dp(raw: i16) -> f32 {
    f32::from(raw) / 100.0
}

/// Collects every non-stall ride's detail into a report. `only_ride` narrows
/// to the program's ride so pre-existing park rides don't drown the signal.
pub fn build(program: Option<ProgramOutcome>, only_ride: Option<u16>) -> EvalReport {
    let mut rides = Vec::new();
    for index in 0..host::ride_count() {
        let Some(stats) = host::ride_stats(index) else {
            continue;
        };
        if stats.is_stall {
            continue;
        }
        if let Some(only) = only_ride {
            if stats.id != only {
                continue;
            }
        }
        let Some(detail) = host::ride_detail(stats.id) else {
            continue;
        };
        let rated = stats.has_ratings;
        rides.push(RideReport {
            id: stats.id,
            ride_type: stats.ride_type,
            status: status_name(stats.status),
            tested: detail.tested,
            crashed: detail.crashed,
            excitement: rated.then(|| fixed2dp(detail.excitement)),
            intensity: rated.then(|| fixed2dp(detail.intensity)),
            nausea: rated.then(|| fixed2dp(detail.nausea)),
            max_speed: detail.max_speed,
            average_speed: detail.average_speed,
            ride_length: detail.ride_length,
            max_positive_g: fixed2dp(detail.max_positive_g),
            max_negative_g: fixed2dp(detail.max_negative_g),
            max_lateral_g: fixed2dp(detail.max_lateral_g),
            total_air_time: detail.total_air_time,
            num_drops: detail.num_drops,
            highest_drop: detail.highest_drop,
            num_inversions: detail.num_inversions,
        });
    }
    EvalReport { program, rides }
}

#[cfg(test)]
mod tests {
    use crate::report::*;

    #[test]
    fn fixed2dp_matches_game_display() {
        assert!((fixed2dp(642) - 6.42).abs() < f32::EPSILON);
        assert!((fixed2dp(0) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn status_names_cover_game_enum() {
        assert_eq!(status_name(0), "closed");
        assert_eq!(status_name(2), "testing");
        assert_eq!(status_name(9), "unknown");
    }
}
