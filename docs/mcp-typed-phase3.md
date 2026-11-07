# Typed MCP tools, phase 3 candidate

> 阶段记录：本文保留该阶段的实现和验证范围。当前开发分支的完整使用说明与限制见 [MCP 使用指南](mcp.md)。

This incremental overlay is based on the integrated phase 2 snapshot. It changes
only MCP tool contracts, tool error handling and focused tests. The separate
phase 3 download/App overlay is required for `App::explain_file_for_mcp(course_id,
file_id)`. A matching calendar query-encoding change preserves RFC 3339 positive
offsets on the wire. This document is not evidence of a full app build, native
release validation, or live Canvas/model-provider interoperability.

## Tool surface and policy

All 24 existing GUI tool names remain. The 21 headless tools remain the same:
headless discovery and invocation exclude `chat`, `explain_file` and `test_token`.
There is no AI opt-in flag and no new syllabus or other tool. The GUI still
exposes the three legacy tools, so callers must review their permissions.

All tools now advertise object input schemas. Required arguments are required,
unknown fields are rejected, and the four zero-input tools accept an empty
object rather than ignoring arbitrary arguments. Clients may continue to omit
the arguments member for a zero-input call if their MCP transport represents it
as an empty argument map.

- IDs are positive signed 64-bit integers. Numeric strings, fractional numbers,
  zero, negatives and values above `i64::MAX` are rejected without narrowing.
- `student_ids` contains 1 to 100 positive IDs. `context_codes` contains 1 to 100
  `course_<positive signed 64-bit ID>` strings. These are local MCP request
  limits, not claims about Canvas service limits, pagination or output size.
  Duplicate IDs/codes are not removed.
- Calendar endpoints must both be exact `YYYY-MM-DD` dates, or both RFC 3339
  timestamps with explicit offsets. Dates must exist and start must not exceed
  end. Timestamp ordering uses instants, including offsets; date-only endpoints
  are not assigned an invented UTC midnight. Mixed kinds are rejected. Validated
  text passes through unchanged. Canvas owns endpoint inclusion/timezone rules.
- `chat.prompt` must be nonblank and at most 32,000 Unicode characters. This is
  a local character limit, not a model token budget.
- `test_token.token` must be nonempty with no whitespace or control characters.
  GUI MCP sends it to the configured Canvas service. This is credential probing,
  not a general credential store or a safe default for arbitrary secrets.

`explain_file` intentionally changes from a caller-supplied `file` object to the
required `course_id` and `file_id` fields. Old `file`, `url` and `path` arguments
are rejected. The tool passes only the two IDs to the dedicated App helper,
which owns fresh metadata resolution and download safety. The tool does not
call the GUI's arbitrary-file explanation API or duplicate metadata lookup.
Do not invoke the standalone typed overlay without the matching App/download
implementation. Locked/oversized files and untrusted fetches remain the helper's
responsibility, not a schema guarantee.

## Safety and errors

Canvas read tools advertise read-only, nondestructive, idempotent hints. Their
operations can contact Canvas and may use local application state; these hints
are not a promise of no disk/cache effects or no network access. The three
sensitive GUI tools deliberately do not claim read-only or idempotent behavior.
All tools advertise open-world interaction. AI descriptions warn that prompts
or file content go to the configured external model provider and may cost money.
Annotations are hints, never authorization or enforcement mechanisms.

Upstream failures and serialization errors no longer expose raw details. The
`ServerHandler::call_tool` adapter also replaces router/serde error messages and
clears error data while retaining the JSON-RPC error code. This prevents malformed
arguments and unknown field names from reflecting credential-like text. Clients
receive a stable instruction to check the schema/configuration rather than raw
provider errors. Semantic validators use static messages in direct calls.
Successful tool results are not redacted: authorized Canvas data may contain
personal details or signed file URLs. This change is an error boundary, not a
general output data-loss-prevention filter.

The existing `CancellableServer` remains responsible for admission, cancellation
and draining. The typed adapter does not introduce tasks or application locks.
The owner, store, transaction, headless startup and lifecycle modules are unchanged
in this overlay. Stopping cannot revoke already-sent remote work.

## Focused verification

`src-tauri/src/mcp/typed_tools_tests.rs` contains schema/annotation contracts,
required and unknown fields for every parameter type, positive i64 boundaries,
vector limits, strict date/offset ordering and mixed-kind rejection, Unicode
prompt sizing, token validation, headless direct guards, pre-App semantic
rejection, and sanitized error/serialization tests. These checks must not use
real accounts, real Canvas traffic, credential probes or AI calls.

When the candidate is integrated with the matching App helper, the intended
focused command on a compatible pre-provisioned toolchain is:

```sh
cd src-tauri
cargo test --locked --offline mcp::typed_tools_tests::
```

The cloud editing environment has no `cargo`, `rustc` or `rustfmt`. It ran only
cheap source-consistency/scope checks recorded in the delivery manifest, not
Rust tests. Independent offline validation must use exact cached dependency
versions and the final integrated sources. A stub-App rmcp harness can test
schemas, parsing, invalid-call rejection and lifecycle boundaries without
accessing accounts, but it cannot prove native Tauri integration or successful
Canvas/AI behavior. Full checks, packaging and native runtime validation remain
separate gates. No dependency installs, downloads, real service calls, commits,
pushes or PRs were performed for this overlay.
