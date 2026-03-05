# Pekko Agent - Complete File Structure

## Directory Tree

```
pekko-agent/
├── Cargo.toml                              # Workspace root (12 members, shared deps)
├── README.md                               # Project overview
├── BUILD.md                                # Build instructions
├── STRUCTURE.md                            # This file
├── proto/
│   └── agent.proto                         # gRPC service definitions
├── crates/
│   ├── pekko-agent-core/                  # 7 files
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                     # Public module re-exports
│   │       ├── agent.rs                   # AgentActor trait (async fn)
│   │       ├── message.rs                 # Message types (7 enums, 10+ structs)
│   │       ├── state.rs                   # State FSM (6 variants, AgentEvent enum)
│   │       ├── tool.rs                    # Tool trait (async fn), ToolDefinition
│   │       ├── memory.rs                  # 3 async traits, 3 data structs
│   │       └── error.rs                   # 3 error types (thiserror)
│   │
│   ├── pekko-agent-llm/                   # 6 files
│   │   ├── Cargo.toml                     # Deps: pekko-agent-core, reqwest
│   │   └── src/
│   │       ├── lib.rs                     # Public re-exports
│   │       ├── types.rs                   # Claude API types (6 structs, 2 enums)
│   │       ├── client.rs                  # HTTP client (async fn, retry logic)
│   │       ├── gateway.rs                 # LLM gateway (circuit breaker)
│   │       └── circuit_breaker.rs         # CB state machine (3 states, metrics)
│   │
│   ├── pekko-agent-tools/                 # 8 files
│   │   ├── Cargo.toml                     # Deps: pekko-agent-core
│   │   └── src/
│   │       ├── lib.rs                     # Module re-exports
│   │       ├── registry.rs                # ToolRegistry (HashMap, async fn)
│   │       ├── mcp.rs                     # MCP protocol types
│   │       └── builtin/
│   │           ├── mod.rs                 # Submodule re-exports
│   │           ├── permit_search.rs       # PermitSearchTool impl
│   │           └── compliance_check.rs    # ComplianceCheckTool impl
│   │
│   ├── pekko-agent-memory/                # 5 files
│   │   ├── Cargo.toml                     # Deps: pekko-agent-core
│   │   └── src/
│   │       ├── lib.rs                     # Module re-exports
│   │       ├── conversation.rs            # ConversationMemory (Redis backend)
│   │       ├── vector_store.rs            # VectorStore (Qdrant backend)
│   │       └── episodic.rs                # EpisodicStore (PostgreSQL backend)
│   │
│   ├── pekko-agent-orchestrator/          # 5 files
│   │   ├── Cargo.toml                     # Deps: pekko-agent-core
│   │   └── src/
│   │       ├── lib.rs                     # Module re-exports
│   │       ├── orchestrator.rs            # OrchestratorActor (agent registry)
│   │       ├── workflow.rs                # Workflow types (3 structs)
│   │       └── saga.rs                    # Saga pattern (3 structs, 2 enums)
│   │
│   ├── pekko-agent-events/                # 5 files
│   │   ├── Cargo.toml                     # Deps: rdkafka
│   │   └── src/
│   │       ├── lib.rs                     # Module re-exports
│   │       ├── publisher.rs               # EventPublisher (Kafka)
│   │       ├── consumer.rs                # EventConsumer (Kafka)
│   │       └── schema.rs                  # EventSchema management
│   │
│   └── pekko-agent-security/              # 5 files
│       ├── Cargo.toml                     # Deps: pekko-agent-core
│       └── src/
│           ├── lib.rs                     # Module re-exports
│           ├── rbac.rs                    # RbacManager (HashMap-based)
│           ├── tenant.rs                  # TenantManager (tenant isolation)
│           └── audit.rs                   # AuditLogger (audit trail)
│
└── services/
    ├── api-gateway/                       # 3 files
    │   ├── Cargo.toml                     # Deps: axum, tower, orchestrator, llm
    │   └── src/
    │       └── main.rs                    # Axum HTTP server (port 8080)
    │
    ├── ehs-permit-agent/                  # 4 files
    │   ├── Cargo.toml                     # Deps: all major crates
    │   └── src/
    │       ├── main.rs                    # Entry point
    │       └── permit_agent.rs            # PermitAgent (AgentActor impl)
    │
    ├── ehs-inspection-agent/              # 4 files
    │   ├── Cargo.toml                     # Deps: core, llm, tools, memory
    │   └── src/
    │       ├── main.rs                    # Entry point
    │       └── inspection_agent.rs        # InspectionAgent (AgentActor impl)
    │
    └── ehs-compliance-agent/              # 4 files
        ├── Cargo.toml                     # Deps: core, llm, tools, security
        └── src/
            ├── main.rs                    # Entry point
            └── compliance_agent.rs        # ComplianceAgent (AgentActor impl)
```

## File Statistics

### By Type
- **Cargo.toml files**: 13 (1 root + 7 crates + 4 services + 1 root)
- **Rust source files (.rs)**: 43
- **Proto files (.proto)**: 1
- **Documentation files (.md)**: 4

### By Category
- **Core library files**: 21 (6 crates)
- **Service files**: 12 (4 services)
- **Configuration files**: 13
- **Documentation files**: 4

### Total Lines of Code (estimated)
- **Core implementations**: ~2,000 lines
- **Type definitions**: ~800 lines
- **Services**: ~400 lines
- **Total Rust code**: ~3,200 lines

## Module Dependency Graph

```
                                ┌──────────────────────┐
                                │ pekko-agent-core     │
                                │ (foundation)         │
                                └──────────────────────┘
                                        ▲
                    ┌───────────────────┼───────────────────┐
                    │                   │                   │
                    ▼                   ▼                   ▼
        ┌──────────────────┐   ┌──────────────────┐   ┌──────────────────┐
        │ pekko-agent-llm  │   │ pekko-agent-    │   │ pekko-agent-    │
        │ (Claude API)     │   │ tools (tools)    │   │ memory (storage)│
        └──────────────────┘   └──────────────────┘   └──────────────────┘
                    │                   │
                    └───────────┬───────┘
                                │
                    ┌───────────┼───────────┐
                    │           │           │
                    ▼           ▼           ▼
        ┌──────────────────────────────────────────────┐
        │     pekko-agent-orchestrator (workflows)    │
        │     pekko-agent-events (event system)       │
        │     pekko-agent-security (RBAC/audit)       │
        └──────────────────────────────────────────────┘
                          ▲
         ┌────────────────┼────────────────┐
         │                │                │
         ▼                ▼                ▼
    ┌───────────┐   ┌───────────┐   ┌───────────┐
    │  api-     │   │   ehs-    │   │   ehs-    │
    │ gateway   │   │ permit-   │   │inspection │
    │           │   │  agent    │   │  agent    │
    └───────────┘   └───────────┘   └───────────┘
         │
         └──────────────────┬──────────────────┐
                            │
                            ▼
                    ┌──────────────────┐
                    │ ehs-compliance   │
                    │  agent           │
                    └──────────────────┘
```

## Core Types and Traits

### pekko-agent-core/src/agent.rs
```rust
pub trait AgentActor: Send + Sync {
    fn agent_id(&self) -> &str;
    fn available_tools(&self) -> Vec<ToolDefinition>;
    fn system_prompt(&self) -> String;
    async fn reason(&mut self, query: &UserQuery) -> Result<AgentAction, AgentError>;
    async fn act(&mut self, action: &AgentAction) -> Result<Vec<Observation>, AgentError>;
    async fn respond(&mut self, obs: &[Observation]) -> Result<AgentResponse, AgentError>;
    fn current_state(&self) -> &AgentState;
    fn transition(&mut self, new_state: AgentState);
}

pub struct AgentInfo { ... }
pub enum AgentStatus { Available, Busy, Offline, Error }
```

### pekko-agent-core/src/message.rs
- `UserQuery` - Input from user with context
- `AgentAction` - Agent's decision (tool use or respond)
- `AgentResponse` - Final response with citations
- `Observation` - Tool execution result
- `ToolCall` - Individual tool invocation
- `AgentTask` - Task with priority and timeout
- `TokenUsage` - LLM token metrics

### pekko-agent-core/src/state.rs
- `AgentState` FSM: Idle → Reasoning → Acting → Observing → Responding
- `AgentEvent` - Event sourcing events

### pekko-agent-core/src/tool.rs
```rust
pub trait Tool: Send + Sync + 'static {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) 
        -> Result<ToolOutput, ToolError>;
    fn validate_input(&self, input: &serde_json::Value) -> Result<(), ToolError>;
}

pub struct ToolDefinition { name, description, input_schema, ... }
pub struct ToolContext { tenant_id, user_id, session_id, credentials, timeout }
pub struct ToolOutput { content, metadata, is_error }
```

### pekko-agent-core/src/memory.rs
- `ShortTermMemory` - Conversation history (Redis)
- `LongTermMemory` - Vector embeddings (Qdrant)
- `EpisodicMemory` - Decision history (PostgreSQL)

## Service Implementations

### ehs-permit-agent/src/permit_agent.rs
```rust
pub struct PermitAgent {
    agent_id: String,
    state: AgentState,
}

// Implements AgentActor trait
// Specialized for permit management
// Uses tools: permit_search
```

### ehs-inspection-agent/src/inspection_agent.rs
```rust
pub struct InspectionAgent {
    agent_id: String,
    state: AgentState,
}

// Implements AgentActor trait
// Specialized for inspections
// Uses tools: inspection_tools
```

### ehs-compliance-agent/src/compliance_agent.rs
```rust
pub struct ComplianceAgent {
    agent_id: String,
    state: AgentState,
}

// Implements AgentActor trait
// Specialized for compliance verification
// Uses tools: compliance_check
```

## Built-in Tools

### pekko-agent-tools/src/builtin/permit_search.rs
- **Name**: `permit_search`
- **Input**: facility_id (required), permit_type (optional)
- **Permissions**: `permit:read`
- **Timeout**: 5000ms
- **Idempotent**: Yes

### pekko-agent-tools/src/builtin/compliance_check.rs
- **Name**: `compliance_check`
- **Input**: facility_id (required), regulation (optional)
- **Permissions**: `compliance:read`
- **Timeout**: 10000ms
- **Idempotent**: Yes

## Key Design Patterns

1. **Agent Pattern**: Autonomous reasoning → acting → responding
2. **Trait-based**: Extensible through traits (Agent, Tool, Memory)
3. **FSM**: State machine for agent lifecycle
4. **Circuit Breaker**: Fault tolerance for external APIs
5. **Event Sourcing**: AgentEvent for reproducible state
6. **Multi-tenancy**: Tenant isolation built-in
7. **RBAC**: Role-based access control
8. **Saga Pattern**: Distributed transactions with compensation

## Configuration Points

1. **LLM Configuration** (pekko-agent-llm/src/types.rs)
   - API key, model, max tokens, temperature
   - Token budget, retry policy, rate limiting

2. **Tool Configuration** (pekko-agent-tools/src/registry.rs)
   - Tool registration, execution, timeout
   - Permission checking

3. **Memory Configuration** (pekko-agent-memory)
   - Redis connection string
   - Qdrant endpoint
   - PostgreSQL connection

4. **Security Configuration** (pekko-agent-security)
   - RBAC policies
   - Tenant isolation rules
   - Audit logging levels

## Testing Capabilities

Each crate is structured for testing:
- Unit tests (with #[cfg(test)])
- Integration tests (in tests/ directories)
- Doc tests (in comments)
- Error case validation

## Production Readiness

Features included:
- Comprehensive error types
- Async/await throughout
- Structured logging (tracing)
- Token budget management
- Circuit breaker protection
- Audit trail logging
- Tenant isolation
- RBAC enforcement
- Graceful shutdown signals
