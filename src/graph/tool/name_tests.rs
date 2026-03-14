use crate::graph::tool::types::{ToolCallArguments, ToolName};

/// Bug: `ToolName::from_str` / `as_str` round-trip broken for a variant —
/// wire name silently mismatches, causing tool dispatch to fail.
#[test]
fn test_tool_name_round_trip_all_variants() {
    let variants = [
        ToolName::Plan,
        ToolName::AddTask,
        ToolName::UpdateWorkItem,
        ToolName::AddDependency,
        ToolName::ReadFile,
        ToolName::WriteFile,
        ToolName::ListDirectory,
        ToolName::SearchFiles,
        ToolName::WebSearch,
        ToolName::Set,
        ToolName::Ask,
        ToolName::Answer,
    ];

    for variant in variants {
        let wire = variant.as_str();
        let parsed = ToolName::from_str(wire);
        assert_eq!(parsed, Some(variant), "round-trip failed for {wire}");
    }
}

/// Bug: `from_str` returns `Some` for an unknown tool name instead of `None`.
#[test]
fn test_tool_name_from_str_unknown_returns_none() {
    assert!(ToolName::from_str("nonexistent").is_none());
    assert!(ToolName::from_str("").is_none());
}

/// Bug: `display_summary` for `Unknown` does not truncate long raw JSON,
/// causing TUI line overflow.
#[test]
fn test_display_summary_unknown_truncates() {
    let args = ToolCallArguments::Unknown {
        tool_name: "custom_tool".to_string(),
        raw_json: "x".repeat(200),
    };
    let summary = args.display_summary();
    assert!(
        summary.contains("..."),
        "long raw_json should be truncated with ellipsis"
    );
    // 80 chars of raw_json + tool_name prefix + "..." should be well under 200.
    assert!(
        summary.len() < 200,
        "summary should be shorter than raw input"
    );
}

/// Bug: `display_summary` for short `Unknown` adds spurious "..." suffix.
#[test]
fn test_display_summary_unknown_short_no_ellipsis() {
    let args = ToolCallArguments::Unknown {
        tool_name: "custom".to_string(),
        raw_json: r#"{"key": "val"}"#.to_string(),
    };
    let summary = args.display_summary();
    assert!(
        !summary.contains("..."),
        "short raw_json should not have ellipsis"
    );
}

/// Bug: `parse_tool_arguments` silently produces `Unknown` for a valid
/// tool name when the JSON fields are correct.
#[test]
fn test_parse_tool_arguments_valid_plan() {
    use crate::graph::tool::types::parse_tool_arguments;
    let args = parse_tool_arguments("plan", r#"{"title": "Fix bug", "description": null}"#);
    match args {
        ToolCallArguments::Plan { title, .. } => assert_eq!(title, "Fix bug"),
        other => panic!("Expected Plan, got {}", other.tool_name()),
    }
}

/// Bug: `parse_tool_arguments` panics on malformed JSON instead of
/// falling back to `Unknown`.
#[test]
fn test_parse_tool_arguments_invalid_json_falls_back() {
    use crate::graph::tool::types::parse_tool_arguments;
    let args = parse_tool_arguments("plan", "not json at all");
    assert!(
        matches!(args, ToolCallArguments::Unknown { .. }),
        "malformed JSON should produce Unknown"
    );
}
