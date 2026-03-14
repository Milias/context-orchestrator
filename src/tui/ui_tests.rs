//! Tests for status-bar formatting helpers.

use crate::tui::ui::format_token_count;

/// Catches: wrong threshold for k/M suffixes, off-by-one at boundaries.
#[test]
fn format_boundaries() {
    assert_eq!(format_token_count(0), "0");
    assert_eq!(format_token_count(999), "999");
    assert_eq!(format_token_count(1_000), "1.0k");
    assert_eq!(format_token_count(1_500), "1.5k");
    assert_eq!(format_token_count(999_999), "1000.0k");
    assert_eq!(format_token_count(1_000_000), "1.0M");
    assert_eq!(format_token_count(2_500_000), "2.5M");
}
