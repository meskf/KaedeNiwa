# KaedeNiwa 状态快照

最后更新：2026-11-03

## 当前版本

- 协议 v8（MIN_SUPPORTED_VERSION = 7），workspace 版本 0.1.2
- 7 个 crate 在 workspace（kaede-denoise 未纳入）
- 前端：apps/web（React 19 + Vite 6 + Tailwind 4）+ apps/desktop（Electron）

## 测试基线（2026-11-03 实测）

    Rust workspace          全绿（0 failed / 8 ignored）
    Rust client native      112 passed / 0 failed
    Playwright e2e-standard 14 passed / 0 failed
    Playwright e2e-multi    10 spec / 21 passed / 1 skipped，串行全绿
    L7 服务端重启混沌        4 clients，重启 16s，4/4 重连，交叉可见 PASS，0 pageError
    clippy                  0 warning（kaede-client / server / wasm）

分层入口：`tools/qa/run-all.ps1`（L0–L7），详见 `tools/qa/README.md`。
QA 使用独立库 `tools/qa/.run/qa.db`，`-Force` 会重建；不污染 `kaede.db`。

## 2026-11-03 本轮完成

### 测试体系
1. 新增 e2e-multi 真人驱动全域套件（10 spec）：
   频道/目录 CRUD、私密频道密码与隔离、消息/反应/typing/多行/连发、
   好友/DM、附件上传下载、搜索、设置/权限、会话面板、
   5 人长稳（刷新+重登）、同账号顶替、三人并发加入、长断网恢复。
2. 每个场景使用独立频道（`setupIsolatedChannel`），避免跨场景 churn
   主群/频道群权威；L6 默认串行。
3. 新增 L7 服务端重启混沌脚本（kill → 10s → 重启 → 重连/交叉可见）；
   `chaos-restart.mjs` 直接 spawn `target/debug/kaede-server.exe`，避免
   start-env 的 cargo/探活耗时把重连窗口拖过客户端退避上限。
4. QA env 支持 `KAEDE_TEST_MODE`、独立 DB、admin 面（:18081, 固定测试 token）。
5. start-env 统一注入 LiveKit/DB/admin env；`-Force` 重建 QA 库。
6. `run-all.ps1` 启动时清除继承的 `CI` 环境变量，否则 Playwright 会强制
   `reuseExistingServer=false` 并与已启动的 QA env 端口冲突。

### 修复的真实缺陷
1. 发送方自己看到重复消息：乐观消息 ts 与服务器 ts 偏差 >2s 去重失效；
   改为发送接口返回权威 `(id, ts)`，前端不再插入无 id 消息。
2. 历史同步整体替换列表，和发送并发时丢消息；`setMessages` 改为原子合并。
3. 图片/音视频附件永远 “Loading...”：effect 依赖 loading + cancelled 标志
   丢弃了第一次请求结果。
4. 登出不撤会话、不断 WS：新增 `signOut()`（断 WS + POST /api/logout）。
5. 顶替后旧会话变“僵尸界面”：服务端 login 主动断开旧 WS；客户端识别
   `ServerError` 致命码（snake_case）回登录页；重连耗尽也清理登录态。
6. 断网 401/transport 错误被误判为“私密频道密码”：只在明确 `http error (401)`
   且非私密频道时处理；断网走重连。
7. 节点改名/删除只认证不校验权限（任意登录用户可删频道）：补齐 op3+ 校验。
8. 新建频道其他用户看不到：节点树 10s 轮询 + 可见性/焦点刷新。
9. 频道群权威全员离线时客户端自建群 → 永久群分裂：
   新增 `POST /api/channels/{id}/takeover`（仅当其他成员全离线）；
   服务端清空旧成员并向其投递 `channel_removed`，客户端丢弃旧 group 重新加入。
10. 连接失败后浏览器迟到的 onclose 触发 wasm panic：
    wasm WS 在释放 Closure 前解绑事件；`Drop` 同样处理。
11. 心跳检测最坏 ~60s：改为 10s ping / 20s timeout / 2.5s 检查；
    重连在长断网后能自动恢复。
12. wasm 发送接口改为返回服务端 `{id,ts}`；`sendMessageWithAttachments` 同步。
13. admin `/api/users` 返回 op 字段（测试可用 `ensureOp5` 编排）。
14. 服务端崩溃/网络断开被 wasm transport 当成“主动关闭”（`ClientError::Closed`），
    event loop 不 emit `Disconnected` 且置 `shutdown`，心跳随之退出 →
    客户端永久停在“看似在线”的僵尸状态，消息只在本地插入不同步。
    传输层新增主动关闭标记：非主动 close/onerror/正常关闭一律返回
    `Transport` 错误（web + native 各自实现），上层据此 emit `Disconnected`
    并走指数退避重连。L7 重启混沌与 L6 长断网场景覆盖此路径。

## 待办 / 已知限制（上生产前建议）

### P0（架构级，需单独设计）
- 主群（reaction / media 信令）仍依赖“第一个用户 = host”。host 离线后
  新用户可能各自建主群 → reaction/media 信令跨群不可解。
  频道群已有 takeover 兜底，主群没有等价机制。
- 单设备在线是刻意约束；新设备登录会顶替旧设备，旧设备的 MLS 群状态
  无法迁移。如需多设备，需要 device linking / 每设备独立 leaf 的设计。
- 新成员看不到加入前的频道历史（频道 key 随 epoch 轮换，前向保密）。
  UI 目前显示 “Welcome to the beginning of the channel.”，建议明确提示。

### P1
- 主群 media 信令与 LiveKit E2EE 的跨群一致性（见 P0 第一条）。
- 消息编辑/删除：客户端 payload 已实现（Edit/Delete），UI 未暴露。
- 元数据最小化：presence / typing 服务端可见。
- safety number / 密钥指纹核验（Signal 同级定义性特征）。

### P2
- wasm/src/lib.rs 拆分模块 + TS 类型化。
- Playwright 扩 firefox/webkit smoke。
- 浏览器访问必须是安全上下文：`http://<局域网IP>` 下 Web Crypto 不可用，
  前端会显示明确提示页；跨机使用需域名 + HTTPS（桌面/Android 不受限）。

## 已知 Open Issue

**主群 host 离线后的 reaction / media 信令分裂**（见 P0）。
表现为：并行/长时间运行后，部分用户对同一消息的 reaction 不同步；
语音/屏幕共享信令跨主群不一致。频道消息本身不受影响（走独立频道群）。


## 打包与产物验收（2026-11-03，版本 0.1.2）

### 产物
    release/KaedeNiwa-server-0.1.2.zip            Windows 独立服务端（含 LiveKit + 前端）
    release/desktop/KaedeNiwa-0.1.2-setup.exe     安装版
    release/desktop/KaedeNiwa-0.1.2-portable.exe  绿色版
    release/apk/KaedeNiwa-android-0.1.2.apk       已签名 release APK（versionCode 2 / versionName 0.1.2）
    release/SHA256SUMS.txt / README-0.1.2.md      校验和与开箱即用说明
    docker compose up -d --build                 一体化镜像（内置 Caddy + 服务端 + LiveKit）

### 实测（全部用 0.1.2 产物重跑）
    Docker 一体化镜像             部署冒烟全绿（文本/附件/双人语音 E2EE/LiveKit 2 人），
                                  LiveKit webhook 全部投递成功，无重试
    Windows 独立包（干净目录解压）  同一套部署冒烟全绿
    桌面客户端（新构建）           真实登录 + 频道 + 发消息上屏 PASS
    Android release APK           安装启动正常（与 0.1.1 相同的 dist，仅版本号变化）

### 打包期修掉的坑
- LiveKit 带 --config 文件时 LIVEKIT_KEYS 被忽略 -> 改用 LIVEKIT_CONFIG(config-body)。
- rtc.port_range_start==port_range_end 在 1.13.x 会 panic -> 统一改 rtc.udp_port。
- rtc.node_ip 只能写在配置里（环境变量 LIVEKIT_RTC_NODE_IP 不生效）-> 入口脚本
  按 LIVEKIT_NODE_IP 用 sed 覆写 node_ip 并关闭 STUN。
- 入口脚本里可执行文件是 /livekit-server（不在 PATH），写 livekit-server 会 127 重启。
- Caddy 顶层 try_files {path} /index.html 会把 /api/* 也回退成 HTML -> 改成 handle 分支。
- 运行镜像不能用 caddy:2-alpine（musl）承载 glibc 的 kaede-server -> debian-slim + 拷入
  静态 caddy 二进制。
- Android/Electron 的 origin 不是服务端，相对 /api/... 会打到本地资源服务器 ->
  前端统一走 apiUrl()；LoginView 默认地址改为 origin（仅 file:// 与 Capacitor 用 18080）。
- Android 的 versionName 原为固定 "1.0"，已随版本改为 0.1.2（versionCode 1 -> 2）。