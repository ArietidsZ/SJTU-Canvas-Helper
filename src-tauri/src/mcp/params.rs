//! MCP-only input contracts. Limits here bound individual tool requests; they
//! are not claims about Canvas API limits or the size of a paginated response.
use rmcp::{schemars, ErrorData};
use serde::Deserialize;

use super::invalid;

pub(super) const MAX_VECTOR_ITEMS: usize = 100;
pub(super) const MAX_PROMPT_CHARS: usize = 32_000;

/// Canvas IDs are positive signed 64-bit integers, never truncated to 32 bits.
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "i64")]
#[schemars(!try_from)]
pub(super) struct CanvasId(#[schemars(range(min = 1))] i64);

impl TryFrom<i64> for CanvasId {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value > 0 {
            Ok(Self(value))
        } else {
            Err("Canvas IDs must be positive signed 64-bit integers")
        }
    }
}

impl CanvasId {
    pub(super) fn get(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct EmptyParams {}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CourseParams {
    /// Positive signed 64-bit Canvas course ID from list_courses.
    pub(super) course_id: CanvasId,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct FolderParams {
    /// Positive signed 64-bit Canvas folder ID.
    pub(super) folder_id: CanvasId,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct AssignmentParams {
    pub(super) course_id: CanvasId,
    pub(super) assignment_id: CanvasId,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct StudentSubmissionParams {
    pub(super) course_id: CanvasId,
    pub(super) assignment_id: CanvasId,
    pub(super) student_id: CanvasId,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct UserSubmissionsParams {
    pub(super) course_id: CanvasId,
    /// MCP request limit: 1 to 100 positive signed 64-bit student IDs.
    #[schemars(length(min = 1, max = 100))]
    pub(super) student_ids: Vec<CanvasId>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct DiscussionParams {
    pub(super) course_id: CanvasId,
    pub(super) topic_id: CanvasId,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CalendarParams {
    /// MCP request limit: 1 to 100 course_<positive signed 64-bit ID> codes.
    #[schemars(length(min = 1, max = 100))]
    pub(super) context_codes: Vec<String>,
    /// YYYY-MM-DD or RFC 3339 with an explicit UTC offset. Both endpoints must
    /// use the same kind. Date strings are passed to Canvas without alteration.
    pub(super) start_date: String,
    /// Same kind as start_date and not earlier. Timestamp ordering compares
    /// instants, not text. Canvas determines endpoint inclusion/timezone rules.
    pub(super) end_date: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ChatParams {
    /// Nonblank text sent to the configured external model provider. MCP
    /// request limit: 1 to 32000 Unicode characters. May incur API charges.
    #[schemars(length(min = 1, max = 32000))]
    pub(super) prompt: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ExplainFileParams {
    pub(super) course_id: CanvasId,
    /// Canvas file ID resolved from server-fetched metadata for this course.
    /// Caller-supplied file objects, paths, and URLs are not accepted.
    pub(super) file_id: CanvasId,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct TokenParams {
    /// Credential sent to the configured Canvas service. Nonempty, without
    /// whitespace/control characters. Available only through GUI MCP.
    #[schemars(length(min = 1))]
    pub(super) token: String,
}

pub(super) fn validate_student_ids(ids: &[CanvasId]) -> Result<(), ErrorData> {
    if ids.is_empty() || ids.len() > MAX_VECTOR_ITEMS {
        return Err(invalid("student_ids must contain 1 to 100 IDs"));
    }
    Ok(())
}

fn course_context(code: &str) -> bool {
    code.strip_prefix("course_").is_some_and(|id| {
        !id.is_empty()
            && id.bytes().all(|c| c.is_ascii_digit())
            && id.parse::<i64>().is_ok_and(|id| id > 0)
    })
}

fn date_only(value: &str) -> Option<chrono::NaiveDate> {
    // chrono's parser accepts less strictly padded forms. The MCP contract is
    // exactly YYYY-MM-DD, with a real Gregorian calendar date.
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes.iter().enumerate().all(|(index, c)| {
            matches!(index, 4 | 7) || c.is_ascii_digit()
        })
    {
        return None;
    }
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

pub(super) fn validate_calendar(params: &CalendarParams) -> Result<(), ErrorData> {
    if params.context_codes.is_empty()
        || params.context_codes.len() > MAX_VECTOR_ITEMS
        || !params.context_codes.iter().all(|code| course_context(code))
    {
        return Err(invalid(
            "context_codes must contain 1 to 100 course_<positive signed 64-bit ID> values",
        ));
    }

    // Do not invent midnight UTC for a date-only endpoint or compare an offset
    // timestamp with a local calendar day. Mixed kinds are rejected explicitly.
    let ordered = match (date_only(&params.start_date), date_only(&params.end_date)) {
        (Some(start), Some(end)) => start <= end,
        (None, None) => match (
            chrono::DateTime::parse_from_rfc3339(&params.start_date),
            chrono::DateTime::parse_from_rfc3339(&params.end_date),
        ) {
            (Ok(start), Ok(end)) => start <= end,
            _ => false,
        },
        _ => false,
    };
    if !ordered {
        return Err(invalid(
            "Use two valid YYYY-MM-DD dates or two RFC 3339 timestamps with start_date <= end_date",
        ));
    }
    Ok(())
}

pub(super) fn validate_prompt(prompt: &str) -> Result<(), ErrorData> {
    if prompt.trim().is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(invalid("prompt must contain 1 to 32000 characters and cannot be blank"));
    }
    Ok(())
}

pub(super) fn validate_token(token: &str) -> Result<(), ErrorData> {
    if token.is_empty() || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid("token must be nonempty without whitespace or control characters"));
    }
    Ok(())
}
