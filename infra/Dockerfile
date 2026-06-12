# ============================================================================
# pekko-agent Multi-Stage Docker Build
# ============================================================================
# 빌드:  docker build --build-arg SERVICE=api-gateway -t pekko-agent/api-gateway .
# 실행:  docker run -p 8080:8080 --env-file .env pekko-agent/api-gateway
# ============================================================================

# ── Stage 1: Builder ─────────────────────────────────────────────────────────
FROM rust:1.82-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Copy workspace members
COPY crates/   crates/
COPY services/ services/
COPY proto/    proto/

# Build argument: which service binary to produce
ARG SERVICE=api-gateway

# Release build (single service to minimize compile time)
RUN cargo build --release -p ${SERVICE} \
    && strip /app/target/release/${SERVICE}

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

ARG SERVICE=api-gateway
ENV SERVICE_NAME=${SERVICE}

COPY --from=builder /app/target/release/${SERVICE} /usr/local/bin/service

# Non-root user for security
RUN useradd --no-create-home --shell /bin/false pekko
USER pekko

ENV RUST_LOG=info
EXPOSE 8080

# Health check (api-gateway exposes /api/health; EHS agents don't bind HTTP)
HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -sf http://localhost:8080/api/health || exit 1

ENTRYPOINT ["/usr/local/bin/service"]
