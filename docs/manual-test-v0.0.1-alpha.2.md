# KaedeNiwa v0.0.1-alpha.2 手测清单

> 覆盖：round2 修复、C14 落地、WebRTC 浏览器双模屏幕共享、块 B 私密频道自动重连。

## 0. 前置

```powershell
cd D:\dev2\kaedeniwa

# 1. LiveKit（若未装，跳过则语音测试失败）
ls tools\livekit\livekit-server.exe

# 2. signaling server 构建
cargo build -p kaede-server

# 3. 检查 dx
dx --version   # 期望 dioxus 0.7.x
```

## 1. 一键启动（新 terminal）

```powershell
cd D:\dev2\kaedeniwa
.\_dev-all.ps1
# 若 _dev-all.ps1 已被清掉，重新手动：
#   tools\livekit\livekit-server.exe --dev  # terminal 1
#   $env:LIVEKIT_API_KEY='devkey'; $env:LIVEKIT_API_SECRET='secret'
#   $env:LIVEKIT_URL='http://localhost:7880'
#   target\debug\kaede-server.exe            # terminal 2
#   dx serve --package kaede-wasm --platform web  # terminal 3
```

浏览器打开 http://localhost:8081（dx 默认端口）。

## 2. 基础功能（两个 tab）

| # | tab A | tab B | 预期 |
|---|---|---|---|
| 1 | 注册 alice | 注册 bob | 都成功 |
| 2 | 登录 alice | 登录 bob | 主群建立 |
| 3 | 在 general 发 "hi" | — | A 自己立刻看到；B 1s 内看到 |
| 4 | — | 在 general 回 "yo" | 双端看到 |
| 5 | 刷新页面 | — | alice 历史保留（自己发的 + 缓存的） |

## 3. C14 落地验证（核心）

### 3.1 主群语音 E2EE

| # | tab A | tab B | 预期 |
|---|---|---|---|
| 1 | 点 general 频道旁的语音按钮 | — | A 的 VoicePanel 出现，成员 1 人 |
| 2 | — | 点同一语音按钮 | B 也进；两端成员列表都 2 人 |
| 3 | A 点麦克风 | — | A 说话 B 能听到；DevTools console 搜 `frameDecrypted` 应 > 0 |
| 4 | — | B 点麦克风 | 双向通 |
| 5 | A 点屏幕共享 | — | 浏览器弹选择器 → 选屏幕；**B 的 VoicePanel 下 `#kaede-remote-videos` 出现 A 的画面** |
| 6 | A 关屏幕共享 | — | B 端画面消失 |
| 7 | A 离开 | — | B 端 A 消失；A 回无语音 |

**关键断言**：步骤 5 是 WebRTC 双模落地的验收点。若 B 端黑屏但控制台有 `trackSubscribed` 日志，可能是 `<video>.play()` 未自动触发——打开 DevTools 检查 video 元素是否有 `srcObject`。

### 3.2 私密频道语音 E2EE + 块 B 自动重连

| # | tab A | tab B | tab C | 预期 |
|---|---|---|---|---|
| 1 | 创建私密频道 secret（密码 ppp） | — | — | 成功 |
| 2 | 进 secret 语音 | — | — | A 进；成员 1 人 |
| 3 | — | 加入 secret（密码 ppp） | — | B 加入；双方成员 2 人；**B 端出现 A 的画面/声音**（若 A 有发） |
| 4 | — | — | 加入 secret（密码 ppp） | C 加入；**A 和 B 的 toast 出现「私密频道密钥已轮换，正在自动重连…」，0.5-1s 后恢复** |
| 5 | 说话 | 说话 | 说话 | 三方通 |

**关键断言**：步骤 4 是块 B 私密频道部分的验收点。

### 3.3 DM 语音 E2EE

| # | tab A | tab B | 预期 |
|---|---|---|---|
| 1 | 加 bob 好友（通过 #general 频道右键或其他入口） | — | 好友列表出现 bob |
| 2 | — | 接受好友 | 双方好友关系建立 |
| 3 | A → 打开 DM → 与 bob 开启语音 | — | A 端 DM 通话 UI 出现 |
| 4 | — | B 进同一 DM 通话 | 双方通 |

**注意**：DM 的 UI 入口若暂未接语音，这一步可跳过，直接看 4.5 的日志验证。

## 4. 日志验证（DevTools console / 终端）

### 4.1 E2EE key 存在性

浏览器 console：
```js
// LiveKit UMD 已加载
typeof window.LivekitClient  // "object"
```

Rust tracing（terminal 3 里 dx serve 输出）搜：
```
begin_media_session        # 每次连语音
E2EE enabled               # C14 落地后的关键日志（DM / private / main）
frameDecrypted             # LiveKit E2EE 解密帧数 > 0
```

### 4.2 块 B 自动重连

搜 `reconnect_request` 或 `ChannelEpochChanged`：
```
[voice] private channel {id} epoch changed to {N}; reconnect
私密频道密钥已轮换，正在自动重连…
```

### 4.3 浏览器 WebRTC（Chrome）

`chrome://webrtc-internals` → 选当前 tab → 应看到：
- `outbound-rtp` 有 video + audio（屏幕共享时）
- `inbound-rtp` 有 video + audio（对方共享时）
- E2EE stats：`frameDecryptionFailure` 应为 0

## 5. 跨设备验证（可选）

前提：两台机同一局域网。

1. 机 1 起 server + livekit
2. 机 1 浏览器：`http://<机1IP>:8081` 登录 alice
3. 机 2 浏览器：同上地址登录 bob（或用 `kaede-desktop.exe`）
4. 重复 3.1 步骤

**关键断言**：机 2 应能看到机 1 屏幕。若跨机器不通，检查：
- LiveKit 的 `--dev` 模式是否会分配公网候选；本地要 `--node-ip` 指向局域网 IP
- 防火墙是否放行 7881/7882 udp+tcp

## 6. 失败排查

| 症状 | 排查 |
|---|---|
| `LivekitClient` undefined | `index.html` 里 UMD script 加载失败，检查 CDN 或换本地文件 |
| `SdkNotLoaded` | 同上 |
| 语音双方 key 不同 → 静默 | 检查 `begin_media_session` 日志是否都显示 "E2EE enabled"；如果一有一无，说明某方 group 缺失 |
| 屏幕共享 B 端黑屏 | DevTools 检查 video 元素 `srcObject`；若 null 说明 attach 失败 |
| 块 B 不触发重连 | 检查 C 加入时 A/B 端是否 log `ChannelSync 0x05` 和 `ChannelEpochChanged` |

## 7. 通过标准

- [ ] 2.1-2.5 全部通过
- [ ] 3.1 全部通过，特别是步骤 5
- [ ] 3.2 全部通过，特别是步骤 4
- [ ] 4.1 日志有 `E2EE enabled`
- [ ] 4.3 webrtc-internals `frameDecryptionFailure == 0`
