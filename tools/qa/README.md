# tools/qa — 全自动化测试体系

## 一键
pwsh -File tools\qa\run-all.ps1 -Layer L0,L1,L2,L3
pwsh -File tools\qa\run-all.ps1 -Layer L6          # 真人多用户/混沌全场景
pwsh -File tools\qa\run-all.ps1 -Layer L5 -SoakClients 10 -SoakMinutes 10

## 分层
- L0: protocol / crypto / mls / storage 单元
- L1: server 集成（in-process）
- L2: client native 集成（真 WS）
- L3: Playwright e2e-standard（现有冒烟/加密/CRUD）
- L4: Playwright 屏幕共享 / 媒体（需 LiveKit）
- L5: 长稳 soak（10 客户端混合发消息/共享/断网）
- L6: Playwright e2e-multi（真人驱动、多用户、私密频道、好友/DM、附件、
      搜索、设置/权限、5 人长稳、顶替、并发加入、长断网恢复）

## 环境
- 后端 :18080, 前端 :5173, admin :18081（仅测试 token）
- `pwsh -File tools\qa\start-env.ps1 [-Force] [-WithMedia]`
- `pwsh -File tools\qa\stop-env.ps1`
- **QA 独立数据库**：`tools/qa/.run/qa.db`（`-Force` 会删除重建）
  - 不会污染开发库 `kaede.db`
  - `-Force` 同时保证“第一个注册用户 = op5”这类场景可复现
- 测试模式：`KAEDE_TEST_MODE=1`，仅 dev-mode 生效，生产构建零影响
  （绕过 per-IP 注册、publish_kp、消息限流，避免多人用例被限流误伤）
- admin 面（:18081）用于测试编排 op5/op4（Playwright 里的 `ensureOp5`）

## 报告
- `tools/qa/.run/reports/summary.json`
- 分层日志：`L0.log` … `L6.log`
- 长稳明细：`L5.json` / `L5-samples.csv`
- Playwright：`apps/web/test-results/`（失败截图/trace）

## 注意
- L6 默认串行（`KAEDE_MULTI_WORKERS=1`）。多 worker 并行会同时 churn
  主群/频道群的权威成员，属于测试 artifact；压测并发请显式设 4。
- 改 `kaede-client` 后必须先 `npm run build:wasm`，否则跑的是旧 wasm。
