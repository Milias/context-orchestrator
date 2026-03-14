use super::plan_tools;
use super::qa_tools;
use uuid::Uuid;

/// Bug: `execute_plan` returned text omits the title — agent loop
/// cannot confirm which plan was created.
#[test]
fn test_execute_plan_includes_title() {
    let result = plan_tools::execute_plan("Fix auth flow");
    assert!(!result.is_error);
    assert!(
        result.content.text_content().contains("Fix auth flow"),
        "should include title: {}",
        result.content.text_content()
    );
}

/// Bug: `execute_add_task` returned text omits the title.
#[test]
fn test_execute_add_task_includes_title() {
    let result = plan_tools::execute_add_task("Implement JWT");
    assert!(!result.is_error);
    assert!(
        result.content.text_content().contains("Implement JWT"),
        "should include title: {}",
        result.content.text_content()
    );
}

/// Bug: `execute_update_work_item` placeholder returns `is_error: true`,
/// causing the agent to retry indefinitely.
#[test]
fn test_execute_update_work_item_not_error() {
    let result = plan_tools::execute_update_work_item();
    assert!(!result.is_error);
}

/// Bug: `execute_add_dependency` placeholder returns `is_error: true`.
#[test]
fn test_execute_add_dependency_not_error() {
    let result = plan_tools::execute_add_dependency();
    assert!(!result.is_error);
}

/// Bug: `execute_ask` returned text omits the question — agent cannot
/// confirm which question was submitted.
#[test]
fn test_execute_ask_includes_question() {
    let result = qa_tools::execute_ask("What JWT library?");
    assert!(!result.is_error);
    assert!(
        result.content.text_content().contains("What JWT library?"),
        "should include question: {}",
        result.content.text_content()
    );
}

/// Bug: `execute_answer` returned text omits the question ID — agent
/// cannot confirm which question was answered.
#[test]
fn test_execute_answer_includes_question_id() {
    let qid = Uuid::new_v4();
    let result = qa_tools::execute_answer(&qid);
    assert!(!result.is_error);
    assert!(
        result.content.text_content().contains(&qid.to_string()),
        "should include question_id: {}",
        result.content.text_content()
    );
}
