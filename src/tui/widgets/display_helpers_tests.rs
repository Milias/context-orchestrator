use ratatui::prelude::*;

use super::display_helpers::{apply_reveal_fade, dim_style};

/// Catches panic on UTF-8 boundary when span-splitting multi-byte characters.
/// Emoji and CJK chars are multi-byte; the split logic must use `char_indices`.
#[test]
fn apply_reveal_fade_handles_multibyte_chars() {
    let mut text = Text::from(vec![Line::from(vec![Span::raw("Hello 🚀🌍✨")])]);

    // Should not panic — fade width spans across multi-byte emoji
    apply_reveal_fade(&mut text, 8);

    // The spans should have been split; verify total char content is preserved
    let total: String = text.lines[0]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(total, "Hello 🚀🌍✨");
}

/// Catches span-count regression: fading a span must produce at least as many
/// spans (the original may be split in two at the fade boundary).
#[test]
fn apply_reveal_fade_splits_span_at_boundary() {
    let mut text = Text::from(vec![Line::from(vec![Span::styled(
        "abcdefghijklmnop".to_string(),
        Style::default().fg(Color::Rgb(200, 200, 200)),
    )])]);

    apply_reveal_fade(&mut text, 8);

    // Original 16-char span should be split: 8 normal + 8 dimmed (or sub-groups)
    assert!(
        text.lines[0].spans.len() >= 2,
        "should split into at least 2 spans, got {}",
        text.lines[0].spans.len()
    );
}

/// Catches colour computation overflow or incorrect dimming arithmetic.
/// `dim_style` with factor 0.5 on Rgb(200, 100, 50) must yield Rgb(100, 50, 25).
#[test]
fn dim_style_preserves_hue_and_reduces_brightness() {
    let style = Style::default().fg(Color::Rgb(200, 100, 50));
    let dimmed = dim_style(style, 0.5);
    assert_eq!(dimmed.fg, Some(Color::Rgb(100, 50, 25)));
}

/// Catches default foreground not being dimmed (would render invisible text
/// as full-bright white instead of faded).
#[test]
fn dim_style_handles_default_foreground() {
    let style = Style::default(); // no fg set
    let dimmed = dim_style(style, 0.5);
    assert_eq!(dimmed.fg, Some(Color::Rgb(120, 120, 120)));
}

/// Catches inverted gradient where the frontier is brightest instead of dimmest.
/// The last char (reveal frontier) must be the dimmest; 8th-from-last the brightest.
#[test]
fn apply_reveal_fade_frontier_is_dimmest() {
    // 8 single-char spans so each char gets its own factor
    let spans: Vec<Span<'static>> = "abcdefgh"
        .chars()
        .map(|c| {
            Span::styled(
                c.to_string(),
                Style::default().fg(Color::Rgb(200, 200, 200)),
            )
        })
        .collect();
    let mut text = Text::from(vec![Line::from(spans)]);

    apply_reveal_fade(&mut text, 8);

    // Extract the green channel from each span's fg to check brightness ordering
    let brightnesses: Vec<u8> = text.lines[0]
        .spans
        .iter()
        .filter_map(|s| match s.style.fg {
            Some(Color::Rgb(_, g, _)) => Some(g),
            _ => None,
        })
        .collect();

    assert_eq!(brightnesses.len(), 8, "should have 8 styled spans");
    // First span (a) = brightest (furthest from frontier)
    // Last span (h) = dimmest (frontier)
    assert!(
        brightnesses[0] > brightnesses[7],
        "first span should be brighter ({}) than last ({})",
        brightnesses[0],
        brightnesses[7]
    );
    // Monotonically non-increasing from first to last
    for i in 0..7 {
        assert!(
            brightnesses[i] >= brightnesses[i + 1],
            "brightness[{i}]={} should be >= brightness[{}]={}",
            brightnesses[i],
            i + 1,
            brightnesses[i + 1]
        );
    }
}
