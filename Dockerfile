# syntax=docker/dockerfile:1
#
# LLMGate 容器镜像（多阶段）：
#   1. web     —— 构建 Vue 面板，产物写入 static/out
#   2. build   —— 用 nightly 工具链编译 lg 二进制（前端产物在编译期由 rust-embed 内嵌）
#   3. runtime —— debian-slim 运行镜像，只带 ca-certificates 与 tzdata
#
# 数据目录在镜像内固定为 /data，可用环境变量 LLMGATE_DATA_DIR 覆盖；
# 把它挂到宿主机任意路径即可完成持久化，见仓库根目录 docker-compose.yml。

# ---------- 1. 前端 ----------
FROM node:22-bookworm-slim AS web

RUN corepack enable && corepack prepare pnpm@10 --activate
WORKDIR /app/web
# 先只拷贝依赖清单：清单不变时 pnpm install 这层可命中缓存
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
# 产物路径由 web/vite.config.ts 固定为 ../static/out
RUN pnpm build

# ---------- 2. 后端 ----------
# nightly 工具链只在 rustlang/rust 发布（library/rust 仅有 stable 标签）。
# 与 rust-toolchain.toml 一样跟随滚动 nightly；想锁死可换成带日期的标签，
# 如 rustlang/rust:nightly-bookworm-2026-10-09。
FROM rustlang/rust:nightly-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs rust-toolchain.toml ./

# 先用空 main 编译一遍，把第三方依赖固化进这一层；
# 之后只改业务代码时，依赖无需重编（arm64 经 QEMU 模拟时这一层尤其值钱）。
RUN mkdir -p src \
 && echo 'fn main() {}' > src/main.rs \
 && cargo build --release --locked \
 && rm -rf src

COPY src ./src
COPY --from=web /app/static/out ./static/out
# COPY 会保留源文件的 mtime，而源码通常早于上一步 dummy 构建的产物；
# 不 touch 的话 cargo 会判定 dummy 构建仍然新鲜，直接跳过真实编译。
RUN find src -type f -exec touch {} + \
 && cargo build --release --locked \
 && cp target/release/lg /usr/local/bin/lg

# ---------- 3. 运行时 ----------
FROM debian:bookworm-slim AS runtime

# ca-certificates：上游 HTTPS 由 rustls 校验，需要系统根证书
# tzdata：统计按本地日期分桶（chrono::Local），TZ 生效依赖它
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/*

# 非 root 运行，UID/GID 固定 1000：挂载宿主机目录时需保证该属主
RUN useradd --uid 1000 --user-group --create-home --shell /usr/sbin/nologin llmgate \
 && mkdir -p /data \
 && chown llmgate:llmgate /data

COPY --from=build /usr/local/bin/lg /usr/local/bin/lg

# 默认数据目录；数据库文件位于 ${LLMGATE_DATA_DIR}/data.db（WAL 模式，另有 -wal/-shm）
ENV LLMGATE_DATA_DIR=/data
VOLUME ["/data"]
EXPOSE 8080
USER llmgate

ENTRYPOINT ["/usr/local/bin/lg"]
