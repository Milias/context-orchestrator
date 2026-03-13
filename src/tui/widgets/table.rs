use ratatui::prelude::*;

use super::markdown::parse_inline_spans;

pub(super) struct TableBuffer {
    header: Option<Vec<String>>,
    rows: Vec<Vec<String>>,
    num_cols: usize,
}

impl TableBuffer {
    pub(super) fn new() -> Self {
        Self {
            header: None,
            rows: Vec::new(),
            num_cols: 0,
        }
    }

    pub(super) fn set_header(&mut self, cells: &[String]) {
        self.num_cols = self.num_cols.max(cells.len());
        self.header = Some(cells.to_vec());
    }

    pub(super) fn add_row(&mut self, cells: &[String]) {
        self.num_cols = self.num_cols.max(cells.len());
        self.rows.push(cells.to_vec());
    }

    pub(super) fn flush(self, lines: &mut Vec<Line<'static>>) {
        // Parse all cells into spans so we can compute visible widths.
        let parsed_header: Option<Vec<Vec<Span<'static>>>> = self
            .header
            .as_ref()
            .map(|h| h.iter().map(|c| parse_inline_spans(c)).collect());
        let parsed_rows: Vec<Vec<Vec<Span<'static>>>> = self
            .rows
            .iter()
            .map(|row| row.iter().map(|c| parse_inline_spans(c)).collect())
            .collect();

        let mut col_widths = vec![0usize; self.num_cols];
        if let Some(header) = &parsed_header {
            for (i, spans) in header.iter().enumerate() {
                col_widths[i] = col_widths[i].max(span_width(spans));
            }
        }
        for row in &parsed_rows {
            for (i, spans) in row.iter().enumerate() {
                if i < col_widths.len() {
                    col_widths[i] = col_widths[i].max(span_width(spans));
                }
            }
        }

        if let Some(header) = parsed_header {
            let base = Style::default().bold().fg(Color::Cyan);
            lines.push(build_table_line(&header, &col_widths, Some(base)));
            let total: usize =
                col_widths.iter().sum::<usize>() + col_widths.len().saturating_sub(1) * 3;
            lines.push(Line::styled(
                "\u{2500}".repeat(total.max(1)),
                Style::default().fg(Color::DarkGray),
            ));
        }
        for row in &parsed_rows {
            lines.push(build_table_line(row, &col_widths, None));
        }
    }
}

fn span_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.len()).sum()
}

fn build_table_line<'a>(
    cells: &[Vec<Span<'a>>],
    col_widths: &[usize],
    base_style: Option<Style>,
) -> Line<'a> {
    let mut row_spans: Vec<Span<'a>> = Vec::new();
    for (i, cell_spans) in cells.iter().enumerate() {
        if i > 0 {
            let sep = match base_style {
                Some(s) => Span::styled(" | ", s),
                None => Span::raw(" | "),
            };
            row_spans.push(sep);
        }
        let vis_w = span_width(cell_spans);
        for span in cell_spans {
            let styled = match base_style {
                Some(base) => Span::styled(span.content.clone(), base.patch(span.style)),
                None => span.clone(),
            };
            row_spans.push(styled);
        }
        let target = col_widths.get(i).copied().unwrap_or(0);
        if vis_w < target {
            let pad = " ".repeat(target - vis_w);
            let padded = match base_style {
                Some(s) => Span::styled(pad, s),
                None => Span::raw(pad),
            };
            row_spans.push(padded);
        }
    }
    Line::from(row_spans)
}

#[cfg(test)]
#[path = "table_tests.rs"]
mod tests;
