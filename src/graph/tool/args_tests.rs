use crate::graph::tool::result::{ImageSource, ToolResultContent, ToolResultContentBlock};
use crate::graph::tool::types::{parse_tool_arguments, ToolCallArguments};

/// Catches `ToolCallArguments` tag discrimination failures across variants.
/// Each variant must serialize with a unique `tool_type` discriminant.
#[test]
fn test_tool_call_arguments_tagged_union_serde() {
    let variants = vec![
        ToolCallArguments::Plan {
            title: "fix".to_string(),
            description: Some("desc".to_string()),
        },
        ToolCallArguments::ReadFile {
            path: "/a".to_string(),
        },
        ToolCallArguments::WriteFile {
            path: "/b".to_string(),
            content: "data".to_string(),
        },
        ToolCallArguments::ListDirectory {
            path: ".".to_string(),
            recursive: Some(true),
        },
        ToolCallArguments::SearchFiles {
            pattern: "fn main".to_string(),
            path: None,
        },
        ToolCallArguments::WebSearch {
            query: "q".to_string(),
        },
        ToolCallArguments::Unknown {
            tool_name: "mcp_thing".to_string(),
            raw_json: r#"{"x":1}"#.to_string(),
        },
    ];

    for variant in &variants {
        let json = serde_json::to_string(variant).unwrap();
        let parsed: ToolCallArguments = serde_json::from_str(&json).unwrap();
        assert_eq!(
            variant.tool_name(),
            parsed.tool_name(),
            "Round-trip failed for {}",
            variant.tool_name()
        );
    }
}

/// Catches `to_input_json` producing invalid JSON or including the serde tag.
/// The Anthropic API expects raw input fields without the `tool_type` discriminant.
#[test]
fn test_to_input_json_strips_tag() {
    let read = ToolCallArguments::ReadFile {
        path: "/tmp/test.rs".to_string(),
    };
    let json = read.to_input_json();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.get("tool_type").is_none(), "tag should be stripped");
    assert_eq!(
        parsed.get("path").unwrap().as_str().unwrap(),
        "/tmp/test.rs"
    );

    let unknown = ToolCallArguments::Unknown {
        tool_name: "mcp_tool".to_string(),
        raw_json: r#"{"key":"val"}"#.to_string(),
    };
    assert_eq!(unknown.to_input_json(), r#"{"key":"val"}"#);

    let plan = ToolCallArguments::Plan {
        title: "fix the bug".to_string(),
        description: Some("plan desc".to_string()),
    };
    let plan_json = plan.to_input_json();
    let plan_parsed: serde_json::Value = serde_json::from_str(&plan_json).unwrap();
    assert!(plan_parsed.get("tool_type").is_none());
    assert_eq!(
        plan_parsed.get("title").unwrap().as_str().unwrap(),
        "fix the bug"
    );

    let write = ToolCallArguments::WriteFile {
        path: "/out.txt".to_string(),
        content: "hello".to_string(),
    };
    let write_json = write.to_input_json();
    let write_parsed: serde_json::Value = serde_json::from_str(&write_json).unwrap();
    assert!(write_parsed.get("tool_type").is_none());
    assert_eq!(
        write_parsed.get("path").unwrap().as_str().unwrap(),
        "/out.txt"
    );
    assert_eq!(
        write_parsed.get("content").unwrap().as_str().unwrap(),
        "hello"
    );

    let search = ToolCallArguments::WebSearch {
        query: "rust serde".to_string(),
    };
    let search_json = search.to_input_json();
    let search_parsed: serde_json::Value = serde_json::from_str(&search_json).unwrap();
    assert!(search_parsed.get("tool_type").is_none());
    assert_eq!(
        search_parsed.get("query").unwrap().as_str().unwrap(),
        "rust serde"
    );

    let list_dir = ToolCallArguments::ListDirectory {
        path: "src".to_string(),
        recursive: Some(true),
    };
    let list_json = list_dir.to_input_json();
    let list_parsed: serde_json::Value = serde_json::from_str(&list_json).unwrap();
    assert!(list_parsed.get("tool_type").is_none());
    assert_eq!(list_parsed.get("path").unwrap().as_str().unwrap(), "src");
    assert!(list_parsed.get("recursive").unwrap().as_bool().unwrap());

    let search_f = ToolCallArguments::SearchFiles {
        pattern: "fn main".to_string(),
        path: None,
    };
    let sf_json = search_f.to_input_json();
    let sf_parsed: serde_json::Value = serde_json::from_str(&sf_json).unwrap();
    assert!(sf_parsed.get("tool_type").is_none());
    assert_eq!(
        sf_parsed.get("pattern").unwrap().as_str().unwrap(),
        "fn main"
    );
    assert!(sf_parsed.get("path").unwrap().is_null());

    // H1: invalid JSON in Unknown falls back to empty object
    let bad_json = ToolCallArguments::Unknown {
        tool_name: "broken".to_string(),
        raw_json: "not valid json {{{".to_string(),
    };
    assert_eq!(bad_json.to_input_json(), "{}");
}

// ── parse_tool_arguments tests ───────────────────────────────────

/// Catches the C1 bug: LLM sends tagless JSON (no `tool_type` field).
/// `parse_tool_arguments` must produce the correct typed variant, not `Unknown`.
#[test]
fn test_parse_tool_arguments_tagless_read_file() {
    let args = parse_tool_arguments("read_file", r#"{"path": "/src/main.rs"}"#);
    match args {
        ToolCallArguments::ReadFile { path } => assert_eq!(path, "/src/main.rs"),
        other => panic!("Expected ReadFile, got: {}", other.tool_name()),
    }
}

/// Catches `write_file` parsing failure from tagless LLM JSON.
#[test]
fn test_parse_tool_arguments_tagless_write_file() {
    let args = parse_tool_arguments("write_file", r#"{"path": "out.txt", "content": "hello"}"#);
    match args {
        ToolCallArguments::WriteFile { path, content } => {
            assert_eq!(path, "out.txt");
            assert_eq!(content, "hello");
        }
        other => panic!("Expected WriteFile, got: {}", other.tool_name()),
    }
}

/// Catches `list_directory` parsing failure; also tests optional field omission.
#[test]
fn test_parse_tool_arguments_tagless_list_directory() {
    let args = parse_tool_arguments("list_directory", r#"{"path": "src"}"#);
    match args {
        ToolCallArguments::ListDirectory { path, recursive } => {
            assert_eq!(path, "src");
            assert_eq!(recursive, None);
        }
        other => panic!("Expected ListDirectory, got: {}", other.tool_name()),
    }

    let args2 = parse_tool_arguments("list_directory", r#"{"path": ".", "recursive": true}"#);
    match args2 {
        ToolCallArguments::ListDirectory { path, recursive } => {
            assert_eq!(path, ".");
            assert!(recursive.unwrap());
        }
        other => panic!("Expected ListDirectory, got: {}", other.tool_name()),
    }
}

/// Catches `search_files` parsing failure from tagless LLM JSON.
#[test]
fn test_parse_tool_arguments_tagless_search_files() {
    let args = parse_tool_arguments("search_files", r#"{"pattern": "fn main", "path": "src"}"#);
    match args {
        ToolCallArguments::SearchFiles { pattern, path } => {
            assert_eq!(pattern, "fn main");
            assert_eq!(path.as_deref(), Some("src"));
        }
        other => panic!("Expected SearchFiles, got: {}", other.tool_name()),
    }
}

/// Catches unknown tool name regression — unrecognized names must produce Unknown.
#[test]
fn test_parse_tool_arguments_unknown_tool() {
    let args = parse_tool_arguments("mcp_custom_tool", r#"{"x": 1}"#);
    match args {
        ToolCallArguments::Unknown {
            tool_name,
            raw_json,
        } => {
            assert_eq!(tool_name, "mcp_custom_tool");
            assert_eq!(raw_json, r#"{"x": 1}"#);
        }
        other => panic!("Expected Unknown, got: {}", other.tool_name()),
    }
}

/// Catches wrong-fields regression — known tool name with invalid fields
/// must fall through to Unknown, not panic.
#[test]
fn test_parse_tool_arguments_mismatched_fields() {
    let args = parse_tool_arguments("read_file", r#"{"wrong_field": 42}"#);
    assert_eq!(args.tool_name(), "read_file");
    // Missing required "path" field: should fall to Unknown
    assert!(matches!(args, ToolCallArguments::Unknown { .. }));
}

// ── ToolResultContent tests ──────────────────────────────────────

#[test]
fn test_tool_result_content_text_serde_roundtrip() {
    let content = ToolResultContent::text("hello");
    let json = serde_json::to_string(&content).unwrap();
    assert_eq!(json, r#""hello""#);
    let parsed: ToolResultContent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.text_content(), "hello");
    assert!(!parsed.has_images());
}

#[test]
fn test_tool_result_content_blocks_serde_roundtrip() {
    let content = ToolResultContent::Blocks(vec![
        ToolResultContentBlock::Text {
            text: "here is the image".to_string(),
        },
        ToolResultContentBlock::Image {
            source: ImageSource::Base64 {
                media_type: "image/png".to_string(),
                data: "iVBOR".to_string(),
            },
        },
    ]);
    let json = serde_json::to_string(&content).unwrap();
    let parsed: ToolResultContent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.text_content(), "here is the image");
    assert!(parsed.has_images());
}

#[test]
fn test_tool_result_content_backward_compat_v2() {
    let old_json = r#""file contents here""#;
    let parsed: ToolResultContent = serde_json::from_str(old_json).unwrap();
    assert_eq!(parsed.text_content(), "file contents here");
    assert!(!parsed.has_images());
}

#[test]
fn test_tool_result_content_char_len() {
    let text = ToolResultContent::text("hello");
    assert_eq!(text.char_len(), 5);

    let blocks = ToolResultContent::Blocks(vec![
        ToolResultContentBlock::Text {
            text: "abc".to_string(),
        },
        ToolResultContentBlock::Image {
            source: ImageSource::Base64 {
                media_type: "image/png".to_string(),
                data: "AAAA".to_string(),
            },
        },
    ]);
    assert_eq!(blocks.char_len(), 7); // 3 + 4
}
