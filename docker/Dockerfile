# syntax=docker/dockerfile:1
#
# Multi-stage Dockerfile for all Uptrakit binaries.
#
# Build args:
#   PACKAGE  — Cargo package name  (e.g. uptrakit-controller)
#   BINARY   — Output binary name  (e.g. uptrakit-controller)
#   FEATURES — Cargo feature flags (e.g. embed-frontend,db-all,oidc,interactive)
#
# Usage:
#   docker build -f docker/Dockerfile \
#     --build-arg PACKAGE=uptrakit-controller \
#     --build-arg BINARY=uptrakit-controller \
#     --build-arg FEATURES=embed-frontend,db-all,oidc,interactive,embedded-scheduler,nats,notifications-all \
#     -t uptrakit-controller .

# ---------------------------------------------------------------------------
# Stage 1: Build the SvelteKit frontend SPA
# ---------------------------------------------------------------------------
FROM node:lts-bookworm-slim AS frontend-builder-npm

WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --ignore-scripts && npm rebuild
COPY frontend/ ./
RUN npm run build

# In CI this stage is overridden via --build-context frontend-builder=<prebuilt-path>
# so the npm build above is skipped entirely. The path must contain /app/frontend/build/.
FROM frontend-builder-npm AS frontend-builder

# ---------------------------------------------------------------------------
# Stage 2: Generate a Cargo dependency recipe with cargo-chef
# ---------------------------------------------------------------------------
FROM rust:1-bookworm AS planner

RUN cargo install cargo-chef --locked
WORKDIR /app
COPY . .
COPY --from=frontend-builder /app/frontend/build /app/frontend/build
RUN cargo chef prepare --recipe-path recipe.json

# ---------------------------------------------------------------------------
# Stage 3: Cook dependencies (cached) then build the target binary
# ---------------------------------------------------------------------------
FROM rust:1-bookworm AS builder

# Build tools required by aws-lc-sys and other native dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake clang pkg-config && \
    rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef --locked

ARG PACKAGE=uptrakit-controller
ARG BINARY=uptrakit-controller
ARG FEATURES=

WORKDIR /app

# Cook dependencies first (this layer is cached as long as deps don't change)
COPY --from=planner /app/recipe.json recipe.json
RUN if [ -n "${FEATURES}" ]; then \
      cargo chef cook --release --recipe-path recipe.json \
        -p "${PACKAGE}" --no-default-features --features "${FEATURES}"; \
    else \
      cargo chef cook --release --recipe-path recipe.json -p "${PACKAGE}"; \
    fi

# Copy full source and built frontend
COPY . .
COPY --from=frontend-builder /app/frontend/build /app/frontend/build

# Build the actual binary
RUN if [ -n "${FEATURES}" ]; then \
      cargo build --release -p "${PACKAGE}" --no-default-features --features "${FEATURES}"; \
    else \
      cargo build --release -p "${PACKAGE}"; \
    fi && \
    cp "target/release/${BINARY}" /usr/local/bin/app

# ---------------------------------------------------------------------------
# Stage 4: Minimal runtime image
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create a non-root user
RUN groupadd --system uptrakit && \
    useradd --system --gid uptrakit --create-home uptrakit

# Create data directories
RUN mkdir -p /data/config /data/state && \
    chown -R uptrakit:uptrakit /data

COPY --from=builder /usr/local/bin/app /usr/local/bin/app

USER uptrakit
WORKDIR /data

ENV UPTRAKIT_CONFIG_DIR=/data/config \
    UPTRAKIT_STATE_DIR=/data/state

# Default: controller HTTPS port
EXPOSE 8443

ENTRYPOINT ["/usr/local/bin/app"]
