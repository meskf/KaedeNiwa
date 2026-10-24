# Beta 部署与运维手册

面向小规模生产 / Beta 环境的部署、可观测性与恢复流程。
服务端是**单二进制 + SQLite**，没有内建 TLS，必须放在反向代理之后。

## 1. 拓扑与端口

```
浏览器 / Desktop ──HTTPS/WSS──> Caddy 或 Nginx ──HTTP──> kaede-server :18080
                                                      ├─ /api/*   HTTP API
                                                      ├─ /ws      WebSocket 信令
                                                      ├─ /health  存活探针
                                                      ├─ /ready   就绪探针（查 DB）
                                                      └─ /metrics Prometheus（勿公网暴露）

管理后台（可选）: kaede-server 独立 listener 127.0.0.1:18081
LiveKit: 独立进程 / 独立端口，LIVEKIT_WS_URL 必须指向它
```

Web 静态站由 Caddy/Nginx 直接提供（`dist/web/public`，compose 挂载 `dist/web` →
`/srv/kaede`，站点 root 指向 `/srv/kaede/public`），服务端**不托管静态文件**。

## 2. 构建产物

```powershell
# Web（PWA）
pwsh -File scripts/bundle.ps1 web          # → dist/web/public

# 服务端（本机）
cargo build --release -p kaede-server      # → target/release/kaede-server

# 服务端（容器）
docker build -f deploy/Dockerfile -t kaede-server:0.0.1 .
```

## 3. 环境变量

| 变量 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `KAEDE_BIND` | 否 | `0.0.0.0:18080` | 监听地址；本机部署建议 `127.0.0.1:18080` |
| `KAEDE_DB` | 否 | `sqlite://kaede.db?mode=rwc` | 只支持 `sqlite:`；父目录会自动创建 |
| `KAEDE_SESSION_TTL_HOURS` | 否 | `168` | 会话有效期，`1..=8760` |
| `KAEDE_DATA_DIR` | 否 | `<cwd>/data` | 附件密文根目录；容器部署应指向可写卷 |
| `KAEDE_TRUSTED_PROXIES` | 容器部署建议 | 仅 loopback | 逗号分隔 IP/CIDR；Docker bridge 需设为 compose 固定网段 `172.28.0.0/16`，否则所有用户共享代理 IP |
| `KAEDE_CORS_ORIGINS` | 生产必填 | 本地开发端口 | 逗号分隔；生产禁止 `*` |
| `KAEDE_LOG_FORMAT` | 否 | `human` | `json` 输出单行 JSON |
| `RUST_LOG` | 否 | `info,kaede_server=debug` | tracing 过滤 |
| `LIVEKIT_API_KEY` | **是** | 无 | 缺失时启动失败 |
| `LIVEKIT_API_SECRET` | **是** | 无 | 缺失时启动失败 |
| `LIVEKIT_WS_URL` | 否 | `ws://localhost:18082` | 必须是 `ws://` 或 `wss://` |
| `KAEDE_LIVEKIT_HTTP_URL` | 否 | 由 `LIVEKIT_WS_URL` 推导（`ws`→`http` / `wss`→`https`） | LiveKit RoomService HTTP 端点；WS 与 HTTP 不同端口时显式设置 |
| `KAEDE_ADMIN_ENABLED` | 否 | `0` | `1` 启动独立管理 listener |
| `KAEDE_ADMIN_BIND` | 否 | `127.0.0.1:18081` | 非 loopback 且无 token 时拒绝启动 |
| `KAEDE_ADMIN_TOKEN` | 公网后台必填 | 空 | 未设置时启动生成一次性 token 并打印 |

启动时会校验以上配置，任何一项非法都直接退出（fail-fast）。
完整样例见 [`../.env.example`](./.env.example)。

### 3.1 LiveKit webhook（强烈建议）

服务端提供 `POST /api/livekit/webhook`，用于在参与者崩溃/断网（无法调用
`/leave`）时立即回收媒体 seed。没有 webhook 时，seed 会保留到 8 小时 TTL，
同一房间下一次会话可能复用同一 E2EE key。

在 LiveKit 配置中加上（`api_key` 必须与 `LIVEKIT_API_KEY` 相同）：

```yaml
webhook:
  api_key: <LIVEKIT_API_KEY>
  urls:
    - https://kaede.example.com/api/livekit/webhook
```

服务端会用 `LIVEKIT_API_SECRET` 校验请求的 HS256 JWT 与 `sha256(body)`。
内置 `livekit-server.exe`（bundle 双击启动）不配置 webhook，仅适合本机试用；
生产请使用独立 LiveKit 并配置上面的 webhook。

### 3.2 媒体访问吊销（踢人 / 封禁 / 移除频道成员）

只删数据库行和 MLS 成员**不会**中断已经建立的 LiveKit 连接（token 有效期 6 小时），
被移除者仍能继续收听该房间的音视频。因此服务端在以下操作后会调用 LiveKit
RoomService `RemoveParticipant` 主动断开：

- 私密频道移除成员 / 成员主动退出（同时回收该房间 media seed）
- 踢出服务器成员、封禁账号（回收该账号在所有房间的 seed）

这需要服务端能访问 LiveKit 的 **HTTP** 管理端点（默认由 `LIVEKIT_WS_URL`
推导，可用 `KAEDE_LIVEKIT_HTTP_URL` 覆盖）。调用失败只记 warn 日志：seed 已经
同步回收，被踢者拿不到新的 e2ee key，仍属于 E2EE 保护范围内。

## 4. 部署方式

### 4.1 systemd（推荐单机）

```bash
sudo useradd --system --home /var/lib/kaede --shell /usr/sbin/nologin kaede
sudo install -d -o kaede -g kaede /var/lib/kaede
sudo install -d -m 0750 /etc/kaede
sudo install -m 0600 deploy/.env.example /etc/kaede/kaede.env   # 填写后保持 600
sudo install -m 0755 target/release/kaede-server /usr/local/bin/
sudo install -m 0644 deploy/kaede-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now kaede-server
sudo systemctl status kaede-server --no-pager
journalctl -u kaede-server -f
```

unit 已启用 `NoNewPrivileges` / `ProtectSystem=strict` / `ReadWritePaths=/var/lib/kaede`，
数据库只能写入 `/var/lib/kaede`。

### 4.2 Docker Compose

```bash
cp deploy/.env.example deploy/.env      # 填写密钥与域名
pwsh -File scripts/bundle.ps1 web       # 生成 dist/web
docker compose -f deploy/docker-compose.yml up -d --build
docker compose -f deploy/docker-compose.yml logs -f kaede
```

compose 只把 Caddy 的 80/443 暴露到公网；`kaede` 仅在容器网络内监听 18080。
数据库与附件放在命名卷 `kaede-data`。compose 已设置 `KAEDE_UPSTREAM=kaede:18080`
（Caddy 反代目标）和 `KAEDE_TRUSTED_PROXIES=172.28.0.0/16`（compose 固定网段），
否则容器内会把所有用户识别成同一个代理 IP，登录失败计数会全站共享。

### 4.3 TLS / 反向代理

- Caddy：直接使用 [`Caddyfile`](./Caddyfile)，设置 `KAEDE_DOMAIN`，自动签发证书。
- Nginx：参考 [`nginx.conf`](./nginx.conf)。要点：
  - `/ws` 必须转发 `Upgrade` / `Connection` 头；
  - `client_max_body_size` ≥ 36m（附件 32 MiB + 包装）；
  - `proxy_read_timeout` 放宽到 3600s；
  - `/metrics` 只允许本机或监控网段访问。

> 服务端自身不终结 TLS。把 `KAEDE_BIND` 绑在 loopback，公网只开 443。

## 5. 可观测性

| 端点 | 用途 | 说明 |
|---|---|---|
| `GET /health` | 存活探针 | 进程能响应即 200，不查 DB |
| `GET /ready` | 就绪探针 | DB 可查询且附件目录可写时 200，否则 503 |
| `GET /metrics` | Prometheus | 计数/容量，不含账号与消息内容 |

主要指标：

- `kaede_ws_connections` / `kaede_ws_connection_limit`
- `kaede_online_peers`
- `kaede_messages_total`
- `kaede_rate_limited_total`
- `kaede_broadcast_budget_rejected_total`
- `kaede_slow_peer_dropped_total`
- `kaede_auth_failures_total`
- `kaede_db_pool_connections` / `kaede_db_pool_idle`

建议告警（Prometheus 规则示例）：

```yaml
groups:
  - name: kaede
    rules:
      - alert: KaedeDown
        expr: up{job="kaede"} == 0
        for: 2m
      - alert: KaedeNotReady
        expr: probe_success{job="kaede-ready"} == 0
        for: 2m
      - alert: KaedeWsSaturation
        expr: kaede_ws_connections / kaede_ws_connection_limit > 0.85
        for: 5m
      - alert: KaedeAuthFailureSpike
        expr: increase(kaede_auth_failures_total[5m]) > 50
        for: 5m
      - alert: KaedeRateLimitSpike
        expr: increase(kaede_rate_limited_total[5m]) > 100
        for: 10m
      - alert: KaedeSlowConsumers
        expr: increase(kaede_slow_peer_dropped_total[5m]) > 0
        for: 10m
```

`/metrics` 不含敏感数据，但仍建议只让监控网段访问（Caddy/Nginx 已默认隐藏）。

## 6. 备份与恢复

SQLite 使用 WAL。**不要直接 `cp kaede.db`**，会漏掉 WAL 中已提交的数据。
使用 sqlite3 在线备份（`.backup` / `VACUUM INTO`）：

```powershell
# Windows
pwsh -File scripts/backup-db.ps1 -DbPath C:\kaede\kaede.db -OutDir C:\kaede\backups

# 恢复（先停服务！）
pwsh -File scripts/backup-db.ps1 -Mode restore -DbPath C:\kaede\kaede.db -Backup C:\kaede\backups\kaede-20261022-120000.db
```

```bash
# Linux
scripts/backup-db.sh backup /var/lib/kaede/kaede.db /var/lib/kaede/backups
sudo systemctl stop kaede-server
scripts/backup-db.sh restore /var/lib/kaede/kaede.db /var/lib/kaede/backups/kaede-20261022-120000.db
sudo systemctl start kaede-server
```

脚本会校验 `PRAGMA integrity_check` 并输出 SHA-256；恢复时若存在 `.sha256`
旁车文件会先校验哈希。恢复流程会删除旧的 `-wal` / `-shm`，避免把旧日志重放到新库上。

**附件不在数据库里**：`KAEDE_DATA_DIR/attachments` 是独立目录，必须一起备份
（加密 blob 丢失后无法从数据库恢复）。恢复时也要一并还原，并确保属主仍是
运行服务的系统用户（如 `kaede`），否则上传/下载会 500。

建议：每日备份数据库 + 附件目录，保留 14 天；备份目录与数据不要放在同一块盘。

## 7. 升级 / 迁移 Runbook

服务端启动时由 `sqlx::migrate!` **自动应用** `crates/kaede-server/migrations/` 中尚未执行的迁移。

1. 备份数据库（见上一节），确认 `integrity_check` 为 `ok`。
2. 构建新二进制 / 镜像，先不要停旧进程。
3. 停服务：`systemctl stop kaede-server`（或 `docker compose stop kaede`）。
   服务端已启用优雅停机：收到 SIGTERM 后停止接收新连接并等待在途请求结束。
4. 替换二进制 / `docker compose up -d --build`。
5. 启动后检查：
   ```bash
   curl -fsS http://127.0.0.1:18080/ready
   curl -fsS http://127.0.0.1:18080/metrics | head
   journalctl -u kaede-server -n 100 --no-pager
   ```
6. 客户端同时升级：协议当前为 **v6**，旧客户端会被明确拒绝（`UnsupportedVersion`），
   不会静默解密失败。

**回滚**：停服务 → 恢复升级前备份 → 换回旧二进制 → 启动。
注意：升级后写入的新数据在回滚备份中不存在，回滚会丢失这部分数据；
若迁移只新增表/列，通常可以保留新库继续使用旧二进制，但需先确认版本说明。

**迁移失败**：服务端会启动失败并打印 `sqlx` 错误。此时数据库可能处于部分迁移状态：
先恢复备份，再排查失败迁移，不要反复重启。

## 8. 故障排查

| 现象 | 排查 |
|---|---|
| 启动即退出，提示 `LIVEKIT_API_KEY must be set` | 补齐 LiveKit 环境变量，或仅开发环境设 `KAEDE_SKIP_LIVEKIT_ENV_CHECK=1` |
| 启动退出，提示 `KAEDE_BIND is not a socket address` | 检查 `host:port` 格式 |
| 启动退出，提示 admin 无 token | 设置 `KAEDE_ADMIN_TOKEN` 或改回 loopback 绑定 |
| `/ready` 503 | 数据库路径不可写 / 磁盘满 / 文件损坏；查看日志 |
| 浏览器 CORS 报错 | `KAEDE_CORS_ORIGINS` 必须是完整 origin（含 scheme，无尾斜杠） |
| WebSocket 404 / 400 | 反向代理没有转发 `Upgrade` 头，或没有代理 `/ws` |
| 附件上传 413 | 反向代理 `client_max_body_size` 太小（需 ≥ 36m） |
| 连接 5 分钟后断开 | 客户端未发心跳；服务端 idle timeout 为 300s |
| 管理后台 401 | `X-Admin-Token` 不匹配；token 变更后需重启服务端 |

## 9. 发布前负载 / 滥用验证

仓库内置两组回归，默认不进入每个 PR 的 CI：

```powershell
# WS 扇出（12 人 × 1 MiB）、顶替风暴、4 MiB 洪泛、超大帧
cargo test -p kaede-server --test load_abuse_e2e -- --ignored --test-threads=1

# 协议解码健壮性（随机/截断/翻位输入，常规 CI 会自动执行）
cargo test -p kaede-protocol --test decode_robustness
```

发布前建议在预发环境按真实在线规模再跑一次扇出测试，并同时观察
`/metrics` 的 `kaede_rate_limited_total` 与出口带宽。

## 10. Beta 上线检查清单

- [ ] `KAEDE_BIND` 绑定 loopback，公网只暴露 443
- [ ] 证书有效，`https://` 与 `wss://` 均可用
- [ ] `KAEDE_CORS_ORIGINS` 只包含真实站点
- [ ] `LIVEKIT_API_KEY/SECRET` 为生产值，未复用开发密钥
- [ ] 管理后台保持 loopback + SSH 隧道，或 HTTPS + IP 白名单
- [ ] `/ready` 接入负载均衡 / systemd watchdog
- [ ] Prometheus 已抓取 `/metrics`，关键告警已配置
- [ ] 每日备份任务已启用，且做过一次**实际恢复演练**
- [ ] 日志为 JSON 并已接入采集（或至少配置 logrotate）
- [ ] 客户端与服务端协议版本一致（v6）
- [ ] `SECURITY.md` 中的半信任 server 边界已告知用户
