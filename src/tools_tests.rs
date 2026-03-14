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

/// Catches plan user trigger producing wrong variant — must produce `Plan`,
/// not `Unknown`.
#[test]
fn test_parse_user_trigger_args_plan() {
    let args = parse_user_trigger_args("plan", "Fix the login bug");
    match args {
        ToolCallArguments::Plan { title, description } => {
            assert_eq!(title, "Fix the login bug");
            assert!(description.is_none());
        }
        other => panic!("Expected Plan, got: {}", other.tool_name()),
    }
}
