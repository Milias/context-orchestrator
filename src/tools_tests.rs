use super::*;

#[test]
fn test_parse_triggers_finds_plan() {
    let triggers = parse_triggers("~plan Fix the login bug");
    assert_eq!(triggers.len(), 1);
    match &triggers[0] {
        TriggerCommand::Plan { args } => assert_eq!(args, "Fix the login bug"),
    }
}

#[test]
fn test_parse_triggers_no_triggers() {
    let triggers = parse_triggers("Hello, how are you?");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_multiple() {
    let triggers = parse_triggers("~plan First task\n~plan Second task");
    assert_eq!(triggers.len(), 2);
    match &triggers[0] {
        TriggerCommand::Plan { args } => assert_eq!(args, "First task"),
    }
    match &triggers[1] {
        TriggerCommand::Plan { args } => assert_eq!(args, "Second task"),
    }
}

#[test]
fn test_parse_triggers_ignores_unknown() {
    let triggers = parse_triggers("~foobar some stuff");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_mid_word_ignored() {
    let triggers = parse_triggers("approx~plan should not match");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_no_args_ignored() {
    let triggers = parse_triggers("~plan");
    assert!(triggers.is_empty());
}

#[test]
fn test_parse_triggers_with_leading_whitespace() {
    let triggers = parse_triggers("  ~plan Indented task");
    assert_eq!(triggers.len(), 1);
    match &triggers[0] {
        TriggerCommand::Plan { args } => assert_eq!(args, "Indented task"),
    }
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
