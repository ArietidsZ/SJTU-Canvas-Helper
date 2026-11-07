# MCP file explanation download boundary (stage 3 candidate)

> 阶段记录：本文保留该阶段的实现和验证范围。当前开发分支的完整使用说明与限制见 [MCP 使用指南](mcp.md)。

This change is isolated from existing GUI file explanation/chat methods and from
stage 2 listener lifecycle and account-owner transactions. It adds no dependency,
changes no persisted LLM configuration, and performs no real AI calls in tests.

## Entry and ownership

The MCP tool must accept only positive `course_id` and `file_id`. The App method
`explain_file_for_mcp(course_id, file_id)` resolves the ID from a fresh authenticated
Canvas course-file listing. Caller-provided File objects, local paths, and download
URLs must not be accepted by the tool. The App method rejects headless mode before
configuration, network, parser, or model access.

The App captures an immutable Canvas origin/token and the existing LLM snapshot
under the configuration-owner lock. This new private Client is never reconfigured.
No owner lock is held over HTTP, parsing, or model work. Owner checks run after
metadata lookup, after parsing immediately before model use, and before returning
the result. These checks do not atomically cancel a request that has already
started; the MCP lifecycle cancellation boundary remains important. No existing
snapshot/reconfigure/rollback method is replaced.

## Metadata and file download policy

- `locked` must be false, and server `locked_for_user` must be explicitly false.
  The new optional model field is backward compatible for ordinary consumers.
  An omitted or null flag is unknown and fails closed for MCP only. This is not a
  complete independent verification of every Canvas permission or scheduled lock.
- Synthetic external-file metadata is refused. Only PDF and DOCX display-name
  extensions are supported. The name is limited to 1024 UTF-8 bytes.
- Metadata size is at most 20 MiB. A supplied Content-Length must agree with the
  metadata size. Every streamed chunk is counted even without Content-Length;
  both overflow and truncated/mismatched files are rejected before parsing.
- URLs and every redirect are restricted to HTTPS, port 443, and exactly
  `oc.sjtu.edu.cn` or `jicanvas.com`. Userinfo (including empty userinfo), fragments,
  IP literals, local paths, other schemes, subdomains, nonstandard ports, and
  malformed/control-character inputs are rejected. URLs are limited to 16 KiB.
- At most five redirects are followed. Automatic redirects are disabled, so the
  next Location is validated before sending that request. A single 30-second
  budget covers the entire chain and body. The connection timeout is 10 seconds.
- A new client is used without the app cookie jar, Authorization, proxy credentials,
  or automatic Referer. Environment proxies and automatic decompression are
  disabled. Requests ask for identity encoding and reject other Content-Encoding.
  Redirect bodies and Set-Cookie are ignored. Only a complete HTTP 200 is accepted.
- Errors returned by metadata lookup, download, parsing, and model execution are
  static descriptions. Raw signed URLs, query strings, tokens, response bodies,
  and underlying transport/parser/model error strings are not returned or logged
  by this path.

Strict hosts intentionally reject third-party/CDN redirects, even if a legitimate
Canvas deployment uses them. No fallback to the general GUI downloader occurs.
The policy does not resolve and pin public IP addresses. OS DNS resolution and
TLS trust remain dependencies: these checks are not proof of DNS-rebinding or
all SSRF protection.

## Parsing and model use

Parsing uses the existing PDF/DOCX implementation on a blocking worker. A single
process-wide semaphore bounds concurrent MCP parser workers. The parser worker
keeps its permit if the awaiting request is cancelled or times out. The combined
slot wait and parse wait has a 15-second timeout. A running blocking parser cannot
be forcefully stopped by this timeout. A hung parser occupies the slot until it
returns, preventing additional MCP parses from starting.

Extracted text must be at most 128,000 Unicode scalar values before the owner guard
and LLM call. This is a post-extraction limit, not a parser allocation/decompression
limit or sandbox. A crafted PDF/DOCX may still consume excessive CPU or memory
inside the third-party parser. No claim of hard parser termination or resource
isolation is made. The configured LLM may be called only after these checks pass;
its existing behavior and settings are reused. Headless MCP does not enable AI.

The fresh metadata listing uses the existing authenticated Canvas API/pagination
implementation and a separate 30-second wait budget. Its response/pagination memory
is not governed by the file download's 20 MiB cap.

## Verification scope

`client/mcp_download.rs` deliberately depends only on reqwest, tokio, and std. Its
unit tests are offline policy/body-buffer/request-construction tests, not live
Canvas, DNS, TLS, redirect-server, or model tests. They cover exact hosts, schemes,
userinfo, ports, redirects and hop limit, absent/locked/synthetic metadata,
Content-Length and streamed-byte bounds, truncation, text bounds, and static errors.
An additional Client test rejects untrusted inputs before parsing, owner guard,
or model access. No test contacts a real model or Canvas account.

Full application compilation and end-to-end validation must still be recorded
separately. Focused helper compilation does not validate the App integration,
third-party document parsers, or full native application dependencies.
