// Dependency-free focused regression: node --test script/test-mcp-connection.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import test from "node:test";

const source = readFileSync(new URL("../src/lib/mcp_connection.ts", import.meta.url), "utf8");
const { mcpEndpoint, mcpConnectionGuide, mcpClientConfig } = await import(
  `data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source)).toString("base64")}`
);
const settings = readFileSync(new URL("../src/page/settings.tsx", import.meta.url), "utf8");

test("default, custom and boundary ports use the exact Streamable HTTP route", () => {
  assert.equal(mcpEndpoint(), "http://127.0.0.1:3100/mcp");
  for (const port of [1, 3100, 4310, 65535]) {
    assert.equal(mcpEndpoint(port), `http://127.0.0.1:${port}/mcp`);
    assert.ok(mcpConnectionGuide(port).includes(`通用连接 URL：http://127.0.0.1:${port}/mcp`));
  }
});

test("invalid ports do not advertise a different or malformed endpoint", () => {
  for (const port of [0, -1, 65536, 3.14, NaN, Infinity, -Infinity, null, "3100"]) {
    assert.equal(mcpEndpoint(port), null);
    assert.match(mcpConnectionGuide(port), /端口无效/);
    assert.doesNotMatch(mcpConnectionGuide(port), /http|mcpServers/);
  }
});

test("generic and OpenCode snippets are exact JSON with one shared URL", () => {
  const guide = mcpConnectionGuide(4310);
  const snippets = guide.match(/^\{\n[\s\S]*?^\}/gm).map((json) => JSON.parse(json));
  const url = "http://127.0.0.1:4310/mcp";
  assert.deepEqual(snippets, [
    { mcpServers: { "sjtu-canvas": { url } } },
    { mcp: { "sjtu-canvas": { type: "remote", url, enabled: true } } },
  ]);
});

test("connection guide avoids legacy transport and unimplemented CLI claims", () => {
  const text = mcpConnectionGuide() + settings;
  assert.doesNotMatch(text, /["']?type["']?\s*:\s*["']sse["']|--mcp/);
  assert.doesNotMatch(text, /即可自动接入|无需额外鉴权/);
  assert.match(mcpConnectionGuide(), /同机其他进程/);
  assert.match(mcpConnectionGuide(), /不代表服务已启动/);
  assert.match(mcpConnectionGuide(), /外部模型服务.*费用/);
  assert.doesNotMatch(mcpConnectionGuide(), /默认关闭|allow.ai/);
});

test("settings display and clipboard use the same pure guide result", () => {
  assert.match(settings, /return mcpConnectionGuide\(formData\.mcp_port\);/);
  assert.match(settings, /copyMcpText\(mcpConnectionText\)/);
  assert.match(settings, /value=\{mcpConnectionText\}/);
});

test("clipboard client configurations are standalone JSON and match the guide", () => {
  for (const port of [1, 3100, 4310, 65535]) {
    const url = mcpEndpoint(port);
    const generic = mcpClientConfig(port, "generic");
    const opencode = mcpClientConfig(port, "opencode");
    assert.deepEqual(JSON.parse(generic), { mcpServers: { "sjtu-canvas": { url } } });
    assert.deepEqual(JSON.parse(opencode), { mcp: { "sjtu-canvas": { type: "remote", url, enabled: true } } });
    assert.ok(mcpConnectionGuide(port).includes(generic));
    assert.ok(mcpConnectionGuide(port).includes(opencode));
  }
});

test("clipboard configurations reject invalid ports and unknown client formats", () => {
  for (const port of [0, -1, 65536, 3.14, NaN, Infinity, -Infinity, null, "3100"]) {
    assert.equal(mcpClientConfig(port, "generic"), null);
    assert.equal(mcpClientConfig(port, "opencode"), null);
  }
  assert.equal(mcpClientConfig(3100, "unsupported"), null);
});
