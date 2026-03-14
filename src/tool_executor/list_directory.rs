use super::security;
use super::ToolExecutionResult;
use crate::graph::tool_types::ToolResultContent;
use std::collections::VecDeque;
use std::fmt::Write;
use std::path::Path;

const MAX_LIST_ENTRIES: usize = 1000;

pub async fn execute(
    path: &str,
    recursive: bool,
    working_dir: Option<&Path>,
) -> ToolExecutionResult {
    let resolved_path = if path.is_empty() { "." } else { path };
    let validated = match security::validate_path(resolved_path, working_dir).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !validated.canonical.is_dir() {
        return ToolExecutionResult {
            content: ToolResultContent::text(format!("Error: not a directory: {path}")),
            is_error: true,
        };
    }
    if recursive {
        list_recursive(&validated.canonical).await
    } else {
        list_single(&validated.canonical).await
    }
}

struct Entry {
    relative_path: String,
    is_dir: bool,
    size: Option<u64>,
}

async fn list_single(dir: &Path) -> ToolExecutionResult {
    let mut reader = match tokio::fs::read_dir(dir).await {
        Ok(r) => r,
        Err(e) => {
            return ToolExecutionResult {
                content: ToolResultContent::text(format!("Error reading directory: {e}")),
                is_error: true,
            }
        }
    };
    let mut entries = Vec::new();
    let mut truncated = false;
    while let Ok(Some(entry)) = reader.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata().await.ok();
        let is_dir = meta.as_ref().is_some_and(std::fs::Metadata::is_dir);
        let size = if is_dir { None } else { meta.map(|m| m.len()) };
        entries.push(Entry {
            relative_path: name,
            is_dir,
            size,
        });
        if entries.len() >= MAX_LIST_ENTRIES {
            truncated = true;
            break;
        }
    }
    format_entries(&entries, truncated)
}

async fn list_recursive(root: &Path) -> ToolExecutionResult {
    let mut entries = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(root.to_path_buf());

    while let Some(dir) = queue.pop_front() {
        let Ok(mut reader) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = reader.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || super::SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            // Use file_type() (lstat) to avoid following symlinks outside CWD.
            let Ok(ft) = entry.file_type().await else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let is_dir = ft.is_dir();
            let size = if is_dir {
                None
            } else {
                entry.metadata().await.ok().map(|m| m.len())
            };
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(&entry.path())
                .to_string_lossy()
                .to_string();
            entries.push(Entry {
                relative_path: rel,
                is_dir,
                size,
            });
            if is_dir {
                queue.push_back(entry.path());
            }
            if entries.len() >= MAX_LIST_ENTRIES {
                return format_entries(&entries, true);
            }
        }
    }
    format_entries(&entries, false)
}

fn format_entries(entries: &[Entry], truncated: bool) -> ToolExecutionResult {
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });

    let mut output = String::new();
    for entry in &sorted {
        if entry.is_dir {
            let _ = writeln!(output, "dir/  {}/", entry.relative_path);
        } else if let Some(size) = entry.size {
            let _ = writeln!(output, "file  {} ({size} bytes)", entry.relative_path);
        } else {
            let _ = writeln!(output, "file  {}", entry.relative_path);
        }
    }
    if truncated {
        let _ = writeln!(
            output,
            "\n[truncated, showing {MAX_LIST_ENTRIES} of more entries]"
        );
    }
    if output.is_empty() {
        output = "Directory is empty".to_string();
    }
    ToolExecutionResult {
        content: ToolResultContent::text(output.trim_end()),
        is_error: false,
    }
}
