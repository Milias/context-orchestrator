//! Tools panel widget: displays all available tools from the registry.
//!
//! Shows a two-column table (name + description), sourced from the static
//! tool registry. Used by the overview tab in the right column.

use crate::tool_executor::tool_registry;
use crate::tui::widgets::tool_status::truncate;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Compute the tools panel height: tool count + 2 borders.
pub fn tools_panel_height() -> u16 {
    let count = tool_registry().len();
    // Cast safety: tool count is small (<20).
    #[allow(clippy::cast_possible_truncation)] // Justified: tool count is small (<20).
    let h = (count as u16).saturating_add(2);
    h
}

/// Render the tools panel as a name + description table.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().title("Tools").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 {
        return;
    }

    let registry = tool_registry();
    if registry.is_empty() {
        let empty = Paragraph::new(Span::styled(
            "(no tools)",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(empty, inner);
        return;
    }

    // Name column width: longest tool name + 1 space separator.
    let name_width = registry
        .iter()
        .map(|e| e.name.as_str().len())
        .max()
        .unwrap_or(0);
    let sep = 1;
    let desc_budget = (inner.width as usize).saturating_sub(name_width + sep);
    let max_rows = inner.height as usize;

    let dim = Style::default().fg(Color::DarkGray);
    let lines: Vec<Line<'_>> = registry
        .iter()
        .take(max_rows)
        .map(|entry| {
            let name = entry.name.as_str();
            let padded_name = format!("{name:<name_width$}");
            let desc = truncate(entry.description, desc_budget);
            Line::from(vec![
                Span::styled(padded_name, Style::default().fg(Color::Magenta)),
                Span::raw(" "),
                Span::styled(desc, dim),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}
