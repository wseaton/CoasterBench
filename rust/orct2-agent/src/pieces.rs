//! Track piece catalog: snake_case names -> game TrackElemType values.
//!
//! Values mirror src/openrct2/ride/ted/TrackElemType.h. This is the vocabulary
//! exposed to LLM agents in track programs, so names favour readability over
//! matching the C++ identifiers exactly. Raw numeric ids are also accepted in
//! programs for anything not listed here.

/// The coaster-relevant subset of TrackElemType, by agent-facing name.
pub const CATALOG: &[(&str, u16)] = &[
    ("flat", 0),
    ("end_station", 1),
    ("begin_station", 2),
    ("middle_station", 3),
    ("up_25", 4),
    ("up_60", 5),
    ("flat_to_up_25", 6),
    ("up_25_to_up_60", 7),
    ("up_60_to_up_25", 8),
    ("up_25_to_flat", 9),
    ("down_25", 10),
    ("down_60", 11),
    ("flat_to_down_25", 12),
    ("down_25_to_down_60", 13),
    ("down_60_to_down_25", 14),
    ("down_25_to_flat", 15),
    ("left_turn_5", 16),
    ("right_turn_5", 17),
    ("flat_to_left_bank", 18),
    ("flat_to_right_bank", 19),
    ("left_bank_to_flat", 20),
    ("right_bank_to_flat", 21),
    ("banked_left_turn_5", 22),
    ("banked_right_turn_5", 23),
    ("left_bank_to_up_25", 24),
    ("right_bank_to_up_25", 25),
    ("up_25_to_left_bank", 26),
    ("up_25_to_right_bank", 27),
    ("left_bank_to_down_25", 28),
    ("right_bank_to_down_25", 29),
    ("down_25_to_left_bank", 30),
    ("down_25_to_right_bank", 31),
    ("left_bank", 32),
    ("right_bank", 33),
    ("left_turn_5_up_25", 34),
    ("right_turn_5_up_25", 35),
    ("left_turn_5_down_25", 36),
    ("right_turn_5_down_25", 37),
    ("s_bend_left", 38),
    ("s_bend_right", 39),
    ("left_vertical_loop", 40),
    ("right_vertical_loop", 41),
    ("left_turn_3", 42),
    ("right_turn_3", 43),
    ("banked_left_turn_3", 44),
    ("banked_right_turn_3", 45),
    ("left_turn_3_up_25", 46),
    ("right_turn_3_up_25", 47),
    ("left_turn_3_down_25", 48),
    ("right_turn_3_down_25", 49),
    ("left_turn_1", 50),
    ("right_turn_1", 51),
    ("half_loop_up", 56),
    ("half_loop_down", 57),
    ("left_corkscrew_up", 58),
    ("right_corkscrew_up", 59),
    ("left_corkscrew_down", 60),
    ("right_corkscrew_down", 61),
    ("flat_to_up_60", 62),
    ("up_60_to_flat", 63),
    ("flat_to_down_60", 64),
    ("down_60_to_flat", 65),
    ("left_helix_up_small", 87),
    ("right_helix_up_small", 88),
    ("left_helix_down_small", 89),
    ("right_helix_down_small", 90),
    ("left_helix_up_large", 91),
    ("right_helix_up_large", 92),
    ("left_helix_down_large", 93),
    ("right_helix_down_large", 94),
    ("brakes", 99),
    ("booster", 100),
];

pub fn lookup(name: &str) -> Option<u16> {
    CATALOG.iter().find(|(n, _)| *n == name).map(|&(_, id)| id)
}

pub fn name_of(id: u16) -> Option<&'static str> {
    CATALOG.iter().find(|(_, i)| *i == id).map(|&(n, _)| n)
}

#[cfg(test)]
mod tests {
    use crate::pieces::*;

    #[test]
    fn lookup_round_trips() {
        assert_eq!(lookup("begin_station"), Some(2));
        assert_eq!(lookup("left_vertical_loop"), Some(40));
        assert_eq!(lookup("nonexistent"), None);
        assert_eq!(name_of(40), Some("left_vertical_loop"));
    }

    #[test]
    fn catalog_has_no_duplicate_names_or_ids() {
        use std::collections::HashSet;
        let names: HashSet<_> = CATALOG.iter().map(|(n, _)| n).collect();
        let ids: HashSet<_> = CATALOG.iter().map(|(_, i)| i).collect();
        assert_eq!(names.len(), CATALOG.len());
        assert_eq!(ids.len(), CATALOG.len());
    }
}
