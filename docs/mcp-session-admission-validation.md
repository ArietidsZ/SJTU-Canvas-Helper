# MCP 首消息准入与会话隔离验证

## 修复

长期运行的 MCP 服务会收到配置错误客户端的重试。rmcp 1.7.0 在无有效文本 `mcp-session-id` 时先创建会话，再检查首消息是否为 `initialize`。对于 `ping`、`tools/list` 和通知，HTTP 返回 422，但会话句柄仍留在 session map，重复请求会累积。

入口现使用与 rmcp 相同的 `ClientJsonRpcMessage` 类型，在已限制为 1 MiB 的 POST 请求体上检查首消息。只有 JSON/Accept 协商有效、且没有有效文本 session header 的非 initialize 消息会提前被拒绝。合法初始化和现有会话保持原路径；无效 JSON、406 和 415 仍由原传输层处理。修复不调用应用工具、不访问账户、不增加依赖。

## 新增回归

- 无会话的 ping、tools/list、initialized/cancelled 通知及其重试不分配残留句柄。
- 非文本 session header 不能绕过首消息检查。
- 无效首消息或 JSON 不影响已有会话。
- DELETE 只删除对应会话、结束对应 SSE；另一客户端仍可 ping，保持 SSE，旧会话 GET/POST 返回 404，重复 DELETE 无副作用，新初始化得到不同 ID。
- 不支持的协议版本在 GET/POST/DELETE 返回 400，不损坏现有会话。
- 406/415 协商错误保持，之后仍可正常初始化。

## 验证边界

这些测试运行生产 MCP 路由、会话管理、HTTP/SSE 和依赖 rmcp 1.7.0；App 构造及外部应用方法为原窄测试 fixture，测试不调用真实 Canvas、账号、模型或 GUI。它们不是完整 App/Tauri、原生安装包、真实远端请求或跨平台验收。

基础为 b8edd6a。此前会话生命周期与 Unix 信号回归可与本轮补丁组合；独立补丁只包含本轮首消息守卫、6项新测试、测试声明和本文件。可复现日志、反向对照、来源哈希与最终计数见交付包。

## 独立兼容性审查补充（2026-11-07 23:00 UTC 任务）

新增回归先在上一版守卫上复现三项差异：`POST /health` 被改为 422 而非路由的 405；不可信 Host/Origin 被改为 422 而非传输层的 403/400；422 的初始化说明变为空体。修订后只在没有显式路由、且符合当前 loopback Host/Origin 策略时提前拒绝，保留 rmcp 的 `Unexpected message, expect initialize request` 响应。Host/Origin 的最终准入决定仍由 rmcp 作出。

新增覆盖包括 JSON-RPC 语法与消息形态、未识别方法/通知、响应和错误消息、不同初始化 ID、协议版本协商、非文本及空/未知 session header、已有会话收到客户端回复、多个 Accept 字段、MIME 参数及大小写、等价 loopback authority、根路径及 fallback 路径。另有直接 URI-authority/header 优先级测试，不能把该单元测试称为端到端 HTTP/2 验证。

HTTP header 测试刻意保持 rmcp 1.7.0 当前行为：它使用 Accept 子串和 Content-Type 前缀匹配，未完整实现 MIME quality 协商。JSON-RPC 边界也以该版本的 typed parser 为兼容性基准，包括自定义消息 fallback，而非声称修正了依赖的全部协议行为。应用允许的 Host/Origin 集合改变时，应同步更新 `uses_allowed_loopback_authority` 及其回归测试。
