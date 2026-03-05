# Multi-stage Rust build
FROM rust:1.78-slim as builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY crates/ crates/
COPY services/ services/
COPY proto/ proto/

ARG SERVICE=api-gateway

# Build workspace first for dependency caching
RUN cargo build --release -p ${SERVICE}

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

ARG SERVICE=api-gateway
COPY --from=builder /app/target/release/${SERVICE} /usr/local/bin/service

ENV RUST_LOG=info
EXPOSE 8080 50051 50052 50053 50054 50055 50056 50057 50058

CMD ["/usr/local/bin/service"]
