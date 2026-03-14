use super::security;
use super::ToolExecutionResult;
use crate::graph::tool_types::ToolResultContent;
use std::collections::VecDeque;
use std::fmt::Write;
use std::path::Path;

const MAX_SEARCH_RESULTS: usize = 50;
const BINARY_CHECK_BYTES: usize = 8192;
const MAX_SEARCH_FILE_BYTES: u64 = 10_000_000; // 10 MB
const MAX_DIRS_VISITED: usize = 5_000;

pub async fn execute(pattern: &str, path: Option<&str>, working_dir: Option<&Path>) -> ToolExecutionResult {
    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => {
            return ToolExecutionResult {
                content: ToolResultContent::text(format!("Invalid regex pattern: {e}")),
                is_error: true,
            }
        }
    };
    let resolved = path.unwrap_or(".");
    let validated = match security::validate_path(resolved, working_dir).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !validated.canonical.is_dir() {
        return ToolExecutionResult {
            content: ToolResultContent::text(format!("Error: not a directory: {resolved}")),
            is_error: true,
        };
    }
    let root = &validated.canonical;
    let mut results = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(root.clone());
    let mut dirs_visited = 0usize;

    'outer: while let Some(dir) = queue.pop_front() {
        dirs_visited += 1;
        if dirs_visited > MAX_DIRS_VISITED {
            break;
        }
        let Ok(mut reader) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        let mut dir_entries = Vec::new();
        while let Ok(Some(entry)) = reader.next_entry().await {
            dir_entries.push(entry);
        }
        dir_entries.sort_by_key(tokio::fs::DirEntry::file_name);

        for entry in dir_entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || super::SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            // Use file_type() (lstat) instead of metadata() (stat) to avoid
            // following symlinks that could point outside CWD.
            let Ok(ft) = entry.file_type().await else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                queue.push_back(entry.path());
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            if let Some(matches) = search_file(&entry.path(), root, &re).await {
                for m in matches {
                    results.push(m);
                    if results.len() >= MAX_SEARCH_RESULTS {
                        break 'outer;
                    }
                }
            }
        }
    }

    if results.is_empty() {
        return ToolExecutionResult {
            content: ToolResultContent::text(format!("No matches found for pattern: {pattern}")),
            is_error: false,
        };
    }

    let truncated = results.len() >= MAX_SEARCH_RESULTS;
    let mut output = String::new();
    for r in &results {
        let _ = writeln!(output, "{r}");
    }
    if truncated {
        let _ = writeln!(output, "\n[{MAX_SEARCH_RESULTS} results, search stopped]");
    }
    ToolExecutionResult {
        content: ToolResultContent::text(output.trim_end()),
        is_error: false,
    }
}

async fn search_file(file_path: &Path, root: &Path, re: &regex::Regex) -> Option<Vec<String>> {
    // Skip files that are too large to search.
    let meta = tokio::fs::symlink_metadata(file_path).await.ok()?;
    if meta.len() > MAX_SEARCH_FILE_BYTES {
        return None;
    }
    // Binary check: read full file (bounded by size guard above) and scan header.
    let content = tokio::fs::read(file_path).await.ok()?;
    let check_len = content.len().min(BINARY_CHECK_BYTES);
    if content[..check_len].contains(&0) {
        return None;
    }
    let text = String::from_utf8(content).ok()?;
    let rel = file_path
        .strip_prefix(root)
        .unwrap_or(file_path)
        .to_string_lossy();
    let mut matches = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if re.is_match(line) {
            let trimmed = truncate_at_boundary(line, 200);
            matches.push(format!("{}:{}: {}", rel, i + 1, trimmed));
        }
    }
    if matches.is_empty() {
        None
    } else {
        Some(matches)
    }
}

/// Truncate a string to at most `max_bytes` bytes at a valid UTF-8 char boundary.
fn truncate_at_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
