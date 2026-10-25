KaedeNiwa HANDOFF
=================

最后更新：2026-10-25
面向：接手的 AI 工程师 / 人类工程师
状态：14/14 e2e 全绿，可公网小规模部署（局域网 / 稳定 Wi-Fi / ≤10 人）
HEAD：b984eaf


第一部分 项目定位
=================

KaedeNiwa = Discord 的使用体验 + Signal 同级的端到端加密 + 私有化部署

  不是 Discord 克隆 —— 服务端无法读取消息内容的私有化 Discord
  不是 Signal —— 带频道/服务器/媒体功能的 Signal
  不是 SaaS —— 用户自己部署在自己的 VPS / 家庭服务器

关键需求（用户原话）：
  完全丢弃所有原生客户端，全部转型 web，客户端全部都是 web 壳子
    桌面/移动 = Tauri v2 壳子（尚未实施，架构已预留）
    拒绝 Electron（体积 100MB+）
  应用体验上要做到和 discord 一样
    UI 完全重做（React + Tailwind + Radix + Lexical）
    现有 UI 是脚手架，不是最终形态
  MLS 是最先进的加密，不要放弃，说的是 Signal 同级的安全性
    使用 MLS (openmls 0.9)，RFC 9420，不是 Signal Protocol
    理由：MLS 更适合群组/频道场景
  不做妥协的半成品
    媒体功能必须真实 E2EE
    分辨率/画质必须真实生效
  文档和注释有可能存在误导性，以实际代码为准
    读代码 > 读注释（早期归档有误导描述）


第二部分 当前状态（2026-10-25）
=============================

  14/14 e2e 测试全绿（约 30 分钟一轮）
  8 个 crate，其中 7 个在 workspace：
    kaede-protocol   协议类型（CBOR + ts-rs）
    kaede-crypto     AES-GCM / HKDF / SFrame
    kaede-mls        MLS 群组（openmls 0.9）
    kaede-storage    IndexedDB (wasm) / SQLite (native)
    kaede-client     客户端核心（无 UI）
    kaede-server     服务端（axum 0.8）
    kaede-client-wasm WASM 桥接（薄封装）
    kaede-denoise    RNNoise 空壳（不在 workspace，遗留）

  唯一的 Web 前端：apps/web（React 19 + Vite 6）

  可以直接跑：start-all.ps1 / stop-all.ps1 / run-all-tests.ps1

  首次运行必须：npm run build:wasm（详见“坑 1”）


第三部分 快速上手（30 分钟）
===========================

  环境需求：
    Rust 1.99+（rust-toolchain.toml 固定）
    Node.js 22+
    wasm-pack（cargo install wasm-pack --locked）
    Playwright（cd apps/web && npx playwright install --with-deps chromium）
    LiveKit server 1.9.1（tools/livekit/livekit-server.exe）
    （Windows 环境用 Playwright msedge channel，不需要下载 chromium）

  一键启动：
    cd D:\dev2\kaedeniwa
    .\start-all.ps1 -CleanDb      清库启动（推荐首次）
    .\start-all.ps1 -SkipTest     不清库启动，不跑测试
    .\stop-all.ps1                停止所有服务

  start-all.ps1 做这些事：
    1. 清理占用 18080/5173/7880/7881/7882 的旧进程
    2. 可选清库（-CleanDb）
    3. 启动 LiveKit（tools/livekit/livekit-server.exe --dev）
    4. 启动后端（cargo run -p kaede-server --features dev-mode）
    5. 重建 wasm（npm run build:wasm）★关键步骤，见坑 1
    6. 启动 Vite（npm run dev）
    7. 等 3 个端口就绪（90 秒超时）

  手动启动：
    LiveKit（终端 1）：
      cd D:\dev2\kaedeniwa
      .\tools\livekit\livekit-server.exe --dev
    后端（终端 2）：
      cd D:\dev2\kaedeniwa
      Remove-Item Env:KAEDE_SKIP_LIVEKIT_ENV_CHECK -ErrorAction SilentlyContinue
      $env:LIVEKIT_API_KEY = 'devkey'
      $env:LIVEKIT_API_SECRET = 'secret'
      $env:LIVEKIT_WS_URL = 'ws://127.0.0.1:7880'
      $env:RUST_LOG = 'info,kaede_server=debug'
      cargo run -p kaede-server --features dev-mode
    前端（终端 3）：
      cd D:\dev2\kaedeniwa\apps\web
      npm run build:wasm
      npm run dev

  测试：
    cd D:\dev2\kaedeniwa
    .\run-all-tests.ps1 -FreshEachMediaTest   稳定模式，正式回归必须用
    .\run-all-tests.ps1                       快速模式，test-7 可能因数据库污染失败
    单跑一个：cd apps/web && node test-8-ten-users.cjs


第四部分 架构全貌
=================

  数据流：
    Web (React) → wasm (JsClient) → kaede-client (Rust) → WS/HTTP → kaede-server → SQLite
    媒体：Web → livekit-client (JS) → LiveKit SFU → 其他客户端

  crate 关系：
    kaede-protocol  ←  所有 crate 的基础类型
    kaede-crypto    ←  kaede-mls, kaede-client
    kaede-mls       ←  kaede-client
    kaede-storage   ←  kaede-client（wasm → IndexedDB，native → SQLite）
    kaede-client    ←  kaede-client-wasm（薄桥接）
    kaede-server    ←  apps/web（HTTP + WS）

  加密层（三层）：
    第 1 层 MLS group（openmls 0.9）
      主群：所有人加入服务器时 join（用于发现频道、成员列表）
      频道群：每个频道独立 MLS group（用于频道消息、媒体 key）
      DM 群：每对用户独立 group（用于私聊）
      MLS 保证：前向安全、后向安全、成员变更的 epoch 前进
      服务端永不解析 MLS payload（零知识转发）

    第 2 层 AES-256-GCM（消息体）
      频道消息：从 MLS group 的 exporter 派生 channel key
      DM 消息：从 MLS group 的 exporter 派生 dm key
      主群消息：走 MLS Application Message（不额外加密）
      sender ratchet 一次性 → 快照必须落盘（见坑 2）

    第 3 层 LiveKit E2EE（媒体）
      ExternalE2EEKeyProvider + Worker
      key 从 channel MLS exporter 派生（P0-Z，服务端无法推导）
      旧方案（服务端签发 seed）已废弃

  网络层（WS + HTTP）：
    WS：实时信令（Joined / PeerJoined / ChannelSync / ChannelMessage / MlsCommit 等）
    HTTP：注册/登录/频道 CRUD/消息历史/附件上传/LiveKit token/DM/好友


第五部分 加密层详解（最重要，不能出错的部分）
===========================================

  MLS 群组三种：
    主群（main group）：
      kaede-client/src/connection/main_group.rs
      所有人共享，用于发现频道/在线成员列表/频道消息加密通道
      Welcome + Commit 走 ws.rs 的 MlsWelcome / MlsCommit handler
      mls_commits 表存主群 Commit（每 256 条滚动保留）

    频道群（channel group）：
      kaede-client/src/connection/channel.rs
      每频道独立，用于频道消息 + 媒体 key
      Welcome 首字节 0x04，Commit 首字节 0x05
      ChannelSync.epoch 用于 gap 检测（P0-2）
      mls_commits 表 (channel_id, epoch) 复合主键

    DM 群：
      kaede-client/src/connection/dm.rs
      每对用户独立 group，用于私聊
      2 人组 handshake 走 channel_b2_e2e.rs 里同样的流程

  sender ratchet 一次性（P0 核心风险）：
    MLS 的 sender ratchet 是单向的，每条消息加密后 ratchet 前进
    如果快照丢失导致回退 → 会触发 AES-GCM nonce 复用（严重安全事故）
    防御：
      - 发送路径：mls_encrypt_app → save_snapshot → mls_send_ciphertext（顺序不能反）
      - 接收路径：250ms 批量 flush（run_event_loop 里）
      - snapshot_broken 标志：持久化失败时暂停发送
    代码位置：
      kaede-client/src/connection.rs 的 run_event_loop
      kaede-client/src/connection/main_group.rs 的 mls_encrypt_app

  媒体 E2EE（P0-Z，本会话完成）：
    旧方案：server 签发 e2ee_seed，客户端 HKDF 派生 → server 能推导 key
    新方案：客户端从 channel MLS exporter 派生 key
      kaede-client/src/connection/channel.rs 的 channel_export_key_for
      kaede-client/src/room.rs 的 begin_media_session
      server 签发的 e2ee_seed 被客户端覆盖（忽略）
    验证方法：
      看浏览器 console：'[media] ch=xxx e2ee key from MLS exporter (TRUE E2EE)'
      或 test-4 的 [Alice] [Bob] console 输出
    P0-Z 完成标准：test-4 通过 + backend ws auth >= 2

  Welcome 强验证（P0-1，本会话完成）：
    服务端：ws.rs 的 ChannelSync handler 校验 sender 是 channel authority
      此前 target 不是 member 的分支会绕过检查（漏洞），已修复
    客户端：channel_join_via_welcome 用 join_with_welcome_checked
      校验：me ∈ group.members + group.members ⊆ channel_members
      失败时：短暂重试（2 次 500ms）→ 仍失败拒绝
    降级保护（P1-3-client）：
      如果本地已有 group 且 probe.epoch() < local_epoch → 拒绝覆盖
      防止 reload 后 u0 误发 Welcome 导致 u1 从 epoch N 掉回 epoch 1

  密钥派生总结：
    channel_key      = MLS exporter(channel group, purpose=Messages)
    channel_media_key = MLS exporter(channel group, purpose=LivekitE2ee)
    dm_key           = MLS exporter(dm group, purpose=Messages)
    主群消息          = 直接走 MLS Application Message


第六部分 网络层详解
===================

  心跳（P0-2，本会话完成）：
    位置：kaede-client/src/connection.rs 的 spawn_heartbeat / heartbeat_loop
    间隔：20s 发 Ping，30s 无 Pong 认为断线
    断线处理：emit Disconnected + transport.close() → JS 层触发重连
    shutdown 标记：run_event_loop 退出时置位，heartbeat 检查后停止（防旧 heartbeat 二次 emit）

  自动重连（P0-1，本会话完成）：
    位置：apps/web/src/wasm/client.ts 的 scheduleReconnect / attemptReconnect
    触发：MainView 收到 Disconnected 事件 → scheduleReconnect
    退避：1s → 2s → 4s → 8s → 16s → 30s（共 6 次）
    重连动作：client.login(serverUrl, wsUrl, loginId, token) 重建 RoomSession
    login 内部走：connect_with_token_and_storage → snapshot 恢复 → spawn_event_loop → spawn_heartbeat → join_room（含 backfill_all_channel_epochs）
    超时保护：每次 login 10s 超时
    stopReconnect：手动登出时停止（allowReconnect 在 MainView mount 时恢复）

  429 指数退避（P1-4，本会话完成）：
    位置：kaede-client/src/http.rs 的 retry_429 helper
    应用：join_channel_only（生产环境 rate=0.5/s cap=3 很容易触发）
    退避：500ms → 1s → 2s → 4s（共 4 次，总窗口 7.5s）
    注意：其他 HTTP 端点（post_message / livekit/token）没有此保护，见技术债

  wait_for_channel_members 超时（P1-5，本会话完成）：
    位置：kaede-client/src/connection/channel.rs
    旧值：6 × 500ms = 3s（生产环境 SQLite 锁时太紧）
    新值：20 × 500ms = 10s

  P0-2 频道 Commit 补发链路：
    DB：mls_commits 表 (channel_id, epoch) 复合主键
    服务端：ChannelCommit 落库 + ReportEpoch 支持 channel_id 分支
    客户端：report_channel_epoch + backfill_all_channel_epochs
    gap 检测：ChannelSync.epoch 携带服务端 epoch，客户端比对本地 epoch
    缓冲：channel_pending_commits（每频道上限 32 条，BTreeMap）
    重连：login 后 backfill_all_channel_epochs 对所有本地 channel 主动 report epoch


第七部分 Session 管理（P1，本会话完成）
=====================================

  多 tab 锁（tab-lock）：
    位置：apps/web/src/App.tsx 的顶层 useEffect
    原理：localStorage 存 active-tab 标记；sessionStorage 存 tabId
    行为：
      新 tab 打开 → 看到已有 active-tab → 立即 logout（先到先得）
      reload → sessionStorage 保留 tabId → 不误判
      关闭 tab → 清理 active-tab（如果自己是持有者）
    监听：window.addEventListener('storage') 检测其他 tab 抢占

  多设备单 session：
    服务端：http.rs 的 create_session 里 DELETE FROM sessions WHERE login_id = ? AND token != ?
    效果：新设备登录踢旧设备；旧设备下次 API 请求返回 401 → 前端登出
    不是实时：旧设备要等到下一次请求才失败（不是主动断 WS）

  设计取舍（诚实标注）：
    多 tab 是“先到先得”，不是“新抢占旧”
    多设备是“新 session 生效，旧 session 下次请求才失败”
    长期方向：每设备独立 MLS identity（需要架构大改，暂不做）
    当前方案 = 用锁/踢旧来避免 MLS identity 冲突，不是真正的多设备支持


第八部分 Pending 队列（本会话完成）
=================================

  位置：
    apps/web/src/stores/chat.ts（PendingMsg 类型 + 5 个 action）
    apps/web/src/components/MainView.tsx（handleSend + 定时器 + 渲染）

  流程：
    1. handleSend 先 addPending（status='sending'）
    2. 调 client.send_message → 成功则 removePending + 加入 messages
    3. 失败则 markPendingFailed（status='failed'）
    4. 定时器每 5s 扫 failed，最多 5 次重试
    5. UI 渲染：灰色 + “发送中…” 或 “发送失败（重试 n/5）” + 手动重试按钮

  验证：test-13（断网发 3 条 → 恢复网络 → 自动重试 → u1 收到 3 条）


第九部分 DM / 好友 / 附件（诚实标注现状）
======================================

  DM：
    后端 API：完整（crates/kaede-server/src/dm.rs）
    wasm 桥：完成（listDmConversations / openDm / dmLoadHistory / sendDm / markDmRead）
    UI：没有
    验证：test-14（openDm + listDmConversations 工作正常）
    已知修复：open_conversation 写 dm_conversations 表（此前发起方看不到自己打开的会话）

  好友：
    后端 API：完整（crates/kaede-server/src/friends.rs）
    wasm 桥：没有
    UI：没有
    工作量估算：wasm 桥 ~10 个方法（半天）+ UI ~半天

  附件：
    后端 API：完整（crates/kaede-server/src/attachments.rs，加密 blob + 进度回调）
    wasm 桥：没有
    UI：没有
    工作量估算：wasm 桥 ~5 个方法（半天）+ UI ~1 天（拖拽 + 进度 + 预览）


第十部分 测试与 CI
==================

  14 个 e2e 测试（apps/web/test-*.cjs）：
    test-1  自动拉人（authority gate + Welcome）
    test-2  频道隔离（频道消息 + MLS group）
    test-3  断线重连（reload + snapshot 恢复）
    test-4  屏幕共享 + E2EE（P0-Z 验证点，看 [media] ... TRUE E2EE 日志）
    test-5  麦克风 + 降噪
    test-6  画质切换（480p / 1080p / 1440p）
    test-7  多人测试（3 人）
    test-8  10 用户规模（epoch 全一致）
    test-9  Authority 退出后语音连续
    test-10 频道 Commit 补发（P0-2 验证点）
    test-11 断线重连（forceDisconnect + 自动重连 + backfill）
    test-12 多 tab / 多设备（tab-lock + server kick session）
    test-13 pending 队列（断网发消息 + 自动重试）
    test-14 DM wasm 桥（openDm + listDmConversations）

  辅助文件：
    test-helpers.cjs        login / randName / injectFakeDisplayMedia / setDisableAdaptive
    test-debug.cjs          调试脚本（不是 e2e）
    test-e2e.cjs            旧的综合测试（不是 e2e）
    test-timing.cjs         时序调试脚本（不是 e2e）

  运行方式：
    cd D:\dev2\kaedeniwa
    .\run-all-tests.ps1 -FreshEachMediaTest
    约 20-25 分钟（每个 media 测试前清库重启后端）

  CI（.github/workflows/ci.yml，本会话重写）：
    7 个 job：
      fmt           rustfmt --check
      check         cargo check 6 个核心 crate
      clippy        cargo clippy 6 个核心 crate
      test          cargo nextest 6 个核心 crate（matrix）
      fuzz-build    cargo fuzz build（Linux 才能跑 run）
      typecheck-web npm run typecheck
      e2e           起 LiveKit + backend + Vite，跑 test-1/2/3/12/13（不含 media）
    未覆盖：
      media 测试（4-11）：需要真实 LiveKit + 假 media 注入，CI 里不稳定
      4K60 真实测试（需要真实 4K 显示器 + 硬件编码器）
      真实 VPS 部署验证
      多设备真实 MLS identity 冲突（当前用锁/踢旧规避）



第十一部分 已知技术债
======================

  P0（生产前必须处理）：
    1. CI 未实际跑过 —— .github/workflows/ci.yml 已重写，但从未 push 到 GitHub 触发过
       首次 push 大概率会暴露：
         - toolchain 版本（stable 现在是否等价于 1.99，需要改成 1.99.0 或验证）
         - e2e job 里 e2e 测试可能需要 headless 模式（现在 test-*.cjs 都是 headless: false + channel: msedge）
         - test-*.cjs 的 login 函数用 page.fill 假设登录页 DOM，CI 的 chromium 可能有差异
       处理方式：首次 push 后逐个修 CI 报错，不要盲目信任“已写好”
    2. deploy/ 目录从未跑过 —— Docker Compose / Caddyfile / Dockerfile 都在，但从未验证
       首次上 VPS 大概率踩：
         - 反向代理 WebSocket upgrade（Caddyfile 需要 correctly proxy /ws）
         - volume 挂载（SQLite 持久化目录）
         - LiveKit 端口暴露（7880/7881/7882 需要正确映射）
       处理方式：在本地 Docker 里先跑通再上 VPS

  P1（生产后 1-2 周内处理）：
    1. 多设备 MLS identity 冲突（当前用锁/踢旧规避）
       真正的多设备支持需要每个设备独立 identity（session_id 作为 identity 后缀）
       或者每设备独立 IndexedDB namespace
       工作量大（架构改动），当前方案可用但不是“完美方案”
    2. 429 退避只覆盖 join_channel
       post_message / livekit/token / upload 等端点没有退避
       高负载 VPS 上，消息发不出去会直接失败（pending 队列能兜底，但体验差）
    3. mls_commits 每频道保留 64 条
       长期离线（一周以上）的用户回来追不上，需要“重新 Welcome”兜底
    4. channel_members 落库延迟的时序竞态
       Welcome 可能早于 HTTP join 落库（P0-2 backfill 兜底了，但仍有窗口）
       生产高负载时可能触发（client 侧 ensure_channel_group_members 已加了 10s 容忍）

  P2（可选优化）：
    1. kaede-client-wasm 的 web_sys E0433（cargo check --workspace 报错）
       host target 编译 wasm crate 时 web_sys 不可用，但 wasm-pack 走 wasm32 target 不受影响
       CI 里已排除此 crate（只 check 6 个核心 crate）
    2. JoinChannelResp struct 未被使用（dead_code warning）
       crates/kaede-server/src/http.rs:40
       历史遗留，可清
    3. cargo-fuzz run 在 Windows 缺 ASan DLL
       build 通过即可，实际跑 fuzz 留 Linux CI
    4. test-7 快速模式（不带 -FreshEachMediaTest）因数据库污染失败
       正式回归用 -FreshEachMediaTest 就行
    5. dev-mode feature 的限流短路是临时补丁
       生产走 --release 不受影响，但 dev / prod 行为差异大
    6. RNNoise WASM 降噪未接入
       @shiguredo/rnnoise-wasm 已装但 createRNNWasmModule 导出名不对
       AudioWorklet 里不能直接 import ESM wasm 模块
       test-5 现在用浏览器原生 noiseSuppression: true
       工作量 30-45 分钟，媒体栈 100% 完成的最后一步
    7. 4K60 真实测试缺失
       test-6 只测 480p / 1080p / 1440p（用 canvas mock）
       需要真实 4K 显示器 + 硬件编码器


第十二部分 踩坑清单（本会话真实踩过，必读）
=========================================

  坑 1：wasm 必须重建才能生效
    现象：客户端 Rust 改动后，e2e 测试跑的是旧 wasm
    原因：apps/web 加载的是 apps/web/src/wasm/pkg/kaede_client_wasm_bg.wasm
          不重建就永远是旧版本
    本会话踩了 3 次（17:11 旧 wasm 让 test-10 假阳性、19:48 旧 wasm 让心跳假阳性、HistoryMessage 编译失败让 test-14 假阳性）
    诊断：apps/web/src/wasm/pkg/kaede_client_wasm_bg.wasm 的 mtime < 源码 mtime
    防御：
      - start-all.ps1 和 run-all-tests.ps1 已接入 wasm rebuild
      - 手动验证：node -e "console.log(require('fs').readFileSync('apps/web/src/wasm/pkg/kaede_client_wasm_bg.wasm','utf8').includes('关键字符串'))"
      - 任何 cargo check 失败时，测试结果全部不可信（跑的是旧产物）

  坑 2：wasm-pack --out-dir 语义
    现象：npm run build:wasm 后，apps/web/src/wasm/pkg 的 mtime 没变
    原因：wasm-pack 的 --out-dir 是相对 crate 根目录解析，不是相对 cwd
          apps/web 下执行 wasm-pack build ../../crates/kaede-client-wasm --out-dir ./src/wasm/pkg
          实际写到 crates/kaede-client-wasm/src/wasm/pkg（多了一层嵌套）
    正确：--out-dir ../../apps/web/src/wasm/pkg
    已修复（commit ad2fb28）

  坑 3：cargo check 不编译 tests/
    现象："9/9 e2e 全绿但 Rust 单测炸"
    原因：cargo check 默认不编译 tests/ 目录，集成测试的历史债藏在里面
    后果：spawn_test_server 被误删也没发现，直到 cargo test --no-run
    修复：每次改动后跑 cargo test -p <crate> --no-run（至少编译一次）

  坑 4：假阳性（本会话 4 次）
    ① test-10 用 17:11 旧 wasm（P0-2 完成时）
    ② test-11 用 setOffline（对已建立 WS 无效）
    ③ 5a77eba commit 编译失败但测试通过（跑旧产物）
    ④ test-13/14 的 $LASTEXITCODE 被后续 node 命令覆盖
    防御：
      - 三重断言（exit code + 关键日志 + backend 侧证据）
      - $LASTEXITCODE 紧跟 node 命令立即捕获
      - cargo check 失败时停止一切测试

  坑 5：PowerShell exit 陷阱
    现象：长命令里 exit 1 会让整个 pwsh 窗口关闭
    原因：PowerShell 的 exit 退出的是整个 session，不是脚本
    修复：用 throw 或 if 分支继续，不要用 exit

  坑 6：wasm 落错位置（本会话 2 次）
    现象：wasm-pack 完成后 mtime 没变
    原因：--out-dir 语义（见坑 2）
    防御：rebuild 后检查 apps/web/src/wasm/pkg/kaede_client_wasm_bg.wasm 的 mtime

  坑 7：test-*.cjs 的 evaluate 里不能有 TS 语法
    现象：page.evaluate 报 "forceDisconnect is not a function"
    原因：Playwright 把函数 toString 后传给浏览器，as any 在 JS 里是语法错误
    修复：evaluate 里的代码写成纯 JS

  坑 8：run-all-tests.ps1 的 test array trailing comma
    现象：加了 test-10/11/12 后 PowerShell 报语法错
    原因：数组最后一项不能有 trailing comma，但插入中间时需要补
    修复：每次改 test array 后用 AST parser 验证语法

  坑 9：服务窗口关闭后 Vite / LiveKit 不会自动恢复
    现象：backend 重启后测试报"缺少前置服务"
    原因：Vite / LiveKit 是独立进程，backend 重启不影响它们
         但窗口关闭会杀掉它们
    防御：每次跑测试前检查 5173 / 7880 端口，不在就起

  坑 10：wait_for_channel_members 3s 太紧
    现象：test-8 的 u5-u9 因 join_channel 429 卡死，wait 3s 后拒绝合法 Welcome
    修复：拉到 10s（P1-5）
    遗留：channel_members 落库延迟的时序竞态仍在（P0-2 backfill 兜底）

  坑 11：already_member 检查的逻辑陷阱
    现象：P1-3 服务端拒绝给已 member 的 target 发 Welcome → test-8 全炸
    原因：正常拉人场景是“先 HTTP join 落库 → 再等 Welcome”
          所以 target 在 channel_members 里不代表“已有 group”
    修复：改用客户端侧保护（P1-3-client：本地 epoch 更高时拒绝覆盖）

  坑 12：tab-lock 的 reload 误判
    现象：test-3 / test-10（都涉及 reload）失败，被锁回 login 页
    原因：tabId 用 Math.random() 生成，reload 后重新生成，与 active-tab 不匹配
    修复：tabId 用 sessionStorage 存（reload 保留，新 tab 独立）

  坑 13：test-helpers checkPrereqs 失败时 exit 0
    现象：test 前服务不可用，脚本静默返回 exit 0（假阳性）
    原因：checkPrereqs 里 return 而不是 process.exit(1)
    修复：改成 process.exit(1)（部分修了，验证时发现原代码已是 1）

  坑 14：open_conversation 不写 dm_conversations 表
    现象：发起方 openDm 后看不到自己打开的会话，对端 sendDm 后能看到
    原因：open_conversation 只返回 conv_id，不写表；post_message 才写表
    修复：open_conversation 加 INSERT OR IGNORE

  坑 15：MLS sender ratchet 一次性
    风险：快照丢失导致回退 → AES-GCM nonce 复用
    防御：
      - 发送路径必须 mls_encrypt_app → save_snapshot → mls_send_ciphertext
      - 接收路径 250ms 批量 flush
      - snapshot_broken 标志：持久化失败时暂停发送
    现状：主要发送路径已正确（encrypt_persist_and_broadcast 已验证），
          但 250ms 窗口内的崩溃仍有理论风险


第十三部分 下一步：UI 重做（用户选择方案 B）
==========================================

  用户原话：“B吧，包含在ui重做里”
  即 DM / 好友 / 附件 不单独做，全部包含在 UI 重做里

  为什么选方案 B：
    现有 UI 是脚手架（MainView.tsx 硬编码 3 个频道，没有 DM 列表，没有好友面板）
    单独给脚手架加 DM/好友/附件 UI 会返工
    用户明确要求“UI 完全重做（React + Tailwind + Radix + Lexical）”

  UI 重做需要包含：
    1. 服务器栏 / 频道栏 / 消息区 / 成员列表（Discord 风格 4 栏）
    2. DM 列表（左侧）+ DM 会话视图（复用频道消息组件）
    3. 好友列表（面板式）+ 添加/接受好友
    4. 附件上传（拖拽 + 进度 + 预览）
    5. 富文本编辑器（Lexical）
    6. Emoji picker
    7. 主题切换
    8. 响应式移动端

  UI 重做前需要先补的 wasm 桥：
    好友：list_friends / send_friend_request / accept_friend / remove_friend
    附件：upload_attachment（含进度回调）/ download_attachment
    DM 已有（本会话完成）

  工作量估算：
    wasm 桥（好友 + 附件）：1 天
    UI 重做：1-2 周
    总计：1.5-2.5 周

  UI 重做的技术选型（已确定）：
    Tailwind CSS v4（已装）
    Radix UI（未装，需要 npm install）
    Lexical（未装，需要 npm install）
    当前 UI 是手写 Tailwind（无组件库）

  重做时需要注意：
    - 不要破坏现有 e2e 测试（test-*.cjs 依赖 input[placeholder^="Message #"] 等选择器）
    - 如果必须改选择器，同步更新所有 test-*.cjs
    - test-12 依赖 tab-lock 行为（App.tsx 的 useEffect）
    - test-13 依赖 pending 队列 UI（“发送失败”文本）


第十四部分 工作方式建议（给接手 AI 的实战经验）
==============================================

  1. 编译失败时，测试结果全部不可信
     cargo check 失败 → 测试跑的是旧产物 → 所有“通过”都是假阳性
     这是本会话反复踩的坑，必须建立反射

  2. 每次客户端 Rust 改动后必须重建 wasm
     命令：cd apps/web && npm run build:wasm
     验证：mtime 变化 + wasm 里包含关键字符串

  3. 每轮改动后跑一次最小编译验证
     cargo check -p <crate>（不要 --workspace，避免 web_sys E0433）
     npm run typecheck（前端）
     cargo test -p <crate> --no-run（tests/ 目录也要编译）

  4. 每轮改动后跑相关 e2e 验证
     改加密层：test-2 / test-4 / test-8
     改网络层：test-3 / test-10 / test-11
     改 session：test-12
     改消息路径：test-13
     改 UI 组件：可能影响所有 test，必须跑全量

  5. 重大改动前先 git commit 快照
     本会话开头 git status 显示 apps/ 全是 untracked，非常危险
     任何时候 git status --short 有 M 但没有 commit，先 commit 再动

  6. 长命令分段执行
     本会话有几次命令超长导致粘贴中断
     经验：每段 ≤ 150 行，超过就拆

  7. 诊断优先于盲改
     本会话前半段（P0-1 / P0-2）花费 15+ 轮在“诊断 → 盲改 → 编译错 → 诊断”的循环
     后半段（pending / DM / test-12-14）先诊断再改，效率高很多
     建议：每个新任务先花 1 轮看清现状，再给完整 patch

  8. e2e 测试是最终验收标准
     不要因为“编译通过”或“单测通过”就认为改好了
     每个 P0 级别的改动必须有对应的 e2e 验证

  9. 假阳性识别（本会话踩了 4 次）
     exit code 0 + 关键日志 + backend 侧证据，三重验证
     test-*.cjs 里 $LASTEXITCODE 要立即捕获（不要放在后续 node 命令后）

  10. 设计取舍要诚实标注
      本会话的“dev-mode 限流短路”“tab-lock 先到先得”“server kick session 不是实时”
      都是妥协方案，必须在文档里明确标注，不要假装完美


第十五部分 联系 / 上下文
========================

  项目根：D:\dev2\kaedeniwa
  归档目录：_archive\legacy-v0.0.4\（不要碰，只做归档）
  release\：构建产物，start-all.ps1 不会用到
  data\attachments\：附件存储（如果有）

  当前状态：
    14/14 e2e 全绿
    加密层 + 网络层 + session 管理完整
    DM wasm 桥完成，UI 待做
    好友 / 附件 wasm 桥未做
    UI 是脚手架，待重做
    CI 已重写，未实际跑过
    deploy/ 未验证

  相关文档：
    STATUS.md   —— 本会话期间的进度快照（部分内容已过时）
    README.md   —— 项目简介（早期版本）
    SECURITY.md —— 安全模型（需要审阅是否反映 P0-Z 真 E2EE）

  给接手方的最后一句话：
    这个项目在加密层和网络层是认真做完的，不是玩具。
    但 UI 是脚手架，多设备支持是规避方案，CI 未验证，deploy 未跑过。
    如果你要上生产，先把 CI 跑通，再把 deploy 在本地 Docker 验证，
    然后才是 UI 重做（这是 1-2 周的大工程）。
    不要相信“文档说已经完成”——读代码，跑测试。

  祝接手顺利。
