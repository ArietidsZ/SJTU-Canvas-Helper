// Offline tests of exact production parsing/request-gate source, plus source checks.
// These do not render React or replace the settings integration suite.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import test from "node:test";

const readSource = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
const { parseMcpStatus, createMcpStatusReader } = await import(
  `data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(readSource("../src/lib/mcp_status.ts"))).toString("base64")}`
);
const panel = readSource("../src/components/mcp_status_panel.tsx");
const settings = readSource("../src/page/settings.tsx");
const snapshot = (overrides = {}) => ({ state: "stopped", configured_enabled: false,
  configured_port: 3100, listening_port: null, headless: false, ...overrides });
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

test("all four states preserve saved enablement and headless mode independently", () => {
  for (const state of ["stopped", "running", "stopping", "exited"]) {
    for (const configured_enabled of [true, false]) for (const headless of [true, false]) {
      const value = snapshot({ state, configured_enabled, headless, listening_port: state === "running" ? 4310 : null });
      assert.deepEqual(parseMcpStatus(value), value);
    }
  }
});

test("zero is allowed only as the legacy saved port, never a listening endpoint", () => {
  for (const configured_port of [0, 1, 65535]) {
    const value = snapshot({ state: "running", configured_port, listening_port: 4310 });
    assert.deepEqual(parseMcpStatus(value), value);
  }
  assert.throws(() => parseMcpStatus(snapshot({ state: "running", listening_port: 0 })));
  assert.match(panel, /未指定固定端口/);
});

test("rejects malformed states, flags, ports, and non-running listening addresses", () => {
  const invalid = [undefined, null, [], {}, "running", snapshot({ state: "unknown" }),
    snapshot({ configured_enabled: 1 }), snapshot({ headless: "false" }), snapshot({ state: "running" })];
  for (const port of [-1, 65536, 3.14, NaN, Infinity, -Infinity, null, "3100", undefined]) {
    invalid.push(snapshot({ configured_port: port }));
    invalid.push(snapshot({ state: "running", listening_port: port }));
  }
  for (const state of ["stopped", "stopping", "exited"]) {
    invalid.push(snapshot({ state, listening_port: 4310 }));
  }
  for (const value of invalid) assert.throws(() => parseMcpStatus(value), /Invalid MCP status/);
});

test("projects only the public status fields, never unexpected owner or secret fields", () => {
  assert.deepEqual(parseMcpStatus({ ...snapshot(), owner: "PRIVATE_OWNER", token: "PRIVATE_TOKEN" }), snapshot());
});

test("publishes loading then a timestamped snapshot, not a live status", async () => {
  const changes = []; const before = Date.now();
  const reader = createMcpStatusReader({ read: async () => snapshot(), isCurrent: () => true, onChange: value => changes.push(value) });
  await reader.refresh();
  assert.deepEqual(changes[0], { phase: "loading" });
  assert.equal(changes[1].phase, "ready");
  assert.deepEqual(changes[1].status, snapshot());
  assert.ok(changes[1].observedAt >= before && changes[1].observedAt <= Date.now());
  assert.equal(changes.length, 2);
});

test("a current failure hides old state, redacts native details, and permits retry", async () => {
  const changes = []; let fail = true;
  const reader = createMcpStatusReader({ read: async () => {
    if (fail) throw new Error("PRIVATE_NATIVE_ERROR");
    return snapshot();
  }, isCurrent: () => true, onChange: value => changes.push(value) });
  await reader.refresh();
  assert.deepEqual(changes, [{ phase: "loading" }, { phase: "error" }]);
  fail = false; await reader.refresh();
  assert.equal(changes.at(-1).phase, "ready");
  assert.doesNotMatch(JSON.stringify(changes), /PRIVATE_NATIVE_ERROR/);
});

test("a malformed current response is a generic error rather than stopped or running", async () => {
  const changes = [];
  const reader = createMcpStatusReader({ read: async () => ({ state: "running" }), isCurrent: () => true, onChange: value => changes.push(value) });
  await reader.refresh();
  assert.deepEqual(changes, [{ phase: "loading" }, { phase: "error" }]);
});

for (const outcome of ["resolve", "reject"]) {
  test(`overlapping refresh discards the older ${outcome} after the newer result`, async () => {
    const changes = []; const old = deferred(); const fresh = deferred(); let count = 0;
    const reader = createMcpStatusReader({ read: () => ++count === 1 ? old.promise : fresh.promise,
      isCurrent: () => true, onChange: value => changes.push(value) });
    const pendingOld = reader.refresh(); const pendingFresh = reader.refresh();
    fresh.resolve(snapshot({ state: "running", listening_port: 4310 })); await pendingFresh;
    const accepted = [...changes];
    if (outcome === "resolve") old.resolve(snapshot({ state: "exited" }));
    else old.reject(new Error("PRIVATE_OLD_ERROR"));
    await pendingOld;
    assert.deepEqual(changes, accepted);
    assert.equal(changes.at(-1).status.listening_port, 4310);
  });

  for (const boundary of ["session", "owner", "invalidate", "dispose"]) {
    test(`${boundary} change drops pending ${outcome} and cannot start a stale read`, async () => {
      const changes = []; const pending = deferred(); let session = 0; let owner = "a"; let calls = 0;
      const reader = createMcpStatusReader({ read: () => { calls++; return pending.promise; },
        isCurrent: () => session === 0 && owner === "a", onChange: value => changes.push(value) });
      const done = reader.refresh();
      if (boundary === "session") session++;
      if (boundary === "owner") owner = "b";
      if (boundary === "invalidate") { session++; reader.invalidate(); }
      if (boundary === "dispose") reader.dispose();
      const accepted = [...changes];
      if (outcome === "resolve") pending.resolve(snapshot({ state: "running", listening_port: 4310 }));
      else pending.reject(new Error("PRIVATE_OLD_ERROR"));
      await done; await reader.refresh();
      assert.deepEqual(changes, accepted);
      assert.equal(calls, 1);
      if (boundary === "invalidate") assert.equal(changes.at(-1).phase, "idle");
    });
  }
}

test("invalidation revokes a pending read even when the owner predicate remains true", async () => {
  const changes = []; const pending = deferred();
  const reader = createMcpStatusReader({ read: () => pending.promise, isCurrent: () => true, onChange: value => changes.push(value) });
  const done = reader.refresh(); reader.invalidate();
  pending.resolve(snapshot()); await done;
  assert.deepEqual(changes, [{ phase: "loading" }, { phase: "idle" }]);
  reader.dispose(); reader.invalidate(); await reader.refresh();
  assert.equal(changes.length, 2);
});

test("source wiring reads only status under owner/session/lifetime guards with no polling", () => {
  assert.deepEqual([...panel.matchAll(/invoke\("([^"]+)"/g)].map(match => match[1]), ["get_mcp_status"]);
  assert.match(panel, /invoke\("get_mcp_status", \{ expectedOwner \}\)/);
  assert.match(panel, /isSessionCurrent\(session\) && isConfigOwnerCurrent\(savedConfig\)/);
  assert.match(panel, /subscribeSession\(\(\) => reader\.invalidate\(\)\)/);
  assert.match(panel, /reader\.dispose\(\);\s+unsubscribe\(\);/);
  assert.match(panel, /\}, \[savedConfig, session\]\)/);
  assert.match(panel, /current && observation\?\.config === savedConfig && observation\.session === session/);
  assert.match(panel, /readerRef\.current\?\.refresh\(\)/);
  assert.doesNotMatch(panel, /setInterval|setTimeout|saveConfig|fetch\(/);
  assert.match(panel, /不是网络可达性或身份鉴权测试/);
  assert.match(panel, /状态仅为读取时的快照/);
});

test("source wiring updates saved config only on initialization or accepted save, not draft edits", () => {
  assert.deepEqual([...settings.matchAll(/setSavedConfig\(([^)]+)\)/g)].map(match => match[1]), ["saved", "normalizedConfig"]);
  assert.match(settings, /if \(isConfigOwnerCurrent\(saved\)\) setSavedConfig\(saved\)/);
  assert.match(settings, /<McpStatusPanel savedConfig=\{savedConfig\} \/>/);
  assert.match(settings, /return mcpConnectionGuide\(formData\.mcp_port\)/);
  assert.match(settings, /copyMcpText\(mcpConnectionText\)/);
  const bytes = readFileSync(new URL("../src/page/settings.tsx", import.meta.url));
  assert.equal(bytes.toString().replaceAll("\r\n", "").includes("\n"), false, "Preserve original settings CRLF");
});
