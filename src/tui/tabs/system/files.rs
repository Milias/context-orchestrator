//! Files section for the System tab.
//!
//! Collects all `Node::GitFile` nodes from the graph, groups them by
//! directory path, and renders as a tree with status badges.
//!
//! ```text
//! ├── src/
//! │   ├── tui/
//! │   │   └── mod.rs [Modified]
//! │   └── main.rs [Tracked]
//! └── Cargo.toml [Staged]
//! ```

use std::collections::BTreeMap;

use crate::graph::node::GitFileStatus;
use crate::graph::{ConversationGraph, Node};
use crate::tui::tabs::graph::tree_lines::TreePrefix;
use crate::tui::widgets::tool_status::truncate;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Maximum number of file lines before capping.
const MAX_FILE_LINES: u16 = 15;

/// Minimum section height when files exist (borders + 1 line).
const MIN_FILES_HEIGHT: u16 = 3;

/// Compute the files section height based on git file count.
///
/// Returns 2 (collapsed header) when no files exist, or borders + line
/// count (each file and directory contributes 1 line, capped).
pub fn files_section_height(graph: &ConversationGraph) -> u16 {
    let files = collect_git_files(graph);
    if files.is_empty() {
        return 2; // collapsed header
    }
    let tree = build_dir_tree(&files);
    let line_count = count_tree_lines(&tree);
    let n = u16::try_from(line_count).unwrap_or(u16::MAX);
    n.saturating_add(2)
        .clamp(MIN_FILES_HEIGHT, MAX_FILE_LINES + 2)
}

/// Render the Files section: `GitFile` nodes displayed as a directory tree.
///
/// Each file shows its status badge: `[Modified]` (yellow), `[Staged]`
/// (green), `[Untracked]` (red), `[Tracked]` (dim).
pub fn render_files(frame: &mut Frame, area: Rect, graph: &ConversationGraph) {
    let files = collect_git_files(graph);

    let title = if files.is_empty() {
        "Files (0)".to_string()
    } else {
        format!("Files ({})", files.len())
    };

    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width < 8 || files.is_empty() {
        return;
    }

    let tree = build_dir_tree(&files);
    let width = inner.width as usize;
    let max_lines = inner.height as usize;
    let mut lines: Vec<Line<'_>> = Vec::new();

    render_tree_level(&tree, &TreePrefix::new(), width, max_lines, &mut lines);

    lines.truncate(max_lines);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

// ── Data types ──────────────────────────────────────────────────────

/// A file entry extracted from a `GitFile` node.
struct FileEntry {
    /// Full path split into segments.
    segments: Vec<String>,
    /// Git status for badge display.
    status: GitFileStatus,
}

/// A node in the virtual directory tree.
/// Uses `BTreeMap` so entries are alphabetically sorted.
enum DirEntry {
    /// A directory containing children.
    Dir(BTreeMap<String, DirEntry>),
    /// A leaf file with its status.
    File(GitFileStatus),
}

// ── Collection and tree building ────────────────────────────────────

/// Collect all `GitFile` nodes and extract their path segments + status.
fn collect_git_files(graph: &ConversationGraph) -> Vec<FileEntry> {
    graph
        .nodes_by(|n| matches!(n, Node::GitFile { .. }))
        .into_iter()
        .filter_map(|node| {
            if let Node::GitFile { path, status, .. } = node {
                let segments: Vec<String> = path.split('/').map(String::from).collect();
                if segments.is_empty() {
                    return None;
                }
                Some(FileEntry {
                    segments,
                    status: status.clone(),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Build a virtual directory tree from file entries.
///
/// Each path is split into segments and inserted into a nested `BTreeMap`.
/// Intermediate segments become `Dir` entries, leaf segments become `File`.
fn build_dir_tree(files: &[FileEntry]) -> BTreeMap<String, DirEntry> {
    let mut root: BTreeMap<String, DirEntry> = BTreeMap::new();

    for entry in files {
        let mut current = &mut root;
        let last_idx = entry.segments.len() - 1;

        for (i, segment) in entry.segments.iter().enumerate() {
            if i == last_idx {
                // Leaf file.
                current.insert(segment.clone(), DirEntry::File(entry.status.clone()));
            } else {
                // Intermediate directory.
                let dir = current
                    .entry(segment.clone())
                    .or_insert_with(|| DirEntry::Dir(BTreeMap::new()));
                if let DirEntry::Dir(ref mut children) = dir {
                    current = children;
                } else {
                    // Path conflict: a file already exists with a directory
                    // name. This shouldn't happen with well-formed paths,
                    // but gracefully skip rather than panic.
                    break;
                }
            }
        }
    }

    root
}

/// Count total lines the tree will render (directories + files).
fn count_tree_lines(tree: &BTreeMap<String, DirEntry>) -> usize {
    let mut count = 0;
    for entry in tree.values() {
        count += 1;
        if let DirEntry::Dir(children) = entry {
            count += count_tree_lines(children);
        }
    }
    count
}

// ── Tree rendering ──────────────────────────────────────────────────

/// Recursively render a level of the directory tree into styled lines.
fn render_tree_level(
    entries: &BTreeMap<String, DirEntry>,
    prefix: &TreePrefix,
    width: usize,
    max_lines: usize,
    lines: &mut Vec<Line<'static>>,
) {
    let total = entries.len();
    for (i, (name, entry)) in entries.iter().enumerate() {
        if lines.len() >= max_lines {
            return;
        }
        let is_last = i + 1 == total;
        let connector = prefix.render(is_last);

        match entry {
            DirEntry::Dir(children) => {
                let dir_name = format!("{name}/");
                let budget = width.saturating_sub(connector.chars().count());
                let display = truncate(&dir_name, budget);
                lines.push(Line::from(vec![
                    Span::styled(connector, Style::default().fg(Color::DarkGray)),
                    Span::styled(display, Style::default().fg(Color::Blue)),
                ]));
                let child_prefix = prefix.child(is_last);
                render_tree_level(children, &child_prefix, width, max_lines, lines);
            }
            DirEntry::File(status) => {
                let (badge, badge_color) = status_badge(status);
                // Budget: connector + name + " " + badge.
                let badge_width = 1 + badge.len() + 1;
                let name_budget = width.saturating_sub(connector.chars().count() + badge_width);
                let display = truncate(name, name_budget);
                lines.push(Line::from(vec![
                    Span::styled(connector, Style::default().fg(Color::DarkGray)),
                    Span::styled(display, Style::default().fg(Color::White)),
                    Span::raw(" "),
                    Span::styled(badge.to_string(), Style::default().fg(badge_color)),
                ]));
            }
        }
    }
}

/// Return a status badge string and color for a `GitFileStatus`.
fn status_badge(status: &GitFileStatus) -> (&'static str, Color) {
    match status {
        GitFileStatus::Modified => ("[Modified]", Color::Yellow),
        GitFileStatus::Staged => ("[Staged]", Color::Green),
        GitFileStatus::Untracked => ("[Untracked]", Color::Red),
        GitFileStatus::Tracked => ("[Tracked]", Color::DarkGray),
    }
}
