# Pekko Agent - Complete Rust Workspace

A comprehensive multi-agent system framework built with Rust, featuring specialized agents for EHS (Environmental, Health, and Safety) management, powered by Claude AI.

## Workspace Structure

```
pekko-agent/
├── Cargo.toml                    # Workspace root
├── crates/                       # Core libraries
│   ├── pekko-agent-core/        # Base traits and types
│   ├── pekko-agent-llm/         # Claude API integration
│   ├── pekko-agent-tools/       # Tool registry & MCP
│   ├── pekko-agent-memory/      # Memory implementations
│   ├── pekko-agent-orchestrator/# Workflow & saga patterns
│   ├── pekko-agent-events/      # Event pub/sub system
│   └── pekko-agent-security/    # RBAC & tenant isolation
├── services/                     # Deployable services
│   ├── api-gateway/             # HTTP API endpoint
│   ├── ehs-permit-agent/        # Permit management agent
│   ├── ehs-inspection-agent/    # Inspection agent
│   └── ehs-compliance-agent/    # Compliance verification
├── proto/                        # gRPC definitions
└── README.md                     # This file
```

## Crates Overview

### pekko-agent-core (Foundation)
- **AgentActor trait**: Base interface for all agents with reasoning → acting → responding lifecycle
- **Message types**: UserQuery, AgentAction, AgentResponse, Observation
- **State FSM**: Idle → Reasoning → Acting → Observing → Responding
- **Tool trait**: Extensible tool interface with MCP compatibility
- **Memory traits**: ShortTermMemory, LongTermMemory, EpisodicMemory
- **Error types**: Comprehensive error handling with custom errors

**Key files:**
- `src/agent.rs` - AgentActor trait
- `src/message.rs` - Message types (UserQuery, AgentAction, etc.)
- `src/state.rs` - State machine and events
- `src/tool.rs` - Tool trait and execution
- `src/memory.rs` - Memory interfaces
- `src/error.rs` - Error types

### pekko-agent-llm (AI Integration)
- **ClaudeClient**: HTTP client for Claude API with retry logic
- **LlmGateway**: Central gateway with circuit breaker and token budgeting
- **LlmConfig**: Configuration management
- **CircuitBreaker**: Fault tolerance pattern implementation

**Key files:**
- `src/client.rs` - Claude API client
- `src/gateway.rs` - LLM gateway with circuit breaker
- `src/types.rs` - Claude API types and configurations
- `src/circuit_breaker.rs` - Circuit breaker implementation

### pekko-agent-tools (Tool Management)
- **ToolRegistry**: Central tool registry with execution support
- **MCP compatibility**: Standard tool definitions
- **Built-in tools**: PermitSearchTool, ComplianceCheckTool

**Key files:**
- `src/registry.rs` - Tool registry and execution
- `src/mcp.rs` - MCP protocol compatibility
- `src/builtin/permit_search.rs` - Permit search tool
- `src/builtin/compliance_check.rs` - Compliance checking tool

### pekko-agent-memory (Memory Management)
- **ConversationMemory**: Redis-based short-term memory
- **VectorStore**: Qdrant-based RAG (Retrieval-Augmented Generation)
- **EpisodicStore**: PostgreSQL-based decision history

**Key files:**
- `src/conversation.rs` - Redis conversation memory
- `src/vector_store.rs` - Qdrant vector database
- `src/episodic.rs` - PostgreSQL episodic memory

### pekko-agent-orchestrator (Orchestration)
- **OrchestratorActor**: Agent coordination and management
- **WorkflowEngine**: Task execution workflows
- **SagaPattern**: Distributed transaction support with compensations

**Key files:**
- `src/orchestrator.rs` - Agent orchestration
- `src/workflow.rs` - Workflow definitions and execution
- `src/saga.rs` - Saga pattern for distributed transactions

### pekko-agent-events (Event System)
- **EventPublisherActor**: Kafka-based event publishing
- **EventConsumer**: Message consumption
- **EventSchema**: Schema management

**Key files:**
- `src/publisher.rs` - Event publishing
- `src/consumer.rs` - Event consumption
- `src/schema.rs` - Event schema definitions

### pekko-agent-security (Security)
- **RbacManager**: Role-based access control
- **TenantManager**: Multi-tenant isolation
- **AuditLogger**: Comprehensive audit trailing

**Key files:**
- `src/rbac.rs` - RBAC implementation
- `src/tenant.rs` - Tenant management
- `src/audit.rs` - Audit logging

## Services

### api-gateway (Port 8080)
REST API gateway providing access to agent capabilities
- `/health` - Health check endpoint
- Future: Query, task assignment, and status endpoints

### ehs-permit-agent
Specialized agent for environmental permit management
- Implements AgentActor trait
- Manages permit searches and validations

### ehs-inspection-agent
Specialized agent for inspection management
- Conducts inspections
- Tracks inspection history

### ehs-compliance-agent
Specialized agent for compliance verification
- Checks compliance status
- Identifies violations
- Recommends remediation

## Building

### Prerequisites
- Rust 1.70+ (install from https://rustup.rs/)
- Cargo (comes with Rust)

### Build Commands

```bash
# Build all crates
cargo build --workspace

# Build in release mode
cargo build --workspace --release

# Check compilation without building
cargo check --workspace

# Run tests
cargo test --workspace

# Run specific service
cargo run --bin api-gateway
cargo run --bin ehs-permit-agent
cargo run --bin ehs-inspection-agent
cargo run --bin ehs-compliance-agent

# Build documentation
cargo doc --workspace --open
```

### Workspace Features

All crates use workspace dependencies from the root `Cargo.toml`:
- `tokio` - Async runtime (full features)
- `async-trait` - Async trait support
- `serde` - Serialization/deserialization
- `reqwest` - HTTP client
- `axum` - Web framework
- `tonic` - gRPC support
- `sqlx` - Database access
- `redis` - Redis client
- `rdkafka` - Kafka support
- `tracing` - Structured logging
- `uuid` & `chrono` - ID and time handling
- Plus 15+ other dependencies

## Project Layout Details

### pekko-agent-core Module Structure
```
pekko-agent-core/
├── lib.rs           - Re-exports all modules
├── agent.rs         - AgentActor trait & AgentInfo
├── message.rs       - Message types (7 message enums, 10+ structs)
├── state.rs         - AgentState FSM & AgentEvent
├── tool.rs          - Tool trait, ToolDefinition, ToolOutput
├── memory.rs        - 3 memory traits + supporting types
└── error.rs         - AgentError, ToolError, MemoryError
```

### Agent Lifecycle
```
UserQuery
    ↓
[REASONING] - LLM decides next action → AgentAction
    ↓
[ACTING] - Execute tool calls → Observations
    ↓
[OBSERVING] - Analyze results, decide if more needed
    ↓
[RESPONDING] - Generate final response → AgentResponse
    ↓
AgentResponse (to user)
```

## Key Features

1. **Modular Architecture**: Independent, reusable crates with clear dependencies
2. **Type Safety**: Strong typing with Rust's type system
3. **Async/Await**: Non-blocking operations with Tokio
4. **Error Handling**: Comprehensive error types with custom messages
5. **Extensibility**: Traits for agents, tools, and memory backends
6. **Multi-tenant**: Built-in tenant isolation
7. **Security**: RBAC and audit logging
8. **Observability**: Structured logging with tracing
9. **Fault Tolerance**: Circuit breaker pattern for API calls
10. **Event-driven**: Kafka-based event system

## Dependencies Summary

The workspace uses carefully selected dependencies:

### Core Async
- `tokio` v1 - Async runtime
- `async-trait` v0.1 - Async traits
- `futures` v0.3 - Future utilities

### HTTP & Web
- `reqwest` v0.12 - HTTP client
- `axum` v0.7 - Web framework
- `tower` v0.4 - Middleware

### Serialization
- `serde` v1 - Serialization framework
- `serde_json` v1 - JSON handling

### Database & Storage
- `sqlx` v0.8 - SQL query builder
- `redis` v0.27 - Redis client
- `rdkafka` v0.36 - Kafka producer/consumer

### Security & Auth
- `jsonwebtoken` v9 - JWT handling
- `rustls` v0.23 - TLS implementation

### Utilities
- `uuid` v1 - UUID generation
- `chrono` v0.4 - DateTime handling
- `thiserror` v1 - Error macros
- `anyhow` v1 - Error handling
- `tracing` v0.1 - Structured logging

## Development

### Common Tasks

```bash
# Format code
cargo fmt --all

# Run clippy linter
cargo clippy --workspace -- -D warnings

# Generate docs
cargo doc --workspace --no-deps --open

# Check for outdated dependencies
cargo outdated

# Audit dependencies for vulnerabilities
cargo audit
```

### Project Statistics

- **7 core libraries** (1,500+ lines of code)
- **4 domain-specific services** (agent implementations)
- **40+ Rust source files**
- **Full type coverage** with comprehensive error types
- **Production-ready patterns**: Circuit breaker, saga, RBAC

## Environment Configuration

Services expect these environment variables:

```bash
# LLM Configuration
export ANTHROPIC_API_KEY=your_key
export LLM_MODEL=claude-sonnet-4-20250514
export LLM_MAX_TOKENS=4096

# Database Configuration
export DATABASE_URL=postgresql://user:password@host/db
export REDIS_URL=redis://localhost:6379

# Kafka Configuration
export KAFKA_BROKERS=localhost:9092
export KAFKA_TOPIC=agent-events

# Service Configuration
export LOG_LEVEL=debug
export TELEMETRY_ENDPOINT=http://localhost:4317
```

## Next Steps

1. **Setup Development Environment**:
   - Install Rust and Cargo
   - Clone repository
   - Run `cargo check --workspace`

2. **Implement Agents**:
   - Extend `AgentActor` trait in services
   - Integrate with `LlmGateway`
   - Register tools with `ToolRegistry`

3. **Add Tools**:
   - Create new tool structs implementing `Tool` trait
   - Add to `pekko-agent-tools/src/builtin/`
   - Register in agent initialization

4. **Configure Storage**:
   - Setup PostgreSQL, Redis, Qdrant
   - Run migrations
   - Configure connection strings

5. **Deploy Services**:
   - Build release binaries
   - Configure environment variables
   - Deploy as containers or systemd services

## License

MIT License - See LICENSE file for details

## Contributing

Contributions welcome! Follow Rust conventions and include tests.
