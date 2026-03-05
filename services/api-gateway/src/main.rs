use axum::{
    extract::{Path, State, Json},
    routing::{get, post},
    Router,
    http::StatusCode,
    response::IntoResponse,
};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use tracing::info;

use pekko_agent_core::{Message, TokenUsage, ShortTermMemory};
use pekko_agent_llm::LlmConfig;
use pekko_agent_tools::{ToolRegistry, builtin::{PermitSearchTool, ComplianceCheckTool}};
use pekko_agent_memory::{InMemoryConversationStore, InMemoryVectorStore, InMemoryEpisodicStore};
use pekko_agent_orchestrator::OrchestratorActor;
use pekko_agent_events::EventPublisher;
use pekko_agent_security::{RbacManager, TenantManager, AuditLogger};

/// Shared application state with all services
#[derive(Clone)]
struct AppState {
    tool_registry: Arc<RwLock<ToolRegistry>>,
    conversation_store: Arc<InMemoryConversationStore>,
    vector_store: Arc<InMemoryVectorStore>,
    episodic_store: Arc<InMemoryEpisodicStore>,
    orchestrator: Arc<RwLock<OrchestratorActor>>,
    event_publisher: Arc<EventPublisher>,
    rbac: Arc<RwLock<RbacManager>>,
    tenant_manager: Arc<RwLock<TenantManager>>,
    audit_logger: Arc<AuditLogger>,
    llm_config: LlmConfig,
}

/// Request payload for agent queries
#[derive(Deserialize)]
struct QueryRequest {
    content: String,
    #[serde(default)]
    session_id: Option<Uuid>,
    #[serde(default = "default_tenant")]
    tenant_id: String,
    #[serde(default = "default_user")]
    user_id: String,
}

fn default_tenant() -> String {
    "default".to_string()
}

fn default_user() -> String {
    "anonymous".to_string()
}

/// Response from agent query
#[derive(Serialize)]
struct QueryResponse {
    session_id: Uuid,
    agent_id: String,
    response: String,
    tools_used: Vec<String>,
    token_usage: TokenUsage,
}

/// Health check response
#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    services: ServiceStatus,
}

/// Status of individual services
#[derive(Serialize)]
struct ServiceStatus {
    orchestrator: String,
    tools_registered: usize,
    active_agents: usize,
}

/// Error response
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    code: String,
}

/// GET /api/health - Health check endpoint
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let tools = state.tool_registry.read().await;
    let orch = state.orchestrator.read().await;

    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        services: ServiceStatus {
            orchestrator: "running".to_string(),
            tools_registered: tools.list_tools().len(),
            active_agents: orch.list_agents().len(),
        },
    })
}

/// GET /api/agents - List all available agents
async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let orch = state.orchestrator.read().await;
    let agents: Vec<_> = orch.list_agents().into_iter().cloned().collect();
    Json(agents)
}

/// POST /api/agents/:agent_id/query - Submit a query to an agent
async fn query_agent(
    Path(agent_id): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<ErrorResponse>)> {
    let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);

    info!(
        agent_id = %agent_id,
        session_id = %session_id,
        content_len = req.content.len(),
        "Agent query received"
    );

    // Store the user message in conversation history
    let msg = Message::user(&req.content);
    let conv_store = state.conversation_store.clone();
    let _ = conv_store.append_message(&session_id, msg).await;

    // Publish task assigned event
    let event = pekko_agent_events::AgentEventEnvelope::new(
        "api-gateway",
        pekko_agent_events::event_types::TASK_ASSIGNED,
        &req.tenant_id,
        session_id,
        serde_json::json!({
            "agent_id": agent_id,
            "content": req.content,
        }),
    );
    let _ = state.event_publisher.publish(event).await;

    // Log audit entry
    state.audit_logger.log(pekko_agent_security::AuditEntry {
        id: Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        tenant_id: req.tenant_id.clone(),
        agent_id: agent_id.clone(),
        action: "query".to_string(),
        resource: format!("agent/{}", agent_id),
        outcome: pekko_agent_security::AuditOutcome::Success,
        details: serde_json::json!({"session_id": session_id}),
    }).await;

    // In production, this would invoke the actual agent via gRPC
    // For now, return a structured response
    let response_text = format!(
        "Agent '{}' received your query. Session: {}. \
         In production, this routes to the agent's ReAct loop via gRPC.",
        agent_id, session_id
    );

    // Store the assistant response
    let assistant_msg = Message::assistant(&response_text);
    let _ = conv_store.append_message(&session_id, assistant_msg).await;

    Ok(Json(QueryResponse {
        session_id,
        agent_id,
        response: response_text,
        tools_used: vec![],
        token_usage: TokenUsage::default(),
    }))
}

/// GET /api/sessions/:session_id/history - Get conversation history for a session
async fn get_session_history(
    Path(session_id): Path<Uuid>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let messages = state.conversation_store
        .get_conversation(&session_id)
        .await
        .unwrap_or_default();
    Json(messages)
}

/// GET /api/tools - List all available tools
async fn list_tools(State(state): State<AppState>) -> impl IntoResponse {
    let registry = state.tool_registry.read().await;
    Json(registry.list_tools())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing with JSON formatting
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .json()
        .init();

    info!("Starting pekko-agent API Gateway");

    // Initialize tool registry with built-in tools
    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(Arc::new(PermitSearchTool));
    tool_registry.register(Arc::new(ComplianceCheckTool));

    // Create application state
    let state = AppState {
        tool_registry: Arc::new(RwLock::new(tool_registry)),
        conversation_store: Arc::new(InMemoryConversationStore::new(100)),
        vector_store: Arc::new(InMemoryVectorStore::new()),
        episodic_store: Arc::new(InMemoryEpisodicStore::new()),
        orchestrator: Arc::new(RwLock::new(OrchestratorActor::new())),
        event_publisher: Arc::new(EventPublisher::new("pekko-agent", 1024)),
        rbac: Arc::new(RwLock::new(RbacManager::new())),
        tenant_manager: Arc::new(RwLock::new(TenantManager::new())),
        audit_logger: Arc::new(AuditLogger::new(10000)),
        llm_config: LlmConfig::default(),
    };

    // Build the router with all routes
    let app = Router::new()
        .route("/api/health", get(health_check))
        .route("/api/agents", get(list_agents))
        .route("/api/agents/:agent_id/query", post(query_agent))
        .route("/api/sessions/:session_id/history", get(get_session_history))
        .route("/api/tools", get(list_tools))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Bind to port and start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    info!("API Gateway listening on 0.0.0.0:8080");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Signal handler for graceful shutdown
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl+c");
    info!("Shutdown signal received, gracefully shutting down");
}
