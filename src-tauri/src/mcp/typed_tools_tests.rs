//! Pure schema/validation checks and negative direct calls only. These tests
//! do not initialize APP, contact Canvas, probe credentials, or invoke AI.
use super::*;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn id(value: i64) -> CanvasId {
    CanvasId::try_from(value).unwrap()
}

fn calendar(start: &str, end: &str) -> CalendarParams {
    CalendarParams {
        context_codes: vec!["course_9000000000".into()],
        start_date: start.into(),
        end_date: end.into(),
    }
}

#[test]
fn tool_surface_schemas_and_annotations_are_explicit() {
    let tools = McpServer { headless: false }.router().list_all();
    let expected: &[(&str, &[&str])] = &[
        ("list_courses", &[]),
        ("get_me", &[]),
        ("list_course_assignments", &["course_id"]),
        ("list_course_files", &["course_id"]),
        ("list_course_folders", &["course_id"]),
        ("list_course_users", &["course_id"]),
        ("list_course_students", &["course_id"]),
        ("list_folder_files", &["folder_id"]),
        ("list_folder_folders", &["folder_id"]),
        ("list_my_folders", &[]),
        ("get_folder_by_id", &["folder_id"]),
        ("get_colors", &[]),
        ("list_calendar_events", &["context_codes", "start_date", "end_date"]),
        ("list_discussion_topics", &["course_id"]),
        ("get_full_discussion", &["course_id", "topic_id"]),
        ("list_course_images", &["course_id"]),
        ("get_my_single_submission", &["course_id", "assignment_id"]),
        ("list_course_assignment_submissions", &["course_id", "assignment_id"]),
        ("get_single_course_assignment_submission", &["course_id", "assignment_id", "student_id"]),
        ("list_user_submissions", &["course_id", "student_ids"]),
        ("list_external_module_items", &["course_id"]),
        ("chat", &["prompt"]),
        ("explain_file", &["course_id", "file_id"]),
        ("test_token", &["token"]),
    ];
    assert_eq!(tools.len(), expected.len());
    for (name, fields) in expected {
        let tool = tools.iter().find(|tool| tool.name == *name).unwrap();
        let schema = serde_json::to_value(&tool.input_schema).unwrap();
        assert_eq!(schema["type"], "object", "{name}");
        assert_eq!(schema["additionalProperties"], false, "{name}");
        let mut required: Vec<&str> = schema["required"].as_array().into_iter()
            .flatten().map(|value| value.as_str().unwrap()).collect();
        required.sort_unstable();
        let mut expected_fields = fields.to_vec();
        expected_fields.sort_unstable();
        assert_eq!(required, expected_fields, "{name}");
        for field in *fields {
            assert!(schema["properties"][field].is_object(), "{name}.{field}");
        }
        let hints = tool.annotations.as_ref().expect("explicit safety annotations");
        let sensitive = ["chat", "explain_file", "test_token"].contains(name);
        assert_eq!(hints.read_only_hint, Some(!sensitive), "{name}");
        assert_eq!(hints.idempotent_hint, Some(!sensitive), "{name}");
        assert_eq!(hints.destructive_hint, Some(false), "{name}");
        assert_eq!(hints.open_world_hint, Some(true), "{name}");
    }
    let schema = |name: &str| -> Value {
        serde_json::to_value(&tools.iter().find(|tool| tool.name == name).unwrap().input_schema).unwrap()
    };
    assert!(schema("get_my_single_submission").to_string().contains("\"minimum\":1"));
    for (name, field) in [("list_user_submissions", "student_ids"), ("list_calendar_events", "context_codes")] {
        assert_eq!(schema(name)["properties"][field]["minItems"], 1);
        assert_eq!(schema(name)["properties"][field]["maxItems"], 100);
    }
    assert_eq!(schema("chat")["properties"]["prompt"]["maxLength"], 32000);
    assert!(schema("explain_file")["properties"]["file"].is_null());
    assert!(schema("explain_file")["properties"]["url"].is_null());
}

#[test]
fn headless_hides_only_ai_and_token_probe_tools() {
    let gui = McpServer { headless: false }.router().list_all();
    let headless = McpServer { headless: true }.router().list_all();
    assert_eq!(gui.len(), 24);
    assert_eq!(headless.len(), 21);
    for name in ["chat", "explain_file", "test_token"] {
        assert!(gui.iter().any(|tool| tool.name == name));
        assert!(!headless.iter().any(|tool| tool.name == name));
    }
    for tool in &headless {
        assert!(gui.iter().any(|original| original.name == tool.name));
    }
}

#[test]
fn positive_ids_preserve_signed_64_bit_range_without_coercion() {
    for value in [1, i32::MAX as i64 + 1, 9_000_000_000, i64::MAX] {
        let params: CourseParams = serde_json::from_value(json!({"course_id": value})).unwrap();
        assert_eq!(params.course_id.get(), value);
    }
    for value in [json!(0), json!(-1), json!(i64::MIN), json!("1"), json!(true), json!(1.0), json!(null), json!([]), json!({}), json!(u64::MAX)] {
        assert!(serde_json::from_value::<CourseParams>(json!({"course_id": value})).is_err());
    }
    assert!(serde_json::from_str::<CourseParams>(r#"{"course_id":9223372036854775808}"#).is_err());
    assert!(serde_json::from_str::<CourseParams>(r#"{"course_id":1,"course_id":2}"#).is_err());
}

fn assert_contract<T: DeserializeOwned>(valid: Value) {
    assert!(serde_json::from_value::<T>(valid.clone()).is_ok());
    let fields = valid.as_object().unwrap();
    for field in fields.keys() {
        let mut missing = fields.clone();
        missing.remove(field);
        assert!(serde_json::from_value::<T>(Value::Object(missing)).is_err());
    }
    let mut unknown = fields.clone();
    unknown.insert("injected_credential".into(), json!("must-not-be-echoed"));
    assert!(serde_json::from_value::<T>(Value::Object(unknown)).is_err());
}

#[test]
fn every_parameter_contract_requires_fields_and_denies_unknown_fields() {
    assert_contract::<EmptyParams>(json!({}));
    assert_contract::<CourseParams>(json!({"course_id":1}));
    assert_contract::<FolderParams>(json!({"folder_id":1}));
    assert_contract::<AssignmentParams>(json!({"course_id":1,"assignment_id":2}));
    assert_contract::<StudentSubmissionParams>(json!({"course_id":1,"assignment_id":2,"student_id":3}));
    assert_contract::<UserSubmissionsParams>(json!({"course_id":1,"student_ids":[2]}));
    assert_contract::<DiscussionParams>(json!({"course_id":1,"topic_id":2}));
    assert_contract::<CalendarParams>(json!({"context_codes":["course_1"],"start_date":"2026-01-01","end_date":"2026-01-02"}));
    assert_contract::<ChatParams>(json!({"prompt":"hi"}));
    assert_contract::<ExplainFileParams>(json!({"course_id":1,"file_id":2}));
    assert_contract::<TokenParams>(json!({"token":"synthetic-test-value"}));
    for field in ["file", "url", "path"] {
        let mut input = json!({"course_id":1,"file_id":2});
        input[field] = json!({"url":"http://127.0.0.1/private"});
        assert!(serde_json::from_value::<ExplainFileParams>(input).is_err());
    }
}

#[test]
fn student_vectors_are_nonempty_bounded_and_strictly_typed() {
    assert!(validate_student_ids(&[]).is_err());
    assert!(validate_student_ids(&vec![id(i64::MAX); MAX_VECTOR_ITEMS]).is_ok());
    assert!(validate_student_ids(&vec![id(1); MAX_VECTOR_ITEMS + 1]).is_err());
    for ids in [json!([0]), json!([-1]), json!([1,"2"]), json!([1,null]), json!(1), json!(null)] {
        assert!(serde_json::from_value::<UserSubmissionsParams>(json!({"course_id":1,"student_ids":ids})).is_err());
    }
}

#[test]
fn calendar_dates_are_real_strict_and_ordered_without_implicit_timezone() {
    for (start, end) in [
        ("2024-02-29", "2024-02-29"),
        ("2026-01-01", "2026-01-31"),
        ("2026-01-02T00:00:00+08:00", "2026-01-01T16:00:00Z"),
        ("2026-01-02T00:00:00+08:00", "2026-01-01T17:00:00Z"),
    ] {
        assert!(validate_calendar(&calendar(start, end)).is_ok(), "{start} / {end}");
    }
    for (start, end) in [
        ("2026-02-29", "2026-03-01"),
        ("2026-1-01", "2026-01-31"),
        ("2026-01-01", "2025-12-31"),
        ("2026-01-01", "2026-01-01T23:00:00Z"),
        ("2026-01-01T23:00:00Z", "2026-01-02"),
        ("2026-01-01T00:00:00", "2026-01-02T00:00:00"),
        ("2026-01-01T18:00:00Z", "2026-01-02T00:00:00+08:00"),
        ("tomorrow", "next week"),
        (" 2026-01-01", "2026-01-31"),
    ] {
        assert!(validate_calendar(&calendar(start, end)).is_err(), "{start} / {end}");
    }
    // No normalization: a validated caller offset must reach the App unchanged.
    let params = calendar("2026-01-01T00:00:00+08:00", "2026-01-02T00:00:00+08:00");
    validate_calendar(&params).unwrap();
    assert_eq!(params.start_date, "2026-01-01T00:00:00+08:00");
}

#[test]
fn calendar_contexts_use_bounded_vectors_of_positive_signed_64_bit_course_ids() {
    let mut params = calendar("2026-01-01", "2026-01-02");
    for code in ["course_1", "course_9000000000", "course_9223372036854775807"] {
        params.context_codes = vec![code.into(); 100];
        assert!(validate_calendar(&params).is_ok());
    }
    for code in ["", "course_", "course_0", "course_-1", "course_+1", "course_9223372036854775808", "course_18446744073709551615", "course_１", "user_1", "course_1&token=secret", "course_1 "] {
        params.context_codes = vec![code.into()];
        assert!(validate_calendar(&params).is_err(), "{code}");
    }
    params.context_codes.clear();
    assert!(validate_calendar(&params).is_err());
    params.context_codes = vec!["course_1".into(); 101];
    assert!(validate_calendar(&params).is_err());
}

#[test]
fn prompt_limit_counts_unicode_characters_and_token_validation_never_echoes() {
    assert!(validate_prompt(&"界".repeat(MAX_PROMPT_CHARS)).is_ok());
    assert!(validate_prompt(&"界".repeat(MAX_PROMPT_CHARS + 1)).is_err());
    for prompt in ["", "  \t\r\n", "\u{2003}"] {
        assert!(validate_prompt(prompt).is_err());
    }
    assert!(validate_token("synthetic-test-token").is_ok());
    for token in ["", " ", "Bearer synthetic-secret", "secret\r\nheader", "secret\0value"] {
        let error = validate_token(token).unwrap_err();
        assert!(!error.message.contains("secret"));
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    }
}

#[tokio::test]
async fn headless_direct_calls_reject_before_app_access_or_semantic_validation() {
    let server = McpServer { headless: true };
    let ai = server.chat(Parameters(ChatParams { prompt: String::new() })).await.unwrap_err();
    let file = server.explain_file(Parameters(ExplainFileParams { course_id: id(1), file_id: id(2) })).await.unwrap_err();
    let token = server.test_token(Parameters(TokenParams { token: String::new() })).await.unwrap_err();
    assert!(ai.message.contains("AI tools are disabled"));
    assert!(file.message.contains("AI tools are disabled"));
    assert!(token.message.contains("Token probing is disabled"));
}

#[tokio::test]
async fn semantic_failures_return_invalid_params_before_any_app_access() {
    let server = McpServer { headless: false };
    let ids = server.list_user_submissions(Parameters(UserSubmissionsParams { course_id: id(1), student_ids: vec![] })).await.unwrap_err();
    let dates = server.list_calendar_events(Parameters(calendar("invalid", "invalid"))).await.unwrap_err();
    let prompt = server.chat(Parameters(ChatParams { prompt: " ".into() })).await.unwrap_err();
    let token = server.test_token(Parameters(TokenParams { token: "synthetic\nsecret".into() })).await.unwrap_err();
    for error in [ids, dates, prompt, token] {
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    }
}

#[test]
fn upstream_router_and_serialization_failures_never_echo_details() {
    let upstream = internal("synthetic-secret https://private.example/?token=synthetic-secret body");
    assert_eq!(upstream.code, ErrorCode::INTERNAL_ERROR);
    assert!(!upstream.message.contains("synthetic-secret"));
    for original in [
        ErrorData::invalid_params("bad synthetic-secret", Some(json!({"synthetic-secret":"response-body"}))),
        ErrorData::internal_error("signed-url synthetic-secret", Some(json!(["synthetic-secret"]))),
    ] {
        let expected_code = original.code;
        let safe = sanitize_tool_error(original);
        assert_eq!(safe.code, expected_code);
        assert!(safe.data.is_none());
        assert!(!serde_json::to_string(&safe).unwrap().contains("synthetic-secret"));
    }
    struct BadSerialization;
    impl Serialize for BadSerialization {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("synthetic-secret"))
        }
    }
    let result = tool_json(BadSerialization);
    assert_eq!(result.is_error, Some(true));
    assert!(!serde_json::to_string(&result).unwrap().contains("synthetic-secret"));
}
