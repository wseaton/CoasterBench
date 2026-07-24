//! Round prompt for the sandboxed Claude Code agent. Interactive edition of
//! the driver.py system prompt: the MCP tools replace the memorised geometry
//! tables (valid_next_pieces and piece_geometry are ground truth).

use crate::Modalities;

pub fn ride_name(ride_type: u16) -> &'static str {
    match ride_type {
        51 => "steel twister roller coaster",
        52 => "wooden roller coaster",
        _ => "roller coaster",
    }
}

pub fn round_prompt(
    ride_type: u16,
    round: u32,
    rounds: u32,
    previous_feedback: Option<&str>,
    modalities: Modalities,
) -> String {
    let name = ride_name(ride_type);
    // Models that can't take images get an MCP endpoint with screenshot
    // stripped, so the tool list must not advertise it either.
    let screenshot = if modalities.contains(Modalities::IMAGE) {
        "\n- screenshot(): park image."
    } else {
        ""
    };
    let inversions = if ride_type == 51 {
        "Inversions are ALLOWED and rewarded (vertical loops, corkscrews, half loops)."
    } else {
        "This type does NOT support inversions."
    };
    let feedback = match previous_feedback {
        Some(report) => format!(
            "\n## Previous round result\nYour previous attempt's eval report (learn from it):\n{report}\n"
        ),
        None => String::new(),
    };
    format!(
        r#"You are competing to design the best RollerCoaster Tycoon 2 {name} (ride_type {ride_type}).
This is round {round} of {rounds}. You build interactively through the "coaster" MCP tools:

- new_ride(ride_type, x, y, dir): start building; the cursor starts at that tile. Directions: 0 faces -x, 1 faces +y, 2 faces +x, 3 faces -y.
- place_piece(piece, chain?): place one piece at the cursor. Returns the new cursor and circuit_closed.
- place_pieces(pieces): batch placement; stops at the first rejection.
- valid_next_pieces(): which catalog pieces the game will accept at the current cursor. Ground truth - use it whenever unsure.
- piece_geometry(dir): exact cursor delta (tiles, z, direction, bank, slope) for every piece from a given direction. Use this to PLAN closure.
- undo_piece(): remove the last piece.
- get_state(): cursor, start, pieces placed, circuit_closed.
- finish_and_test(ticks?): places entrance/exit, runs a real test train, returns the full eval report with excitement/intensity/nausea.{screenshot}
- demolish(): tear down and start over (then new_ride again).

## Rules
- Build a CLOSED CIRCUIT: get_state must show circuit_closed before finish_and_test will pass.
- Start with begin_station, middle_station, end_station on flat ground (3-7 station pieces).
- Chain lift ({{"chain": true}}) only works on 25-degree slopes. Trains start slow; climb first, then coast.
- Intensity above ~10 tanks excitement; keep it under 10. Crashes disqualify.
- {inversions}
- Your track is compared to the stock design library; similarity above 0.5 scales your score toward zero. Design something original.
- Map: flat grass around tile (60, 60); a lake sits roughly at tiles (68-85, 55-75) - do NOT build into it. Stay within tiles 20-120.

## How to work
1. Plan a layout, then new_ride and build with place_pieces in chunks.
2. On rejection, read the error, use valid_next_pieces/piece_geometry, fix, continue.
3. Close the circuit (watch cursor vs start in get_state; plan the return leg with piece_geometry).
4. finish_and_test, read the report. If time permits, demolish and rebuild better.
5. End your session with a one-line summary of your final coaster and its excitement.

Maximise excitement. The last finish_and_test result of this session is your round score.
{feedback}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twister_prompt_mentions_inversions_allowed() {
        let p = round_prompt(51, 1, 6, None, Modalities::TEXT | Modalities::IMAGE);
        assert!(p.contains("ALLOWED"));
        assert!(p.contains("ride_type 51"));
        assert!(p.contains("round 1 of 6"));
    }

    #[test]
    fn wooden_prompt_forbids_inversions() {
        assert!(
            round_prompt(52, 2, 4, None, Modalities::TEXT | Modalities::IMAGE)
                .contains("NOT support")
        );
    }

    #[test]
    fn feedback_is_included_when_present() {
        let p = round_prompt(
            51,
            2,
            6,
            Some("{\"excitement\": 5.0}"),
            Modalities::TEXT | Modalities::IMAGE,
        );
        assert!(p.contains("Previous round result"));
        assert!(p.contains("excitement"));
    }

    #[test]
    fn text_only_prompt_omits_the_screenshot_tool() {
        let p = round_prompt(52, 1, 6, None, Modalities::TEXT);
        assert!(!p.contains("screenshot"));
        assert!(p.contains("demolish()"), "other tools still listed");
    }
}
