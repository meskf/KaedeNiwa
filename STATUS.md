KaedeNiwa STATUS
================

Last updated: 2026-10-25 (P0-1 + P0-2 + 网络健壮性完成, e2e 11/11 green)

Commit history (this session)
-----------------------------
  5c4eee3  P0-1: strict welcome verification + restore spawn_test_server
  2fcfb47  fix(rate-limit): dev-mode short-circuit for check_rate_scoped / check_rate_key
  224ff32  P0-2 r2: per-channel commit gap buffering + drain + reconnect backfill
  4c65547  P0-2 r1: mls_commits channel_id dimension + protocol ReportEpoch/MlsCommit.channel_id
  3550cb7  fix(wasm): rebuild with correct out-dir + fix build:wasm script; test-10 green
  8a68d79  chore: wasm rebuild wired into start-all + run-all-tests; STATUS.md
  8a544a8  fix(run-all-tests): trailing comma after test-9 entry
  9c9fa70  fix(run-all-tests): wasm rebuild path uses $PSScriptRoot directly

Completed
---------
  [x] P0-1 Welcome 强验证（服务端 authority gate + 客户端 strict verify + 时序容忍）
  [x] P0-2 频道 Commit 补发
      - mls_commits (channel_id, epoch) 复合主键 migration
      - 服务端 ChannelCommit 落库（INSERT OR IGNORE）
      - 服务端 ReportEpoch 支持 channel_id 分支 + 日志
      - 客户端 report_channel_epoch + backfill_all_channel_epochs
      - ChannelSync.epoch gap 检测 + per-channel buffered drain
      - test-10-commit-replay 验证 reload + backfill 全链路
  [x] dev-mode 限流短路（honor "dev 无限流" contract）
  [x] spawn_test_server 恢复（cleanup regression 修复）
  [x] cargo-fuzz scaffold（build ok；run 留 Linux CI）
  [x] wasm rebuild 接入 start-all.ps1 + run-all-tests.ps1

Test status (as of this commit)
-------------------------------
  Rust unit:  cargo test -p protocol -p client -p server --no-run  OK
  e2e (11):   run-all-tests.ps1 -FreshEachMediaTest  ✅ 11/11 PASS
    - test-1 ~ test-10  PASS（同前 10/10）
    - test-11 断线重连  PASS  (forceDisconnect → 自动重连 → backfill，ws_auth=3)
  fuzz build: cargo +nightly fuzz build  OK

!!!!! 踩过的坑（必读，避免重复） !!!!!
------------------------------------
  【wasm 缓存陷阱】
  客户端 Rust 改动（kaede-client）必须 npm run build:wasm 重建 wasm，
  否则 apps/web 加载的是旧的 kaede_client_wasm_bg.wasm，e2e 会跑旧代码。
  这是本次会话最大的坑：P0-1 客户端 + P0-2 客户端两层修复一度全是"假阳性"，
  test-8 通过但跑的是旧 wasm。诊断线索：
    - apps/web/src/wasm/pkg/kaede_client_wasm_bg.wasm 的 mtime < crates 下源文件 mtime
    - 客户端日志（[join_room] / [backfill_all]）完全不出现
  现在 start-all.ps1 和 run-all-tests.ps1 都已接入 wasm rebuild，不会再踩。

Known technical debt
--------------------
  1. dev-mode 限流短路是临时补丁——生产走 --release 不受影响。
  2. kaede-client-wasm 的 web_sys 依赖有问题（cargo check --workspace 报 E0433）；
     实际 WASM 构建走 target=wasm32 不受影响。
  3. JoinChannelResp struct 未被使用（dead_code warning）——历史遗留，可清。
  4. cargo-fuzz run 在 Windows 缺 ASan DLL；build 通过即可，run 留 Linux CI。
  5. test-10 用 reload 测试，覆盖了"snapshot 恢复 + backfill"全链路。但用 CDP
     setOffline 单独测 WS 断线重连（不 reload）仍是更纯粹的 backfill 隔离测试。
  6. 【P1 观察】reload 后 u0 authority 收到 PeerJoined { u1 } 会认为 u1 是新成员，
     给 u1 发 Welcome epoch=1，覆盖 u1 本地可能已 epoch=2 的 group。test-10 通过
     靠 backfill 兜底，但这是潜在数据丢失。修复方向：服务端拒绝给已在
     channel_members 的 target 发 Welcome（但要注意"房主重连重建主群"的合法场景）。

Next priorities
---------------
  A. RNNoise WASM 收尾（test-5 断言）——30-45 分钟
  B. CI（GitHub Actions）跑 cargo check/test + 10 个 e2e + fuzz build
  C. P1 修复：服务端 already_member 检查 + 客户端拒绝覆盖 Welcome
  D. DM / 好友 / 编辑 / 反应 / 附件（每功能 1-2 天）
  E. UI 重做（Tailwind + Radix + Lexical）——1-2 周
  F. Tauri v2 壳子（架构已预留）
  G. Docker Compose 部署验证（deploy/ 未跑过）

Why this session matters
------------------------
  上个对话把 spawn_test_server 在清理误伤，且暴露了"9/9 e2e 但 Rust 单测炸"的现状
  （cargo check 不编译 tests/）。本轮：
  - 补 P0-1 从"主群做了频道没做"到"主群+频道都做"
  - 从"频道 Commit 完全无补发"到"两级 gap 检测 + 补发链路 + 持久化"
  - 加 test-10 验证断线重连 backfill 全链路
  - 修复 wasm 缓存陷阱（本次最大隐形炸弹）

  现有系统在 10 人场景下频道 epoch 一致、断线重连后能追平——这是 Discord
  级别频道隔离 + Signal 级别 MLS 前向安全的必要基础。
