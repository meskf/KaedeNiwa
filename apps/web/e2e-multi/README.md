# e2e-multi 目录约定

真人驱动（只用 click / type / press / 选择器 / 文件选择器），覆盖多用户、混沌、长稳场景。

## 两类文件，不能混

### `*.spec.ts` —— 真人驱动测试（CI / L6 跑）
- **驱动**：只用 `click / type / fill / press / getByTestId / getByText / waitFor / setInputFiles`
- **禁止**：`page.evaluate` 里调任何"改变状态"的方法（`store.addMessage`、`client.send_message`、`debug.dm.send` 等）
- **允许**：`page.evaluate` **只读** store / events / DOM 做断言
- **login**：必须走 `keyboard.type`（`fill` 视为绕过键盘事件，不算真人）
- **断言**：优先 `expect(locator).toBeVisible()/toHaveText()`；状态级断言放其次

### `diag-*.cjs` —— 诊断脚本（不进 CI，仅本地挖 bug）
- 可以用 `__kaede_debug__` 直调内部 API
- 目的：拿三层证据（事件层 / store 层 / DOM 层）快速定位
- **绝对不要**：把这些 diag 当成 spec 加到 playwright.config 的 testDir 里

## 场景隔离（重要）

每个 spec 用 `setupIsolatedChannel(users, prefix)` 自建独立文字频道，而不是共用
`general`。原因：所有用例共用 general 时，不同 worker 会互相触发频道权威接管
（takeover），把彼此的成员清出频道，导致消息"丢失"——那是测试 artifact，不是产品 bug。

`setupIsolatedChannel` 会：
1. 创建者点 "+" 新建频道 → 进入并建好 MLS group
2. 其他用户等节点同步（服务端 10s 轮询）后逐个进入
3. 等每个客户端的 `channelState(id).has_group === true`
4. 用 sentinel 消息确认收发链路

## 已知限制（测试层面）

- L6 默认串行。跨 spec 并行会 churn 主群（reaction / media 信令所在的 MLS 主群）
  的权威关系；需要压测并发时设 `KAEDE_MULTI_WORKERS=4`，并接受更高失败率。
- 新成员看不到加入前的频道历史（频道 key 随 epoch 轮换，前向保密）。测试只断言
  加入后的实时互通。
- 频道权威全员离线时客户端会请求 `/api/channels/{id}/takeover`：
  服务端清空其他成员并排入 `channel_removed`，被移除者下次连接丢弃旧 group。
