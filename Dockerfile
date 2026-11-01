# Stage 1: 编译 Rust 后端与 WASM
FROM docker.m.daocloud.io/library/rust:1.91-slim AS rust-builder
WORKDIR /app

# 1. 替换 Debian apt 源
RUN find /etc/apt/ -type f -exec sed -i 's/deb.debian.org/mirrors.tuna.tsinghua.edu.cn/g' {} + \
    && find /etc/apt/ -type f -exec sed -i 's/security.debian.org/mirrors.tuna.tsinghua.edu.cn/g' {} + \
    && apt-get update && apt-get install -y curl git pkg-config libssl-dev

# 2. 升级 Node 到 22
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && apt-get install -y nodejs

# 3. 安装 wasm-pack
RUN curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

COPY . .
RUN rm -f rust-toolchain.toml

# 编译 WASM (强制清理宿主机的 node_modules)
WORKDIR /app/apps/web
RUN rm -rf node_modules
RUN npm config set registry https://registry.npmmirror.com
RUN npm install
RUN npm run build:wasm

# 编译 Rust 后端 (绕过 sqlx 版本限制)
WORKDIR /app
RUN cargo build -p kaede-server --release --features dev-mode --ignore-rust-version

# Stage 2: 编译前端 React
FROM docker.m.daocloud.io/library/node:22-slim AS web-builder
WORKDIR /app
RUN npm config set registry https://registry.npmmirror.com
COPY --from=rust-builder /app/apps/web/src/wasm/pkg ./apps/web/src/wasm/pkg
COPY apps/web ./apps/web
WORKDIR /app/apps/web
# 强制清理 Windows 环境带进来的 node_modules 污染
RUN rm -rf node_modules
RUN npm install && npm run build

# Stage 3: 运行环境
FROM docker.m.daocloud.io/library/caddy:alpine
COPY --from=rust-builder /app/target/release/kaede-server /usr/local/bin/kaede-server
COPY --from=web-builder /app/apps/web/dist /srv/web
COPY Caddyfile /etc/caddy/Caddyfile
RUN mkdir -p /data

EXPOSE 8080
CMD ["sh", "-c", "KAEDE_BIND=127.0.0.1:18080 KAEDE_DB=sqlite:///data/kaede.db?mode=rwc kaede-server & caddy run --config /etc/caddy/Caddyfile"]
