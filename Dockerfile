# =============================================================================
#  深华辉日化 ERP - 后端 Dockerfile（多阶段构建）
# =============================================================================
#  Builder  : rust:1.82-slim   编译二进制
#  Runtime  : debian:bookworm-slim 运行时（含 ODBC 18 + ca-certificates）
# =============================================================================

# ---------- Stage 1: 依赖缓存 ----------
FROM rust:1.82-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

# ---------- Stage 2: 配方计算 ----------
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---------- Stage 3: 编译 ----------
FROM chef AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --bin erp_server

# ---------- Stage 4: 运行时 ----------
FROM debian:bookworm-slim AS runtime

# 微软 ODBC 18 + SQL Server TLS CA 证书
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl gnupg \
    && curl -fsSL https://packages.microsoft.com/keys/microsoft.asc | gpg --dearmor -o /usr/share/keyrings/microsoft-prod.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/microsoft-prod.gpg] https://packages.microsoft.com/debian/12/prod bookworm main" > /etc/apt/sources.list.d/mssql-release.list \
    && apt-get update \
    && ACCEPT_EULA=Y apt-get install -y --no-install-recommends msodbcsql18 \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# 非 root 用户
RUN useradd -r -u 1001 -m -d /home/erp -s /bin/bash erp

WORKDIR /app
COPY --from=builder /app/target/release/erp_server /app/erp_server
COPY --from=builder /app/scripts /app/scripts

# 健康检查（容器内访问本机 8080）
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1

USER erp
EXPOSE 8080

ENV RUST_LOG=info \
    BIND_ADDR=0.0.0.0:8080

ENTRYPOINT ["/app/erp_server"]
