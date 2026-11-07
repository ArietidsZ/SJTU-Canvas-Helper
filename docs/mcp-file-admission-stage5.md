# MCP file explanation admission

> 阶段记录：本文保留该阶段的实现和验证范围。当前开发分支的完整使用说明与限制见 [MCP 使用指南](mcp.md)。

This unpublished candidate limits the GUI MCP `explain_file` pipeline to one admitted request in the process. Headless MCP still denies this tool. Ordinary GUI `App::explain_file` and Client file-chat paths are unchanged.

## Reason for the limit

The previous implementation capped each download at 20 MiB and allowed one blocking parser, but fetched metadata and downloaded bodies before waiting for that parser slot. Concurrent requests in one real MCP HTTP session could therefore complete multiple downloads and retain their buffers while one parser ran. A controlled four-request reproduction used only four synthetic 17-byte documents; all four downloads completed while only one parser entered. No large memory allocation, real Canvas download or model request was used in the reproduction.

## Ownership and lifetime

The App validates its configuration owner and store, then obtains a non-queued admission permit before snapshot construction and metadata acquisition. Busy requests fail promptly without starting metadata/download work. The required typed permit cannot be cloned or constructed by unrelated callers. It travels into the Client and blocking parser, returns with the parsed result, and remains held through text validation, owner recheck and model completion.

A cancellation or parse timeout does not release admission while a blocking parser still owns it. Admission is released when that actual parser/result is dropped, rather than when only its async waiter disappears. Other error paths release it normally. The existing separate parser semaphore, owner checks, HTTP/file limits, error sanitization and account transactions remain intact.

The limit is intentionally fail-closed: a parser that never exits can keep MCP file explanation busy until process exit. It is not a parser sandbox, an allocation budget for metadata or decompression, or a guarantee that an already-issued remote model request has been cancelled. Other MCP tools and ordinary GUI operations are outside this admission gate.

## Verification boundary

Regression evidence uses exact App/Client code, the real MCP HTTP router/lifecycle and real ConfigStore, with explicit synthetic metadata/download/parser/model stages. It checks same-session concurrency, parser cancellation/timeout retention, model-stage retention, error release and existing owner isolation. Production guard/source tests serialize their global-permit access with a test-only lock. Full native App, real Canvas/TLS, actual PDF/DOCX parsing and model integration remain unverified; no new dependency, network installation, PR or publication is involved.
