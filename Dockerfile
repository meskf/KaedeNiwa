# KaedeNiwa 一体化镜像：Rust 服务端 + WASM + React 前端 + Caddy。
# LiveKit 由 docker-compose 作为独立服务启动（见 docker-compose.yml）。
# 目标：`docker compose up -d` 后 http://<host>:8080 直接可用。

# ---------- Stage 1: 编译 WASM 与服务端 ----------
FROM docker.m.daocloud.io/library/rust:1.99-bookworm AS rust-builder
WORKDIR /app

# 国内网络镜像：rustup 静态站 / crates.io 索引走 rsproxy，构建无需访问 GitHub。
# 限制并行编译任务数：16GB 内存的小机器全并行会把 buildkit 直接 OOM 掉。
ENV RUSTUP_DIST_SERVER=https://rsproxy.cn \
    RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup \
    CARGO_BUILD_JOBS=2

RUN find /etc/apt/ -type f -exec sed -i 's/deb.debian.org/mirrors.tuna.tsinghua.edu.cn/g' {} + \
    && find /etc/apt/ -type f -exec sed -i 's/security.debian.org/mirrors.tuna.tsinghua.edu.cn/g' {} + \
    && apt-get update \
    && apt-get install -y curl git pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# wasm32 target（rust-toolchain.toml 在镜像内被移除，避免锁定本机工具链）
RUN rustup target add wasm32-unknown-unknown

# 用 cargo 安装 wasm-bindgen-cli（版本必须与 Cargo.lock 中的 wasm-bindgen 一致），
# 替代 wasm-pack：后者会从 GitHub Releases 下载预编译二进制，国内网络不可达。
RUN printf '%s\n' \
      '[source.crates-io]' \
      'replace-with = "rsproxy-sparse"' \
      '' \
      '[source.rsproxy-sparse]' \
      'registry = "sparse+https://rsproxy.cn/index/"' \
      '' \
      '[net]' \
      'git-fetch-with-cli = true' \
      > "$CARGO_HOME/config.toml" \
    && cargo install wasm-bindgen-cli --version 0.2.130 --locked

COPY . .
RUN rm -f rust-toolchain.toml

# 编译 WASM（等价于 wasm-pack build --target web，产物路径与 npm run build:wasm 一致）
RUN cargo build -p kaede-client-wasm --release --target wasm32-unknown-unknown --locked \
    && mkdir -p apps/web/src/wasm/pkg \
    && wasm-bindgen --target web --out-dir apps/web/src/wasm/pkg \
         --out-name kaede_client_wasm \
         target/wasm32-unknown-unknown/release/kaede_client_wasm.wasm

# 编译 Rust 后端（release）
RUN cargo build -p kaede-server --release --locked

# ---------- Stage 2: 编译前端 ----------
FROM docker.m.daocloud.io/library/node:22-slim AS web-builder
WORKDIR /app
RUN npm config set registry https://registry.npmmirror.com
COPY --from=rust-builder /app/apps/web/src/wasm/pkg ./apps/web/src/wasm/pkg
COPY apps/web ./apps/web
WORKDIR /app/apps/web
RUN rm -rf node_modules \
    && npm install --no-audit --no-fund \
    && npm run build

# ---------- Stage 3: 运行时（Caddy + server） ----------
# 注意：不能用 caddy:2-alpine 作为运行时——它是 musl 环境，而 kaede-server 是在
# bookworm(glibc) 上编译的动态链接 ELF，直接拷进去会 "not found"。
# 这里用 debian-slim，再把 caddy 官方静态二进制从 alpine 镜像拷进来。
FROM docker.m.daocloud.io/library/debian:bookworm-slim

RUN find /etc/apt/ -type f -exec sed -i 's/deb.debian.org/mirrors.tuna.tsinghua.edu.cn/g' {} + \
    && find /etc/apt/ -type f -exec sed -i 's/security.debian.org/mirrors.tuna.tsinghua.edu.cn/g' {} + \
    && apt-get update \
    && apt-get install -y ca-certificates tzdata curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=docker.m.daocloud.io/library/caddy:2-alpine /usr/bin/caddy /usr/bin/caddy
COPY --from=docker.m.daocloud.io/library/caddy:2-alpine /etc/caddy /etc/caddy
COPY --from=rust-builder /app/target/release/kaede-server /usr/local/bin/kaede-server
COPY --from=web-builder /app/apps/web/dist /srv/web
COPY Caddyfile /etc/caddy/Caddyfile

RUN mkdir -p /data /config /srv/web

EXPOSE 8080

# 容器内始终监听 0.0.0.0:18080，公网只暴露 Caddy 的 8080。
ENV KAEDE_BIND=0.0.0.0:18080 \
    KAEDE_DB=sqlite:///data/kaede.db?mode=rwc \
    KAEDE_DATA_DIR=/data \
    XDG_DATA_HOME=/data \
    XDG_CONFIG_HOME=/config

# server 后台运行，Caddy 前台；Caddy 挂掉时容器退出由 restart 策略重建。
CMD ["sh", "-c", "kaede-server & exec caddy run --config /etc/caddy/Caddyfile --adapter caddyfile"]
