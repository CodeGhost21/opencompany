//! Char-boundary-safe slicing shared by every
//! [`ContextStore`](crate::ports::ContextStore) backend.
//!
//! The ranged `peek` and the search-snippet window both compute **byte**
//! offsets — a caller-supplied range, or ±24 bytes around a match — and a byte
//! offset lands mid-codepoint on any non-ASCII body. A raw `body[start..end]`
//! there panics, and `memory_recall` routes agent queries straight into
//! `search`, so the panic is reachable from ordinary chunk content. These
//! helpers widen outward to the nearest boundary instead: slightly more text
//! than asked, never a failed read.

use std::ops::Range;

/// Clamps `range` to the string's length and to char boundaries.
///
/// Widening is outward on both ends (floor the start, ceil the end), so the
/// requested bytes are always contained in the answer. An inverted or empty
/// range yields the empty string rather than a panic — including a zero-length
/// range landing mid-codepoint, which the widening alone would have grown into
/// a whole character (asking for no bytes must never answer with one).
pub(crate) fn slice_on_char_boundaries(body: &str, range: Range<usize>) -> String {
    if range.start >= range.end {
        return String::new();
    }
    let start = floor_boundary(body, range.start.min(body.len()));
    let end = ceil_boundary(body, range.end.min(body.len()));
    if start >= end {
        return String::new();
    }
    body[start..end].to_string()
}

/// The nearest char boundary at or below `at`.
pub(crate) fn floor_boundary(s: &str, mut at: usize) -> usize {
    while at > 0 && !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The nearest char boundary at or above `at` (callers clamp to `s.len()`).
pub(crate) fn ceil_boundary(s: &str, mut at: usize) -> usize {
    while at < s.len() && !s.is_char_boundary(at) {
        at += 1;
    }
    at
}

#[cfg(test)]
mod test {
    use super::*;

    // The inverted range below is the point of the last assertion: `peek` takes
    // its range from a caller, and a caller that computed one backwards must get
    // an empty read rather than a panic in a tenant container.
    #[expect(
        clippy::reversed_empty_ranges,
        reason = "the reversed range is the input under test"
    )]
    #[test]
    fn ranges_widen_to_char_boundaries_instead_of_panicking() {
        // "é" is two bytes; a range that splits it would panic on a raw slice.
        let body = "aébc";
        assert_eq!(slice_on_char_boundaries(body, 0..2), "aé");
        assert_eq!(slice_on_char_boundaries(body, 1..2), "é");
        // Past the end clamps rather than panicking.
        assert_eq!(slice_on_char_boundaries(body, 0..999), body);
        // An inverted range yields nothing, not a panic.
        assert_eq!(slice_on_char_boundaries(body, 3..1), "");
        // A zero-length range asks for no bytes and gets none, even where the
        // outward widening would otherwise have grown it into a whole "é".
        assert_eq!(slice_on_char_boundaries(body, 2..2), "");
        assert_eq!(slice_on_char_boundaries(body, 0..0), "");
        assert_eq!(slice_on_char_boundaries(body, 99..99), "");
    }

    #[test]
    fn boundaries_floor_down_and_ceil_up() {
        let body = "aébc";
        // Byte 2 is mid-"é": floor lands before it, ceil after it.
        assert_eq!(floor_boundary(body, 2), 1);
        assert_eq!(ceil_boundary(body, 2), 3);
        // A boundary stays where it is in both directions.
        assert_eq!(floor_boundary(body, 3), 3);
        assert_eq!(ceil_boundary(body, 3), 3);
    }
}
