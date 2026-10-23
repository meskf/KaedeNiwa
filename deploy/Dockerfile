# KaedeNiwa 服务端镜像
#
# 构建（仓库根目录）：
#   docker build -f deploy/Dockerfile -t kaede-server:0.0.1 .
#
# 只编译 kaede-server 及其依赖；不会拉入 _webrtc/、LiveKit 或桌面端工具链。
ARG RUST_VERSION=1.99-bookworm
FROM rust:${RUST_VERSION} AS builder
WORKDIR /src

# 保守并发：webrtc-sys / tao 在多核并发编译时偶发失败（本镜像不编译它们，
# 但仍保留该设置以匹配 CI 与 README 的构建约定）。
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates

RUN cargo build --release --locked -p kaede-server

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl sqlite3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /var/lib/kaede --shell /usr/sbin/nologin kaede \
    && mkdir -p /var/lib/kaede \
    && chown kaede:kaede /var/lib/kaede

COPY --from=builder /src/target/release/kaede-server /usr/local/bin/kaede-server

USER kaede
WORKDIR /var/lib/kaede

ENV KAEDE_BIND=0.0.0.0:18080 \
    KAEDE_DB=sqlite:///var/lib/kaede/kaede.db?mode=rwc \
    KAEDE_DATA_DIR=/var/lib/kaede/data \
    KAEDE_LOG_FORMAT=json

EXPOSE 18080

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:18080/ready || exit 1

ENTRYPOINT ["kaede-server"]
