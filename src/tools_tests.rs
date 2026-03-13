use super::*;

#[test]
fn test_parse_triggers_finds_plan() {
    let triggers = parse_triggers("/plan Fix the login bug");
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].tool_name, "plan");
    assert_eq!(triggers[0].args, "Fix the login bug");
}

#[test]
fn test_parse_triggers_no_triggers() {
    let triggers = parse_triggers("Hello, how are you?");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_multiple() {
    let triggers = parse_triggers("/plan First task\n/plan Second task");
    assert_eq!(triggers.len(), 2);
    assert_eq!(triggers[0].args, "First task");
    assert_eq!(triggers[1].args, "Second task");
}

#[test]
fn test_parse_triggers_ignores_unknown() {
    let triggers = parse_triggers("/foobar some stuff");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_mid_word_ignored() {
    let triggers = parse_triggers("approx/plan should not match");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_no_args_ignored() {
    let triggers = parse_triggers("/plan");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_with_leading_whitespace() {
    let triggers = parse_triggers("  /plan Indented task");
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].args, "Indented task");
}

/// Registry-based parsing matches any registered tool, not just plan.
#[test]
fn test_parse_triggers_read_file() {
    let triggers = parse_triggers("/read_file src/main.rs");
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].tool_name, "read_file");
    assert_eq!(triggers[0].args, "src/main.rs");
}

/// The /set tool is parsed from the registry.
#[test]
fn test_parse_triggers_set() {
    let triggers = parse_triggers("/set max_tokens 16384");
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].tool_name, "set");
    assert_eq!(triggers[0].args, "max_tokens 16384");
}

#[test]
fn test_plan_extraction_result_serde() {
    let result = PlanExtractionResult {
        title: "Fix login".to_string(),
        description: Some("The login page has a bug".to_string()),
    };
    let json = serde_json::to_string(&result).unwrap();
    let parsed: PlanExtractionResult = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.title, "Fix login");
    assert_eq!(
        parsed.description,
        Some("The login page has a bug".to_string())
    );
}

#[test]
fn test_plan_extraction_result_null_description() {
    let json = r#"{"title": "Quick fix", "description": null}"#;
    let parsed: PlanExtractionResult = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.title, "Quick fix");
    assert!(parsed.description.is_none());
}

#[test]
fn test_plan_result_to_node() {
    let result = PlanExtractionResult {
        title: "Test task".to_string(),
        description: Some("Details".to_string()),
    };
    let node = plan_result_to_node(&result);
    assert_eq!(node.content(), "Test task");
    match &node {
        Node::WorkItem {
            title,
            status,
            description,
            ..
        } => {
            assert_eq!(title, "Test task");
            assert_eq!(*status, WorkItemStatus::Todo);
            assert_eq!(description.as_deref(), Some("Details"));
        }
        _ => panic!("Expected WorkItem node"),
    }
}

#[test]
fn test_parse_triggers_planning_not_matched() {
    let triggers = parse_triggers("/planning some stuff");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_plan_with_punctuation() {
    // /plan. is not a match because '.' is not ' ' or empty
    let triggers = parse_triggers("/plan.something");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_plan_at_eof_no_newline() {
    let triggers = parse_triggers("/plan Fix it");
    assert_eq!(triggers.len(), 1);
}

#[test]
fn test_truncate_content_unicode_safe() {
    let emoji_str = "🎉🎊🎈🎁🎂";
    let result = truncate_content(emoji_str, 3);
    assert_eq!(result, "🎉🎊🎈...");
}

#[test]
fn test_truncate_content_cjk_safe() {
    let cjk = "你好世界测试";
    let result = truncate_content(cjk, 4);
    assert_eq!(result, "你好世界...");
}

#[test]
fn test_truncate_content_short_string_unchanged() {
    assert_eq!(truncate_content("hello", 10), "hello");
}
