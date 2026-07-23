//! The stock track design library (the .TD6 files shipped with RCT2), as
//! piece sequences. Fetched once from the C++ host, which scans the same
//! directories as the game's own track design list and imports each file.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::host;
use crate::pieces;

#[derive(Debug, Clone, Deserialize)]
pub struct LibraryDesign {
    pub name: String,
    pub ride_type: u16,
    /// Raw TrackElemType values, in track order.
    pub pieces: Vec<u16>,
}

/// Loads every importable design. Errors mean the host had no library to give
/// (no game context, or asset path missing), not a malformed design.
pub fn load() -> Result<Vec<LibraryDesign>, String> {
    let json = host::track_library_json().ok_or("track design library unavailable")?;
    serde_json::from_str(&json).map_err(|e| format!("track design library parse: {e}"))
}

/// A design's pieces in track-program form: catalog names where known, raw
/// track types where not. Directly submittable via place_pieces or a program.
pub fn program_pieces(design: &LibraryDesign) -> Vec<Value> {
    design
        .pieces
        .iter()
        .map(|&id| match pieces::name_of(id) {
            Some(name) => json!(name),
            None => json!(id),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::library::*;

    #[test]
    fn design_json_round_trips() {
        let json = r#"[{"name": "Big Twister", "ride_type": 52, "pieces": [2, 3, 1, 4]}]"#;
        let designs: Vec<LibraryDesign> = serde_json::from_str(json).expect("parse");
        assert_eq!(designs.len(), 1);
        assert_eq!(designs[0].name, "Big Twister");
        assert_eq!(designs[0].pieces, vec![2, 3, 1, 4]);
    }

    #[test]
    fn load_without_host_is_an_error() {
        // The test stub host returns null for the library.
        assert!(load().is_err());
    }
}
