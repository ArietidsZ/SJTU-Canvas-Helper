/** A backend observation, not a connectivity or authentication probe. */
export interface McpStatus {
  state: "stopped" | "running" | "stopping" | "exited";
  configured_enabled: boolean;
  configured_port: number;
  listening_port: number | null;
  headless: boolean;
}

export type McpStatusView =
  | { phase: "idle" | "loading" | "error" }
  | { phase: "ready"; status: McpStatus; observedAt: number };

function isPort(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 1 && value <= 65535;
}

/** Reject malformed observations rather than displaying an invented endpoint. */
export function parseMcpStatus(value: unknown): McpStatus {
  if (!value || typeof value !== "object") throw new Error("Invalid MCP status");
  const status = value as Record<string, unknown>;
  if (!["stopped", "running", "stopping", "exited"].includes(status.state as string)
    || typeof status.configured_enabled !== "boolean"
    // The legacy GUI may persist zero for an ephemeral bind. It is not a URL.
    || (status.configured_port !== 0 && !isPort(status.configured_port))
    || typeof status.headless !== "boolean"
    || (status.state === "running" ? !isPort(status.listening_port) : status.listening_port !== null)) {
    throw new Error("Invalid MCP status");
  }
  return {
    state: status.state as McpStatus["state"],
    configured_enabled: status.configured_enabled,
    configured_port: status.configured_port,
    listening_port: status.listening_port as number | null,
    headless: status.headless,
  };
}

/**
 * One view lifetime. Only the newest read in the same owner/session may publish.
 * Invalidating or disposing also revokes callbacks from already pending reads.
 */
export function createMcpStatusReader({ read, isCurrent, onChange }: {
  read: () => Promise<unknown>;
  isCurrent: () => boolean;
  onChange: (view: McpStatusView) => void;
}) {
  let generation = 0;
  let active = true;
  return {
    async refresh(): Promise<void> {
      if (!active || !isCurrent()) return;
      const request = ++generation;
      const owns = () => active && generation === request && isCurrent();
      onChange({ phase: "loading" });
      try {
        const value = await read();
        if (!owns()) return;
        const status = parseMcpStatus(value);
        onChange({ phase: "ready", status, observedAt: Date.now() });
      } catch {
        // Never surface raw native errors: they may contain account details.
        if (owns()) onChange({ phase: "error" });
      }
    },
    invalidate(): void {
      generation += 1;
      if (active) onChange({ phase: "idle" });
    },
    dispose(): void {
      active = false;
      generation += 1;
    },
  };
}
