/** A configured endpoint, not a claim about runtime availability. */
export function mcpEndpoint(port = 3100): string | null {
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  return `http://127.0.0.1:${port}/mcp`;
}

/** One source for the displayed guide and clipboard contents. */
export function mcpConnectionGuide(port = 3100): string {
  const url = mcpEndpoint(port);
  if (!url) return "MCP 端口无效：请输入 1–65535 之间的整数并保存设置。";
  return [
    "Canvas Helper 使用本机 Streamable HTTP MCP。",
    "请先配置 Canvas Token，在应用内启用 MCP 并保存设置，保持应用运行。",
    "以下地址按设置端口生成，不代表服务已启动或连接已验证。",
    `通用连接 URL：${url}`,
    "通用 URL 配置片段，需按客户端支持的格式调整：",
    JSON.stringify({ mcpServers: { "sjtu-canvas": { url } } }, null, 2),
    "",
    "OpenCode 配置片段（合并到现有配置的 mcp 字段）：",
    JSON.stringify({ mcp: { "sjtu-canvas": { type: "remote", url, enabled: true } } }, null, 2),
    "",
    "客户端需支持 Streamable HTTP；仅粘贴说明不会自动建立连接。",
    "连接后可调用 list_courses 查看课程，再查询作业截止时间。",
    "现有 AI 工具可能将内容发送至已配置的外部模型服务并产生费用，调用前请确认客户端权限与模型配置。",
    "仅供可信本机环境使用。本机监听不是身份鉴权，同机其他进程也可能访问 Canvas 数据，请勿通过代理公开。",
  ].join("\n");
}
