//! Tools panel widget: displays all available tools from the registry.
//!
//! Shows tool names in a compact 2-column grid, sourced from the static
//! tool registry. Used by the overview tab in the right column.

use crate::tool_executor::tool_registry;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Number of columns in the tool name grid.
const GRID_COLS: usize = 2;

/// Compute the tools panel height: `ceil(tool_count / GRID_COLS)` + 2 borders.
pub fn tools_panel_height() -> u16 {
    let count = tool_registry().len();
    let rows = count.div_ceil(GRID_COLS);
    // Cast safety: tool count is small (<20).
    #[allow(clippy::cast_possible_truncation)] // Justified: tool count is small (<20).
    let h = (rows as u16).saturating_add(2);
    h
}

/// Render the tools panel showing all registered tool names in a 2-column grid.
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

    let col_width = (inner.width as usize) / GRID_COLS;
    let max_rows = inner.height as usize;
    let mut lines: Vec<Line<'_>> = Vec::new();

    let names: Vec<&str> = registry.iter().map(|entry| entry.name.as_str()).collect();

    for row_idx in 0..max_rows {
        let mut spans: Vec<Span<'_>> = Vec::new();

        for col in 0..GRID_COLS {
            let tool_idx = row_idx * GRID_COLS + col;
            if tool_idx < names.len() {
                let name = names[tool_idx];
                let display: String = if name.len() > col_width {
                    name[..col_width].to_string()
                } else {
                    format!("{name:<col_width$}")
                };
                spans.push(Span::styled(display, Style::default().fg(Color::Magenta)));
            }
        }

        if spans.is_empty() {
            break;
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}
