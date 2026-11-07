import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";
import { Alert, Box, Button, CircularProgress, Stack, Typography } from "@mui/material";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { isConfigOwnerCurrent } from "../lib/config";
import { mcpEndpoint } from "../lib/mcp_connection";
import { createMcpStatusReader, type McpStatus, type McpStatusView } from "../lib/mcp_status";
import type { AppConfig } from "../lib/model";
import { isSessionCurrent, subscribeSession, useSessionEpoch } from "../lib/session";

const stateLabels: Record<McpStatus["state"], string> = {
  stopped: "未运行",
  running: "运行中（监听任务）",
  stopping: "正在停止",
  exited: "监听任务已退出",
};

export default function McpStatusPanel({ savedConfig }: { savedConfig: AppConfig | null }) {
  const session = useSessionEpoch();
  const readerRef = useRef<ReturnType<typeof createMcpStatusReader> | null>(null);
  const [observation, setObservation] = useState<{
    config: AppConfig; session: number; view: McpStatusView;
  } | null>(null);
  const current = !!savedConfig && isConfigOwnerCurrent(savedConfig);

  useEffect(() => {
    if (!savedConfig?.__config_owner || !isConfigOwnerCurrent(savedConfig)) return;
    const expectedOwner = savedConfig.__config_owner;
    const reader = createMcpStatusReader({
      read: () => invoke("get_mcp_status", { expectedOwner }),
      isCurrent: () => isSessionCurrent(session) && isConfigOwnerCurrent(savedConfig),
      onChange: (view) => setObservation({ config: savedConfig, session, view }),
    });
    readerRef.current = reader;
    const unsubscribe = subscribeSession(() => reader.invalidate());
    void reader.refresh();
    return () => {
      reader.dispose();
      unsubscribe();
      if (readerRef.current === reader) readerRef.current = null;
    };
  }, [savedConfig, session]);

  // Render-time ownership also hides an old observation before effect cleanup.
  const view = current && observation?.config === savedConfig && observation.session === session
    ? observation.view : null;

  return (
    <Box component="section" aria-label="MCP 运行状态快照"
      sx={{ p: 2, borderRadius: "10px", border: "1px solid", borderColor: "divider" }}>
      <Stack spacing={1}>
        <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
          <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>MCP 运行状态快照</Typography>
          <Button size="small" aria-label="刷新 MCP 状态" disabled={!current}
            startIcon={view?.phase === "loading" ? <CircularProgress size={14} /> : <RefreshRoundedIcon />}
            onClick={() => void readerRef.current?.refresh()}>刷新状态</Button>
        </Stack>
        <Box role="status" aria-live="polite" aria-busy={view?.phase === "loading"}>
          {!current ? (
            <Typography variant="body2" color="text.secondary">
              {savedConfig ? "当前配置会话已失效，请重新加载设置后刷新。" : "等待读取已保存的配置。"}
            </Typography>
          ) : view?.phase === "ready" ? (
            <Stack spacing={0.5}>
              <Typography variant="body2">运行状态：{stateLabels[view.status.state]}</Typography>
              <Typography variant="body2">已保存设置：{view.status.configured_enabled ? "已启用" : "已禁用"} · 端口 {view.status.configured_port}{view.status.configured_port === 0 ? "（未指定固定端口）" : ""}</Typography>
              <Typography variant="body2">监听地址（快照）：{view.status.listening_port === null ? "无" : mcpEndpoint(view.status.listening_port)}</Typography>
              <Typography variant="body2">运行模式：{view.status.headless ? "无界面" : "桌面"}</Typography>
              <Typography variant="caption" color="text.secondary">读取时间：{new Date(view.observedAt).toLocaleTimeString()}</Typography>
            </Stack>
          ) : view?.phase === "error" ? (
            <Alert severity="warning">无法读取 MCP 状态，请刷新重试。</Alert>
          ) : (
            <Typography variant="body2" color="text.secondary">{view?.phase === "loading" ? "正在读取状态快照…" : "暂无状态快照。"}</Typography>
          )}
        </Box>
        <Typography variant="caption" color="text.secondary">
          状态仅为读取时的快照，不会自动持续更新。“运行中”仅表示应用跟踪的监听任务仍在运行，不是网络可达性或身份鉴权测试。已保存设置与实际监听端口分别展示，不受未保存的表单端口影响；下方连接说明仍按表单草稿生成。
        </Typography>
      </Stack>
    </Box>
  );
}
