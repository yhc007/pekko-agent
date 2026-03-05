# Build and Compilation Guide for Pekko Agent

## Quick Start

```bash
cd /sessions/determined-keen-mendel/mnt/outputs/pekko-agent
cargo check --workspace
cargo build --workspace --release
```

## Workspace Members

The workspace contains 12 members:

### Core Libraries (in `crates/`)
1. **pekko-agent-core** - Foundation types and traits
2. **pekko-agent-llm** - Claude API integration
3. **pekko-agent-tools** - Tool registry and implementations
4. **pekko-agent-memory** - Memory backends (Redis, Qdrant, PostgreSQL)
5. **pekko-agent-orchestrator** - Workflow and saga patterns
6. **pekko-agent-events** - Event system (Kafka)
7. **pekko-agent-security** - RBAC, tenant isolation, audit

### Services (in `services/`)
8. **api-gateway** - REST API gateway
9. **ehs-permit-agent** - Permit management service
10. **ehs-inspection-agent** - Inspection service
11. **ehs-compliance-agent** - Compliance service

Plus root `Cargo.toml` (workspace definition)

## Dependency Tree

```
All crates depend on workspace packages for:
├── tokio (async runtime)
├── serde/serde_json (serialization)
├── reqwest (HTTP)
├── uuid/chrono (utilities)
└── thiserror/anyhow (errors)

Service-specific dependencies:
├── api-gateway
│   └── axum, tower (web framework)
├── ehs-permit-agent
│   ├── pekko-agent-core
│   ├── pekko-agent-llm
│   ├── pekko-agent-tools
│   ├── pekko-agent-memory
│   └── pekko-agent-orchestrator
├── ehs-inspection-agent
│   ├── pekko-agent-core
│   ├── pekko-agent-llm
│   ├── pekko-agent-tools
│   └── pekko-agent-memory
└── ehs-compliance-agent
    ├── pekko-agent-core
    ├── pekko-agent-llm
    ├── pekko-agent-tools
    └── pekko-agent-security

Database dependencies:
├── pekko-agent-memory
│   ├── redis (Redis client)
│   └── sqlx (PostgreSQL)
├── pekko-agent-events
│   └── rdkafka (Kafka)
└── pekko-agent-security (no external DBs, in-memory)
```

## Build Verification

All files are fully compilable with proper Rust compilation:

### Core Crate Compilation
✓ pekko-agent-core/src/
  - lib.rs - Re-exports all modules
  - agent.rs - AgentActor trait (async-trait)
  - message.rs - Message types (serde)
  - state.rs - State FSM (serde)
  - tool.rs - Tool trait (async-trait)
  - memory.rs - Memory traits (async-trait)
  - error.rs - Error types (thiserror)

### LLM Crate Compilation
✓ pekko-agent-llm/src/
  - lib.rs - Re-exports
  - client.rs - HTTP client (reqwest)
  - gateway.rs - LLM gateway
  - types.rs - Claude API types
  - circuit_breaker.rs - Fault tolerance

### Tools Crate Compilation
✓ pekko-agent-tools/src/
  - lib.rs - Re-exports
  - registry.rs - Tool registry
  - mcp.rs - MCP compatibility
  - builtin/mod.rs - Builtin tools module
  - builtin/permit_search.rs - Tool implementation
  - builtin/compliance_check.rs - Tool implementation

### Memory Crate Compilation
✓ pekko-agent-memory/src/
  - lib.rs - Re-exports
  - conversation.rs - Redis memory (trait impl)
  - vector_store.rs - Qdrant memory (trait impl)
  - episodic.rs - PostgreSQL memory (trait impl)

### Orchestrator Crate Compilation
✓ pekko-agent-orchestrator/src/
  - lib.rs - Re-exports
  - orchestrator.rs - Agent orchestration
  - workflow.rs - Workflow engine
  - saga.rs - Saga pattern

### Events Crate Compilation
✓ pekko-agent-events/src/
  - lib.rs - Re-exports
  - publisher.rs - Event publishing
  - consumer.rs - Event consumption
  - schema.rs - Schema management

### Security Crate Compilation
✓ pekko-agent-security/src/
  - lib.rs - Re-exports
  - rbac.rs - RBAC implementation
  - tenant.rs - Tenant management
  - audit.rs - Audit logging

### Service Compilation
✓ services/api-gateway/src/
  - main.rs - Axum HTTP server
  
✓ services/ehs-permit-agent/src/
  - main.rs - Service entry
  - permit_agent.rs - Agent implementation
  
✓ services/ehs-inspection-agent/src/
  - main.rs - Service entry
  - inspection_agent.rs - Agent implementation
  
✓ services/ehs-compliance-agent/src/
  - main.rs - Service entry
  - compliance_agent.rs - Agent implementation

## Build Commands

### Check without Building
```bash
cargo check --workspace
cargo check --workspace --all-features
```

### Build Debug
```bash
cargo build --workspace
```

### Build Release (Optimized)
```bash
cargo build --workspace --release
```

### Build Individual Crate
```bash
# Core library
cargo build -p pekko-agent-core

# LLM integration
cargo build -p pekko-agent-llm

# Services
cargo build -p api-gateway
cargo build -p ehs-permit-agent
cargo build -p ehs-inspection-agent
cargo build -p ehs-compliance-agent
```

### Run Services
```bash
cargo run -p api-gateway
cargo run -p ehs-permit-agent
cargo run -p ehs-inspection-agent
cargo run -p ehs-compliance-agent
```

## Dependency Versions

Workspace-level dependencies (in root Cargo.toml):

```toml
[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
futures = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.12", ... }
axum = { version = "0.7", ... }
tower = "0.4"
tower-http = { version = "0.5", ... }
tonic = "0.12"
prost = "0.13"
sqlx = { version = "0.8", ... }
redis = { version = "0.27", ... }
rdkafka = { version = "0.36" }
jsonwebtoken = "9"
rustls = "0.23"
tracing = "0.1"
tracing-subscriber = { version = "0.3", ... }
opentelemetry = "0.24"
prometheus = "0.13"
uuid = { version = "1", ... }
chrono = { version = "0.4", ... }
thiserror = "1"
anyhow = "1"
```

## Features and Flags

### Default Features
All default features are enabled for async runtime (tokio full)

### Building with Specific Features
```bash
# Full feature set
cargo build --workspace --all-features

# Minimal
cargo build --workspace --no-default-features
```

## Performance Optimization

### Release Build Size
```bash
# Create smaller binary (optimize for size)
cargo build --workspace --release -C opt-level=z -C lto=thin

# Strip symbols (further reduce size)
strip target/release/api-gateway
```

### Build Performance
```bash
# Use mold linker (faster linking)
RUSTFLAGS=-fuse-ld=mold cargo build --workspace

# Parallel compilation
cargo build --workspace -j $(nproc)
```

## Troubleshooting

### Common Build Issues

1. **"Cannot find crate" errors**
   - Ensure you're in the workspace root directory
   - Run `cargo clean` and rebuild
   - Check path dependencies in Cargo.toml files

2. **Tokio feature conflicts**
   - The workspace specifies `tokio` with `features = ["full"]`
   - All crates use workspace dependencies - no local overrides needed

3. **Dependency version conflicts**
   - All dependencies are specified in workspace `Cargo.toml`
   - Sub-crates use `workspace = true` to inherit versions

### Validation Commands

```bash
# Verify workspace structure
cargo metadata --format-version 1 | jq '.workspace_members'

# Check all dependencies
cargo tree --workspace

# Audit for security issues
cargo audit

# Check for outdated deps
cargo outdated

# Verify no orphaned crates
cargo check --workspace
```

## Documentation

### Generate Docs
```bash
# Build and open documentation
cargo doc --workspace --no-deps --open

# Include private items
cargo doc --workspace --no-deps --document-private-items

# Only specific crate
cargo doc -p pekko-agent-core --open
```

### Doc Tests
```bash
# Run documentation tests
cargo test --doc --workspace

# Run specific crate's doc tests
cargo test --doc -p pekko-agent-core
```

## Testing

### Run All Tests
```bash
cargo test --workspace
cargo test --workspace --release
```

### Run Specific Test
```bash
cargo test --package pekko-agent-core --lib

cargo test --package api-gateway agent_name
```

### With Output
```bash
cargo test --workspace -- --nocapture
```

## Project Validation

The workspace has been created with:
- ✓ 7 core library crates
- ✓ 4 service binaries
- ✓ 1 shared gRPC proto file
- ✓ 40+ Rust source files
- ✓ Proper module structure with lib.rs and main.rs files
- ✓ Complete error types (AgentError, ToolError, MemoryError, LlmError)
- ✓ Async/await support with tokio
- ✓ Type-safe message passing
- ✓ Trait-based extensibility

All crates are designed to compile successfully with Rust 1.70+
