use super::strip_json_fences;

#[test]
fn test_strip_json_fences_bare_json() {
    let input = r#"{"title": "Fix bug"}"#;
    assert_eq!(strip_json_fences(input), input);
}

#[test]
fn test_strip_json_fences_with_json_tag() {
    let input = "```json\n{\"title\": \"Fix bug\"}\n```";
    assert_eq!(strip_json_fences(input), r#"{"title": "Fix bug"}"#);
}

#[test]
fn test_strip_json_fences_without_language_tag() {
    let input = "```\n{\"title\": \"Fix bug\"}\n```";
    assert_eq!(strip_json_fences(input), r#"{"title": "Fix bug"}"#);
}

#[test]
fn test_strip_json_fences_with_surrounding_whitespace() {
    let input = "  ```json\n{\"title\": \"Fix bug\"}\n```  ";
    assert_eq!(strip_json_fences(input), r#"{"title": "Fix bug"}"#);
}

#[test]
fn test_strip_json_fences_no_closing_fence() {
    let input = "```json\n{\"title\": \"Fix bug\"}";
    // No closing fence, should return trimmed input
    assert_eq!(strip_json_fences(input), input.trim());
}

#[test]
fn test_strip_json_fences_empty_string() {
    assert_eq!(strip_json_fences(""), "");
}
