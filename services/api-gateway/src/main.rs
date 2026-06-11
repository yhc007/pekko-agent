use axum::{
    extract::{Path, State, Json},
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    routing::{get, post},
    Router,
    http::StatusCode,
    response::{IntoResponse, sse::{Event, KeepAlive, Sse}},
};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tower_http::timeout::TimeoutLayer;
use std::sync::Arc;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc, oneshot};
use futures::stream;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use tracing::{info, error};

use pekko_actor::ActorRef;
use pekko_agent_core::{TokenUsage, ShortTermMemory, AgentInfo, AgentStatus};
use pekko_agent_llm::{LlmConfig, ClaudeClient, GeminiClient, OpenAIClient};
use pekko_agent_tools::{ToolRegistry, builtin::{PermitSearchTool, ComplianceCheckTool, EhsQueryTool}};
use pekko_agent_memory::PgConversationStore;
use pekko_agent_orchestrator::{OrchestratorActor, OrchestratorDeps, OrchestratorMessage};
use pekko_agent_events::EventPublisher;
use pekko_agent_security::AuditLogger;
use sqlx::PgPool;

// ── Timeouts ──────────────────────────────────────────────────────────────────
// Applied to blocking routes only. Streaming routes (SSE, WS) have no
// response-body timeout since the connection is intentionally long-lived.
const BLOCKING_TIMEOUT: Duration = Duration::from_secs(60);

// ── Shared application state ──────────────────────────────────────────────────
#[derive(Clone)]
struct AppState {
    tool_registry:      Arc<RwLock<ToolRegistry>>,
    conversation_store: Arc<PgConversationStore>,
    orchestrator_ref:   ActorRef<OrchestratorActor>,
    event_publisher:    Arc<EventPublisher>,
    audit_logger:       Arc<AuditLogger>,
}

// ── Request / Response types ──────────────────────────────────────────────────
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

/// Incoming WebSocket message from the client.
#[derive(Deserialize)]
struct WsQueryRequest {
    content: String,
    #[serde(default)]
    session_id: Option<Uuid>,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
}

fn default_tenant() -> String { "default".to_string() }
fn default_user()   -> String { "anonymous".to_string() }

#[derive(Serialize)]
struct QueryResponse {
    session_id:  Uuid,
    agent_id:    String,
    response:    String,
    tools_used:  Vec<String>,
    token_usage: TokenUsage,
}

#[derive(Serialize)]
struct HealthResponse {
    status:   String,
    version:  String,
    services: ServiceStatus,
}

#[derive(Serialize)]
struct ServiceStatus {
    orchestrator:     String,
    tools_registered: usize,
    active_agents:    usize,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    code:  String,
}

// ── Error helpers ──────────────────────────────────────────────────────────────
fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    let msg = msg.into();
    error!("{}", msg);
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
        error: msg,
        code: "INTERNAL_ERROR".into(),
    }))
}

fn service_unavailable(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse {
        error: msg.into(),
        code: "ACTOR_UNAVAILABLE".into(),
    }))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/health
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let tool_count = state.tool_registry.read().await.list_tools().len();

    let (reply_tx, reply_rx) = oneshot::channel::<Vec<AgentInfo>>();
    let agent_count = if state.orchestrator_ref
        .tell(OrchestratorMessage::GetAgents { reply_to: reply_tx })
        .is_ok()
    {
        reply_rx.await.map(|v| v.len()).unwrap_or(0)
    } else {
        0
    };

    Json(HealthResponse {
        status:  "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        services: ServiceStatus {
            orchestrator:     "running".to_string(),
            tools_registered: tool_count,
            active_agents:    agent_count,
        },
    })
}

/// GET /api/agents
async fn list_agents(
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentInfo>>, (StatusCode, Json<ErrorResponse>)> {
    let (reply_tx, reply_rx) = oneshot::channel::<Vec<AgentInfo>>();
    state.orchestrator_ref
        .tell(OrchestratorMessage::GetAgents { reply_to: reply_tx })
        .map_err(|_| service_unavailable("Orchestrator actor unavailable"))?;
    let agents = reply_rx.await
        .map_err(|_| internal_error("Orchestrator did not reply"))?;
    Ok(Json(agents))
}

/// POST /api/agents/:agent_id/query  — blocking request-reply
async fn query_agent(
    Path(agent_id): Path<String>,
    State(state):   State<AppState>,
    Json(req):      Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<ErrorResponse>)> {
    let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);

    info!(agent_id = %agent_id, session_id = %session_id, "Agent query → OrchestratorActor");

    let event = pekko_agent_events::AgentEventEnvelope::new(
        "api-gateway",
        pekko_agent_events::event_types::TASK_ASSIGNED,
        &req.tenant_id,
        session_id,
        serde_json::json!({ "agent_id": agent_id, "content": req.content }),
    );
    let _ = state.event_publisher.publish(event).await;

    state.audit_logger.log(pekko_agent_security::AuditEntry {
        id:        Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        tenant_id: req.tenant_id.clone(),
        agent_id:  agent_id.clone(),
        action:    "query".to_string(),
        resource:  format!("agent/{agent_id}"),
        outcome:   pekko_agent_security::AuditOutcome::Success,
        details:   serde_json::json!({ "session_id": session_id }),
    }).await;

    let (reply_tx, reply_rx) = oneshot::channel();
    state.orchestrator_ref
        .tell(OrchestratorMessage::QueryAgent {
            agent_id:   agent_id.clone(),
            content:    req.content,
            session_id,
            tenant_id:  req.tenant_id,
            user_id:    req.user_id,
            reply_to:   reply_tx,
        })
        .map_err(|_| service_unavailable("Orchestrator actor unavailable"))?;

    let result = reply_rx.await
        .map_err(|_| internal_error("Orchestrator did not reply"))?
        .map_err(|e| internal_error(format!("Query failed: {e}")))?;

    Ok(Json(QueryResponse {
        session_id:  result.session_id,
        agent_id:    result.agent_id,
        response:    result.response,
        tools_used:  result.tools_used,
        token_usage: TokenUsage {
            input_tokens:  result.input_tokens,
            output_tokens: result.output_tokens,
        },
    }))
}

/// POST /api/agents/:agent_id/query/stream  — SSE streaming
///
/// Uses a bounded channel (256) so a slow browser can apply backpressure.
/// No TimeoutLayer here — the connection is intentionally long-lived.
async fn stream_query_agent(
    Path(agent_id): Path<String>,
    State(state):   State<AppState>,
    Json(req):      Json<QueryRequest>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);

    info!(agent_id = %agent_id, session_id = %session_id, "SSE stream → OrchestratorActor");

    // Bounded: if the browser stalls, the actor's emit_event().await yields,
    // creating natural backpressure instead of unbounded memory growth.
    let (event_tx, event_rx) = mpsc::channel::<String>(256);

    let result = state.orchestrator_ref.tell(OrchestratorMessage::StreamAgent {
        agent_id,
        content:   req.content,
        session_id,
        tenant_id: req.tenant_id,
        user_id:   req.user_id,
        event_tx:  event_tx.clone(),
    });

    if result.is_err() {
        let _ = event_tx.try_send(
            serde_json::json!({"type":"error","message":"Orchestrator actor unavailable"}).to_string()
        );
    }

    let stream = stream::unfold(event_rx, |mut rx| async move {
        rx.recv().await.map(|data| {
            (Ok::<Event, Infallible>(Event::default().data(data)), rx)
        })
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// GET /api/agents/:agent_id/ws  — WebSocket bidirectional streaming
///
/// Protocol:
///   Client → Server: JSON `{"content":"...", "session_id":"<uuid>", ...}`
///   Server → Client: same JSON event stream as SSE
///                    (thinking / tool_use / tool_result / text_chunk / done / error)
///
/// Multiple queries per connection are supported — send a new JSON message
/// after receiving `{"type":"done", ...}` from the previous one.
async fn ws_agent(
    Path(agent_id): Path<String>,
    State(state):   State<AppState>,
    ws:             WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_agent_handler(socket, state, agent_id))
}

async fn ws_agent_handler(mut socket: WebSocket, state: AppState, agent_id: String) {
    info!(agent_id = %agent_id, "WebSocket connection opened");

    loop {
        // Wait for a query from the client
        let msg = match socket.recv().await {
            Some(Ok(m))  => m,
            Some(Err(e)) => { error!(error = %e, "WS recv error"); break; }
            None         => break, // Connection closed
        };

        let text = match msg {
            Message::Text(t)  => t,
            Message::Close(_) => break,
            // axum 0.7 auto-responds to Ping with Pong; skip other frames
            _ => continue,
        };

        let req: WsQueryRequest = match serde_json::from_str(&text) {
            Ok(r)  => r,
            Err(e) => {
                let err = serde_json::json!({"type":"error","message": format!("Invalid JSON: {e}")}).to_string();
                let _ = socket.send(Message::Text(err)).await;
                continue;
            }
        };

        let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);
        let tenant_id  = req.tenant_id.unwrap_or_else(default_tenant);
        let user_id    = req.user_id.unwrap_or_else(default_user);

        info!(agent_id = %agent_id, session_id = %session_id, "WS query received");

        let (event_tx, mut event_rx) = mpsc::channel::<String>(256);

        let result = state.orchestrator_ref.tell(OrchestratorMessage::StreamAgent {
            agent_id:  agent_id.clone(),
            content:   req.content,
            session_id,
            tenant_id,
            user_id,
            event_tx,
        });

        if result.is_err() {
            let err = serde_json::json!({"type":"error","message":"Orchestrator actor unavailable"}).to_string();
            let _ = socket.send(Message::Text(err)).await;
            continue;
        }

        // Forward every event from the actor to the WebSocket client.
        // Stops early if the client disconnects.
        while let Some(data) = event_rx.recv().await {
            if socket.send(Message::Text(data)).await.is_err() {
                info!(agent_id = %agent_id, "WS client disconnected mid-stream");
                return;
            }
        }
    }

    info!(agent_id = %agent_id, "WebSocket connection closed");
}

/// GET /api/sessions/:session_id/history
async fn get_session_history(
    Path(session_id): Path<Uuid>,
    State(state):     State<AppState>,
) -> impl IntoResponse {
    let messages = state.conversation_store
        .get_conversation(&session_id)
        .await
        .unwrap_or_default();
    Json(messages)
}

/// GET /api/tools
async fn list_tools(State(state): State<AppState>) -> impl IntoResponse {
    let registry = state.tool_registry.read().await;
    Json(registry.list_tools())
}

// ── Main ──────────────────────────────────────────────────────────────────────
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .json()
        .init();

    info!("Starting pekko-agent API Gateway");

    // ── LLM config ────────────────────────────────────────────────────────────
    let llm_config = LlmConfig {
        api_key: std::env::var("CLAUDE_API_KEY")
            .expect("CLAUDE_API_KEY must be set"),
        model: std::env::var("CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
        max_tokens: std::env::var("CLAUDE_MAX_TOKENS")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(4096),
        ..LlmConfig::default()
    };

    let llm_provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "claude".to_string());
    info!(model = %llm_config.model, provider = %llm_provider, "LLM configured");

    let claude_client = Arc::new(ClaudeClient::new(llm_config.clone()));
    let gemini_client = std::env::var("GOOGLE_API_KEY").ok().map(|key| {
        Arc::new(GeminiClient::new(key, std::env::var("GEMINI_MODEL").ok()))
    });
    let openai_client = std::env::var("OPENAI_API_KEY").ok().map(|key| {
        Arc::new(OpenAIClient::new(key, std::env::var("OPENAI_MODEL").ok()))
    });

    // ── PostgreSQL ────────────────────────────────────────────────────────────
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://paulyu@localhost:5432/astgroup".to_string());
    let pg_pool = Arc::new(
        PgPool::connect(&database_url).await
            .expect("Failed to connect to PostgreSQL")
    );
    info!("Connected to PostgreSQL");

    PgConversationStore::migrate(&pg_pool).await
        .expect("Conversation store migration failed");

    let conversation_store = Arc::new(PgConversationStore::new(pg_pool.clone(), 100));

    // ── Tool registry ─────────────────────────────────────────────────────────
    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(Arc::new(PermitSearchTool::new(pg_pool.clone())));
    tool_registry.register(Arc::new(ComplianceCheckTool::new(pg_pool.clone())));
    tool_registry.register(Arc::new(EhsQueryTool::new(pg_pool.clone())));
    let tool_registry = Arc::new(RwLock::new(tool_registry));
    info!(tools = tool_registry.read().await.list_tools().len(), "Tool registry ready");

    // ── ActorSystem + OrchestratorActor ───────────────────────────────────────
    let actor_system = pekko_actor::ActorSystem::new("pekko-agent");

    let deps = OrchestratorDeps {
        conversation_store: conversation_store.clone(),
        tool_registry:      tool_registry.clone(),
        claude_client,
        gemini_client,
        openai_client,
        llm_config,
        llm_provider,
    };

    let orchestrator_ref = actor_system
        .spawn(OrchestratorActor::new(deps), "orchestrator").await
        .expect("Failed to spawn OrchestratorActor");

    // Register agents via Actor messages (actor is the single source of truth)
    let agents = [
        AgentInfo {
            agent_id:     "ehs-permit-agent".into(),
            agent_type:   "ehs".into(),
            description:  "위험작업 허가(PTW) 관리 에이전트".into(),
            capabilities: vec!["permit_search".into(), "ehs_query".into()],
            status:       AgentStatus::Available,
        },
        AgentInfo {
            agent_id:     "ehs-inspection-agent".into(),
            agent_type:   "ehs".into(),
            description:  "안전점검 및 위험성 평가 에이전트".into(),
            capabilities: vec!["ehs_query".into()],
            status:       AgentStatus::Available,
        },
        AgentInfo {
            agent_id:     "ehs-compliance-agent".into(),
            agent_type:   "ehs".into(),
            description:  "법규/규정 준수 확인 에이전트".into(),
            capabilities: vec!["compliance_check".into(), "ehs_query".into()],
            status:       AgentStatus::Available,
        },
    ];

    for agent in &agents {
        orchestrator_ref.tell(OrchestratorMessage::RegisterAgent(agent.clone()))
            .expect("Failed to send RegisterAgent");
    }
    info!(agents = agents.len(), "OrchestratorActor spawned in ActorSystem");

    // ── Infrastructure ────────────────────────────────────────────────────────
    let event_publisher = Arc::new(EventPublisher::new("pekko-agent", 1024));
    let audit_logger    = Arc::new(AuditLogger::new(10000));

    let state = AppState {
        tool_registry,
        conversation_store,
        orchestrator_ref,
        event_publisher,
        audit_logger,
    };

    // ── Router ────────────────────────────────────────────────────────────────
    //
    // Blocking routes → wrapped with TimeoutLayer (60 s).
    // Streaming routes (SSE, WS) → no response timeout; connection is long-lived.
    //
    let blocking = Router::new()
        .route("/api/health",                        get(health_check))
        .route("/api/agents",                        get(list_agents))
        .route("/api/agents/:agent_id/query",        post(query_agent))
        .route("/api/sessions/:session_id/history",  get(get_session_history))
        .route("/api/tools",                         get(list_tools))
        .layer(TimeoutLayer::new(BLOCKING_TIMEOUT));

    let streaming = Router::new()
        .route("/api/agents/:agent_id/query/stream", post(stream_query_agent))
        .route("/api/agents/:agent_id/ws",           get(ws_agent));

    let port: u16 = std::env::var("API_GATEWAY_PORT")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(8080);

    let app = Router::new()
        .merge(blocking)
        .merge(streaming)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "API Gateway listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
    info!("Graceful shutdown");
}
