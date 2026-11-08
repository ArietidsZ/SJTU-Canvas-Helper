# MCP 初始化中断验证

在 23:00 reviewed 会话准入、SSE 生命周期和 Unix 信号组合候选上，补充两个不同于既有“未读完 HTTP body”场景的边界：请求体已读完、rmcp 已创建 session，但应用 initialize handler 仍等待锁时发生中断。

1. 客户端在取得 session ID 前关闭真实 loopback TCP 连接。测试观察 HTTP future 的析构，确认请求已取消后才释放应用锁；session 自动删除，随后新客户端仍能初始化并 DELETE 清理。
2. 相同阶段发生服务停机。第一次 stop 因人为持锁被外层超时中断；取消后 pending session 被删除，重试 stop 成功，客户端收到 503，监听端口可重新绑定。

HTTP 观察器仅在测试 router 外记录请求 future 的完成/取消，不改变生产中间件、依赖、准入权限或会话管理。应用写锁是显式 fixture，用于稳定停在初始化处理中，而不是实际远端请求。

## 时序与边界

初始探索测试在断线后立刻释放锁，存在“initialize response 与 HTTP 断线检测竞争”的假失败；50ms 观察实验通过后，正式测试改为 oneshot 完成/取消屏障，不依赖固定 sleep。初始失败不是已确认的生产 bug。

此测试只要求 HTTP 初始化 future 已取消后的资源回收。如果服务已将初始化响应交给 HTTP 层，而客户端随后离线，rmcp 仍允许该 session 在默认 5 分钟 idle timeout 前保留，支持客户端重连；本轮没有把这种保留误判成泄漏，也未实际等待 5 分钟验收。

测试不访问真实账号、Canvas、模型或 GUI，不等于完整 App/Tauri 构建或真实客户端验收。本轮没有生产代码修复；只整合已审查候选并补充中断回归。最终结果和复现命令见交付目录 REPORT.md。
