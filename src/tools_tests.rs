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

/// Bug: `/set` trigger parsed into wrong variant — key/value
/// not split correctly (both end up in `key`).
#[test]
fn test_parse_user_trigger_args_set_splits_key_value() {
    let args = parse_user_trigger_args("set", "model claude-opus");
    match args {
        ToolCallArguments::Set { key, value } => {
            assert_eq!(key, "model");
            assert_eq!(value, "claude-opus");
        }
        other => panic!("Expected Set, got: {}", other.tool_name()),
    }
}

/// Bug: `/ask` trigger with explicit destination parsed incorrectly —
/// destination not recognized, defaults silently to `User`.
#[test]
fn test_parse_user_trigger_args_ask_llm() {
    use crate::graph::node::QuestionDestination;
    let args = parse_user_trigger_args("ask", "llm Should we use JWT?");
    match args {
        ToolCallArguments::Ask {
            question,
            destination,
            ..
        } => {
            assert_eq!(destination, QuestionDestination::Llm);
            assert_eq!(question, "Should we use JWT?");
        }
        other => panic!("Expected Ask, got: {}", other.tool_name()),
    }
}

/// Bug: `/ask` with user destination doesn't parse the question text.
#[test]
fn test_parse_user_trigger_args_ask_user() {
    use crate::graph::node::QuestionDestination;
    let args = parse_user_trigger_args("ask", "user What JWT library?");
    match args {
        ToolCallArguments::Ask {
            question,
            destination,
            ..
        } => {
            assert_eq!(destination, QuestionDestination::User);
            assert_eq!(question, "What JWT library?");
        }
        other => panic!("Expected Ask, got: {}", other.tool_name()),
    }
}

/// Bug: unrecognized tool name does not fall through to `Unknown` —
/// panics or returns wrong variant.
#[test]
fn test_parse_user_trigger_args_unknown_tool() {
    let args = parse_user_trigger_args("nonexistent", "some args");
    match args {
        ToolCallArguments::Unknown {
            tool_name,
            raw_json,
        } => {
            assert_eq!(tool_name, "nonexistent");
            assert_eq!(raw_json, "some args");
        }
        other => panic!("Expected Unknown, got: {}", other.tool_name()),
    }
}

/// Bug: `/read_file` trigger does not preserve path with spaces.
#[test]
fn test_parse_user_trigger_args_read_file() {
    let args = parse_user_trigger_args("read_file", "src/my file.rs");
    match args {
        ToolCallArguments::ReadFile { path } => {
            assert_eq!(path, "src/my file.rs");
        }
        other => panic!("Expected ReadFile, got: {}", other.tool_name()),
    }
}
