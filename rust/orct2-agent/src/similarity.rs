//! Plagiarism check: how close is an agent's track to a stock library design?
//!
//! Tracks are token sequences (TrackElemType values), so this is plain
//! sequence similarity, no geometry needed. Two measures, max-combined:
//!
//! - edit similarity: 1 - levenshtein / max(len). Catches whole-track copies
//!   with light editing.
//! - common substring share: longest common contiguous run over the program
//!   length. Catches a big lifted chunk with an original ending stapled on,
//!   which edit distance alone dilutes.
//!
//! Every library design is also compared mirrored (left/right pieces swapped)
//! so flipping a stock coaster does not dodge the check.

use serde::Serialize;

use crate::library::LibraryDesign;

/// Nearest library design and how close the program got to it.
#[derive(Debug, Clone, Serialize)]
pub struct SimilarityReport {
    pub nearest_design: String,
    pub nearest_ride_type: u16,
    /// max(edit_similarity, common_substring_share), in [0, 1].
    pub similarity: f64,
    pub edit_similarity: f64,
    pub common_substring_share: f64,
    /// True when the nearest match was the mirrored variant of the design.
    pub mirrored: bool,
}

fn levenshtein(a: &[u16], b: &[u16]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitute = prev[j] + usize::from(ca != cb);
            cur[j + 1] = substitute.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn edit_similarity(a: &[u16], b: &[u16]) -> f64 {
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - levenshtein(a, b) as f64 / max_len as f64
}

/// Longest common contiguous run, as a share of the program's length.
fn common_substring_share(program: &[u16], other: &[u16]) -> f64 {
    if program.is_empty() || other.is_empty() {
        return 0.0;
    }
    let mut prev = vec![0usize; other.len() + 1];
    let mut cur = vec![0usize; other.len() + 1];
    let mut longest = 0usize;
    for &cp in program {
        for (j, &co) in other.iter().enumerate() {
            cur[j + 1] = if cp == co { prev[j] + 1 } else { 0 };
            longest = longest.max(cur[j + 1]);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    longest as f64 / program.len() as f64
}

/// Scores `program` against every library design (and its mirror) and returns
/// the single closest match. None for an empty program or empty library.
pub fn best_match(
    program: &[u16],
    library: &[LibraryDesign],
    mirror: impl Fn(u16) -> u16,
) -> Option<SimilarityReport> {
    if program.is_empty() {
        return None;
    }
    let mut best: Option<SimilarityReport> = None;
    for design in library {
        if design.pieces.is_empty() {
            continue;
        }
        let mirrored: Vec<u16> = design.pieces.iter().map(|&p| mirror(p)).collect();
        let variants: [(&[u16], bool); 2] = [(&design.pieces, false), (&mirrored, true)];
        for (pieces, is_mirrored) in variants {
            // Symmetric designs mirror to themselves; skip the duplicate pass.
            if is_mirrored && pieces == design.pieces.as_slice() {
                continue;
            }
            let edit = edit_similarity(program, pieces);
            let share = common_substring_share(program, pieces);
            let combined = edit.max(share);
            if best.as_ref().is_none_or(|b| combined > b.similarity) {
                best = Some(SimilarityReport {
                    nearest_design: design.name.clone(),
                    nearest_ride_type: design.ride_type,
                    similarity: combined,
                    edit_similarity: edit,
                    common_substring_share: share,
                    mirrored: is_mirrored,
                });
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use crate::similarity::*;

    fn design(name: &str, pieces: &[u16]) -> LibraryDesign {
        LibraryDesign {
            name: name.into(),
            ride_type: 52,
            pieces: pieces.to_vec(),
        }
    }

    /// Toy mirror: swaps 16<->17 (left/right turn), leaves the rest alone.
    fn toy_mirror(p: u16) -> u16 {
        match p {
            16 => 17,
            17 => 16,
            other => other,
        }
    }

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein(&[], &[]), 0);
        assert_eq!(levenshtein(&[1, 2, 3], &[1, 2, 3]), 0);
        assert_eq!(levenshtein(&[1, 2, 3], &[]), 3);
        assert_eq!(levenshtein(&[1, 2, 3], &[1, 9, 3]), 1);
        assert_eq!(levenshtein(&[1, 2, 3], &[2, 3]), 1);
    }

    #[test]
    fn edit_similarity_range() {
        assert!((edit_similarity(&[1, 2, 3], &[1, 2, 3]) - 1.0).abs() < f64::EPSILON);
        assert!((edit_similarity(&[1, 2], &[3, 4]) - 0.0).abs() < f64::EPSILON);
        let half = edit_similarity(&[1, 2, 3, 4], &[1, 2, 9, 9]);
        assert!((half - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn substring_share_catches_lifted_chunk() {
        // Program of 10 pieces, 6 of which are a contiguous lift from `other`.
        let program = [2u16, 1, 40, 41, 42, 43, 44, 45, 7, 8];
        let other = [40u16, 41, 42, 43, 44, 45, 99, 98];
        assert!((common_substring_share(&program, &other) - 0.6).abs() < f64::EPSILON);
    }

    #[test]
    fn exact_copy_scores_one() {
        let lib = vec![design("stock", &[2, 3, 1, 4, 16, 10, 17])];
        let report = best_match(&[2, 3, 1, 4, 16, 10, 17], &lib, toy_mirror).expect("match");
        assert_eq!(report.nearest_design, "stock");
        assert!((report.similarity - 1.0).abs() < f64::EPSILON);
        assert!(!report.mirrored);
    }

    #[test]
    fn mirrored_copy_scores_one_and_is_flagged() {
        let lib = vec![design("stock", &[2, 3, 1, 16, 16, 10])];
        // The same track with turns flipped right.
        let report = best_match(&[2, 3, 1, 17, 17, 10], &lib, toy_mirror).expect("match");
        assert!((report.similarity - 1.0).abs() < f64::EPSILON);
        assert!(report.mirrored);
    }

    #[test]
    fn original_design_scores_low() {
        let lib = vec![design("stock", &[2, 3, 1, 4, 4, 9, 16, 10, 10, 15, 16, 0])];
        let original = [2u16, 3, 1, 6, 5, 5, 8, 17, 11, 14, 17, 38, 0];
        let report = best_match(&original, &lib, toy_mirror).expect("match");
        assert!(report.similarity < 0.5, "got {}", report.similarity);
    }

    #[test]
    fn empty_program_or_library_is_none() {
        assert!(best_match(&[], &[design("x", &[1])], toy_mirror).is_none());
        assert!(best_match(&[1, 2], &[], toy_mirror).is_none());
        assert!(best_match(&[1, 2], &[design("empty", &[])], toy_mirror).is_none());
    }

    #[test]
    fn picks_the_closest_of_several_designs() {
        let lib = vec![
            design("far", &[90, 91, 92, 93]),
            design("near", &[2, 3, 1, 4, 9, 0]),
        ];
        let report = best_match(&[2, 3, 1, 4, 9, 10], &lib, toy_mirror).expect("match");
        assert_eq!(report.nearest_design, "near");
    }
}
