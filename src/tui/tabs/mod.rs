//! Top-level tab views for the monitoring-centric TUI layout.
//!
//! Each tab fills the left content area and has its own rendering logic.
//! The tab bar at the top shows all tabs with the active one highlighted.

use crate::tui::state::TopTab;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render a placeholder for tabs not yet implemented.
pub fn render_placeholder(frame: &mut Frame, area: Rect, tab: TopTab) {
    let text = format!("{} tab (coming soon)", tab.label());
    let style = Style::default().fg(Color::DarkGray);
    let p = Paragraph::new(text)
        .style(style)
        .alignment(Alignment::Center);
    frame.render_widget(p, area);
}
