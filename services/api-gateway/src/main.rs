use axum::{
    async_trait,
    extract::{FromRequestParts, Path, State, Json},
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{request::Parts, StatusCode, header, Request},
    middleware::{self, Next},
    routing::{get, post},
    Router,
    response::{IntoResponse, sse::{Event, KeepAlive, Sse}},
};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tower_http::timeout::TimeoutLayer;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc, oneshot};
use futures::stream;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use tracing::{info, warn, error};

use pekko_actor::ActorRef;
use pekko_agent_core::{TokenUsage, ShortTermMemory, AgentInfo, AgentProfile, AgentStatus};
use pekko_agent_llm::{LlmConfig, ClaudeClient, GeminiClient, OpenAIClient};
use pekko_agent_tools::{ToolRegistry, builtin::{PermitSearchTool, ComplianceCheckTool, EhsQueryTool}};
use pekko_agent_memory::{PgConversationStore, PgVectorStore};
use pekko_agent_core::{LongTermMemory, MemoryDocument};
use pekko_agent_llm::EmbeddingClient;
use pekko_agent_observability::{MetricsRegistry, TracerProvider};
use pekko_agent_orchestrator::{
    OrchestratorActor, OrchestratorDeps, OrchestratorMessage,
    OrchestratorPersistence,
    Workflow, WorkflowResult,
};
use pekko_agent_events::EventPublisher;
use pekko_agent_security::{
    AuditLogger, AuditEntry, AuditOutcome, Claims, JwtError, JwtManager, RbacManager,
    RateLimiter, RateLimitConfig,
};
use pekko_actor::{CircuitBreaker, CircuitBreakerBuilder};
use sqlx::PgPool;

// ── Constants ─────────────────────────────────────────────────────────────────
const BLOCKING_TIMEOUT: Duration = Duration::from_secs(60);

// ── API key entry (replaces a user-store for now) ─────────────────────────────
#[derive(Clone)]
struct ApiKeyEntry {
    user_id:   String,
    tenant_id: String,
    roles:     Vec<String>,
}

// ── Shared application state ──────────────────────────────────────────────────
#[derive(Clone)]
struct AppState {
    tool_registry:      Arc<RwLock<ToolRegistry>>,
    conversation_store: Arc<PgConversationStore>,
    orchestrator_ref:   ActorRef<OrchestratorActor>,
    event_publisher:    Arc<EventPublisher>,
    audit_logger:       Arc<AuditLogger>,
    jwt_manager:        Arc<JwtManager>,
    rbac:               Arc<RwLock<RbacManager>>,
    /// API-key → user identity map.  Keyed by the raw key string.
    api_keys:           Arc<HashMap<String, ApiKeyEntry>>,
    vector_store:       Option<Arc<PgVectorStore>>,
    metrics:            Arc<MetricsRegistry>,
    rate_limiter:       Arc<RateLimiter>,
}

// ── JWT extractor ─────────────────────────────────────────────────────────────

/// Extracted from a valid `Authorization: Bearer <token>` header.
/// Adding this as a parameter to a handler makes that route require auth.
struct AuthUser(Claims);

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = (StatusCode, Json<ErrorResponse>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let raw = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| unauthorized("Missing Authorization header"))?;

        let token = raw
            .strip_prefix("Bearer ")
            .ok_or_else(|| unauthorized("Authorization header must be 'Bearer <token>'"))?;

        let claims = state.jwt_manager.validate(token).map_err(|e| match e {
            JwtError::Expired    => unauthorized("Token has expired — please re-authenticate"),
            JwtError::Invalid(_) => unauthorized("Token is invalid"),
            JwtError::Encode(_)  => unreachable!(),
        })?;

        Ok(AuthUser(claims))
    }
}

// ── RBAC helper ───────────────────────────────────────────────────────────────

async fn require_permission(
    state: &AppState,
    claims: &Claims,
    permission: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if state.rbac.read().await.check_user_permission(&claims.roles, permission) {
        Ok(())
    } else {
        warn!(
            user = %claims.sub,
            tenant = %claims.tenant_id,
            permission,
            roles = ?claims.roles,
            "Permission denied"
        );
        Err(forbidden(format!(
            "Role(s) {:?} do not grant '{permission}'",
            claims.roles
        )))
    }
}

// ── Request / Response types ──────────────────────────────────────────────────

/// POST /api/auth/token
#[derive(Deserialize)]
struct AuthRequest {
    api_key:   String,
    /// Optional override; defaults to the key's registered tenant.
    #[serde(default)]
    tenant_id: Option<String>,
}

#[derive(Serialize)]
struct AuthResponse {
    token:      String,
    token_type: &'static str,
    expires_in: u64,
    tenant_id:  String,
    user_id:    String,
    roles:      Vec<String>,
}

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

fn make_error(status: StatusCode, code: &str, msg: impl Into<String>)
    -> (StatusCode, Json<ErrorResponse>)
{
    (status, Json(ErrorResponse { error: msg.into(), code: code.to_string() }))
}

fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    let msg = msg.into();
    error!("{}", msg);
    make_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", msg)
}

fn service_unavailable(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    make_error(StatusCode::SERVICE_UNAVAILABLE, "ACTOR_UNAVAILABLE", msg)
}

fn unauthorized(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    make_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg)
}

fn forbidden(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    make_error(StatusCode::FORBIDDEN, "FORBIDDEN", msg)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

// ── Rate limit middleware ─────────────────────────────────────────────────────

async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    // Skip rate limiting for public endpoints
    let path = req.uri().path();
    if path == "/metrics" || path == "/api/health" || path == "/api/auth/token" {
        return next.run(req).await.into_response();
    }

    // Extract claims from Authorization header (best-effort; auth middleware runs later)
    let (roles, tenant_id) = if let Some(auth_header) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        match state.jwt_manager.validate(auth_header) {
            Ok(c) => (c.roles, c.tenant_id),
            Err(_) => (vec![], "anonymous".to_string()),
        }
    } else {
        (vec![], "anonymous".to_string())
    };

    if let Err(e) = state.rate_limiter.check(&tenant_id, &roles) {
        state.metrics.rate_limit_rejections
            .with_label_values(&[&tenant_id])
            .inc();
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [
                ("Retry-After", e.retry_after_secs.to_string()),
                ("X-RateLimit-Limit", e.limit.to_string()),
            ],
            Json(ErrorResponse {
                error: e.to_string(),
                code: "RATE_LIMIT_EXCEEDED".to_string(),
            }),
        ).into_response();
    }

    next.run(req).await.into_response()
}

// ── HTTP metrics middleware ────────────────────────────────────────────────────

/// Classify a full path into a stable label (avoids high-cardinality IDs).
fn path_label(path: &str) -> &'static str {
    if path.starts_with("/api/agents/") && path.ends_with("/query/stream") {
        "/api/agents/:id/query/stream"
    } else if path.starts_with("/api/agents/") && path.ends_with("/query") {
        "/api/agents/:id/query"
    } else if path.starts_with("/api/agents/") && path.ends_with("/ws") {
        "/api/agents/:id/ws"
    } else if path.starts_with("/api/sessions/") {
        "/api/sessions/:id/history"
    } else if path.starts_with("/api/workflows/") && path != "/api/workflows/stream" {
        "/api/workflows/:id"
    } else if path.starts_with("/api/memory/") && !path.ends_with("/store") && !path.ends_with("/search") {
        "/api/memory/:id"
    } else {
        match path {
            "/api/health"             => "/api/health",
            "/api/auth/token"         => "/api/auth/token",
            "/api/agents"             => "/api/agents",
            "/api/agents/register"    => "/api/agents/register",
            "/api/tools"              => "/api/tools",
            "/api/workflows"          => "/api/workflows",
            "/api/workflows/stream"   => "/api/workflows/stream",
            "/api/memory/store"       => "/api/memory/store",
            "/api/memory/search"      => "/api/memory/search",
            "/metrics"                => "/metrics",
            _                         => "other",
        }
    }
}

async fn http_metrics_middleware(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    let method = req.method().to_string();
    let path   = path_label(req.uri().path());
    let start  = std::time::Instant::now();

    let resp = next.run(req).await;

    let status = resp.status().as_u16().to_string();
    let elapsed = start.elapsed().as_secs_f64();

    state.metrics.http_requests_total
        .with_label_values(&[&method, path, &status])
        .inc();
    state.metrics.http_duration_secs
        .with_label_values(&[&method, path])
        .observe(elapsed);

    resp
}

/// GET /metrics  — Prometheus scrape endpoint (public, no auth)
async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    match state.metrics.render() {
        Ok(body) => (
            [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
            body,
        ).into_response(),
        Err(e) => internal_error(format!("Metrics render error: {e}")).into_response(),
    }
}

/// GET /api/health  — public, no auth required
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

/// POST /api/auth/token  — public; exchange API key for a JWT
async fn issue_token(
    State(state): State<AppState>,
    Json(req):    Json<AuthRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, Json<ErrorResponse>)> {
    let entry = state.api_keys.get(&req.api_key)
        .ok_or_else(|| unauthorized("Invalid API key"))?;

    let tenant_id = req.tenant_id
        .unwrap_or_else(|| entry.tenant_id.clone());

    let token = state.jwt_manager
        .issue(&entry.user_id, &tenant_id, entry.roles.clone())
        .map_err(|e| internal_error(format!("Token issuance failed: {e}")))?;

    info!(user = %entry.user_id, tenant = %tenant_id, "JWT issued");

    Ok(Json(AuthResponse {
        token,
        token_type: "Bearer",
        expires_in: state.jwt_manager.token_ttl_seconds,
        tenant_id,
        user_id: entry.user_id.clone(),
        roles:   entry.roles.clone(),
    }))
}

/// GET /api/agents  — requires memory.read
async fn list_agents(
    auth:         AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentInfo>>, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "memory.read").await?;

    let (reply_tx, reply_rx) = oneshot::channel::<Vec<AgentInfo>>();
    state.orchestrator_ref
        .tell(OrchestratorMessage::GetAgents { reply_to: reply_tx })
        .map_err(|_| service_unavailable("Orchestrator actor unavailable"))?;
    let agents = reply_rx.await
        .map_err(|_| internal_error("Orchestrator did not reply"))?;
    Ok(Json(agents))
}

/// POST /api/agents/:agent_id/query  — requires agent.delegate
async fn query_agent(
    auth:           AuthUser,
    Path(agent_id): Path<String>,
    State(state):   State<AppState>,
    Json(req):      Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "agent.delegate").await?;

    let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);
    let user_id    = auth.0.sub.clone();
    let tenant_id  = auth.0.tenant_id.clone();

    info!(
        agent_id = %agent_id, session_id = %session_id,
        user = %user_id, tenant = %tenant_id,
        "Agent query → OrchestratorActor"
    );

    let event = pekko_agent_events::AgentEventEnvelope::new(
        "api-gateway",
        pekko_agent_events::event_types::TASK_ASSIGNED,
        &tenant_id,
        session_id,
        serde_json::json!({ "agent_id": agent_id, "content": req.content, "user_id": user_id }),
    );
    let _ = state.event_publisher.publish(event).await;

    state.audit_logger.log(AuditEntry {
        id:        Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        tenant_id: tenant_id.clone(),
        agent_id:  agent_id.clone(),
        action:    "query".to_string(),
        resource:  format!("agent/{agent_id}"),
        outcome:   AuditOutcome::Success,
        details:   serde_json::json!({ "session_id": session_id, "user_id": user_id }),
    }).await;

    let (reply_tx, reply_rx) = oneshot::channel();
    state.orchestrator_ref
        .tell(OrchestratorMessage::QueryAgent {
            agent_id:   agent_id.clone(),
            content:    req.content,
            session_id,
            tenant_id,
            user_id,
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

/// POST /api/agents/:agent_id/query/stream  — requires agent.delegate, SSE
async fn stream_query_agent(
    auth:           AuthUser,
    Path(agent_id): Path<String>,
    State(state):   State<AppState>,
    Json(req):      Json<QueryRequest>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "agent.delegate").await?;

    let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);
    let tenant_id  = auth.0.tenant_id.clone();
    let user_id    = auth.0.sub.clone();

    info!(agent_id = %agent_id, session_id = %session_id, user = %user_id, "SSE stream → OrchestratorActor");

    let (event_tx, event_rx) = mpsc::channel::<String>(256);

    let result = state.orchestrator_ref.tell(OrchestratorMessage::StreamAgent {
        agent_id,
        content:   req.content,
        session_id,
        tenant_id,
        user_id,
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

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// GET /api/agents/:agent_id/ws  — requires agent.delegate, WebSocket
///
/// Token is passed as query param `?token=<jwt>` since browsers cannot set
/// Authorization headers on WebSocket upgrades.
async fn ws_agent(
    Path(agent_id): Path<String>,
    State(state):   State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Validate JWT from query param (browsers can't set WS headers)
    let token = match params.get("token") {
        Some(t) => t.clone(),
        None    => return unauthorized("Missing 'token' query parameter").into_response(),
    };

    let claims = match state.jwt_manager.validate(&token) {
        Ok(c)  => c,
        Err(JwtError::Expired)    => return unauthorized("Token has expired").into_response(),
        Err(JwtError::Invalid(_)) => return unauthorized("Invalid token").into_response(),
        Err(JwtError::Encode(_))  => unreachable!(),
    };

    // RBAC check before upgrade
    let has_perm = state.rbac.read().await
        .check_user_permission(&claims.roles, "agent.delegate");
    if !has_perm {
        return forbidden(format!(
            "Role(s) {:?} do not grant 'agent.delegate'", claims.roles
        )).into_response();
    }

    ws.on_upgrade(move |socket| ws_agent_handler(socket, state, agent_id, claims))
}

async fn ws_agent_handler(
    mut socket:  WebSocket,
    state:       AppState,
    agent_id:    String,
    claims:      Claims,
) {
    info!(agent_id = %agent_id, user = %claims.sub, "WebSocket connection opened");
    state.metrics.active_ws_connections.inc();

    loop {
        let msg = match socket.recv().await {
            Some(Ok(m))  => m,
            Some(Err(e)) => { error!(error = %e, "WS recv error"); break; }
            None         => break,
        };

        let text = match msg {
            Message::Text(t)  => t,
            Message::Close(_) => break,
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
        let tenant_id  = req.tenant_id.unwrap_or_else(|| claims.tenant_id.clone());
        let user_id    = req.user_id.unwrap_or_else(|| claims.sub.clone());

        info!(agent_id = %agent_id, session_id = %session_id, user = %user_id, "WS query received");

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

        while let Some(data) = event_rx.recv().await {
            if socket.send(Message::Text(data)).await.is_err() {
                info!(agent_id = %agent_id, "WS client disconnected mid-stream");
                return;
            }
        }
    }

    state.metrics.active_ws_connections.dec();
    info!(agent_id = %agent_id, user = %claims.sub, "WebSocket connection closed");
}

/// GET /api/sessions/:session_id/history  — requires memory.read
async fn get_session_history(
    auth:              AuthUser,
    Path(session_id):  Path<Uuid>,
    State(state):      State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "memory.read").await?;

    let messages = state.conversation_store
        .get_conversation(&session_id)
        .await
        .unwrap_or_default();
    Ok(Json(messages))
}

/// GET /api/tools  — requires memory.read
async fn list_tools(
    auth:         AuthUser,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "memory.read").await?;

    let registry = state.tool_registry.read().await;
    Ok(Json(registry.list_tools()))
}

// ── Long-term memory (vector store) endpoints ────────────────────────────────

#[derive(Deserialize)]
struct MemoryStoreRequest {
    id:       Option<String>,
    content:  String,
    source:   String,
    #[serde(default = "default_tenant")]
    agent_id: String,
    #[serde(default)]
    metadata: std::collections::HashMap<String, String>,
}

#[derive(Serialize)]
struct MemoryStoreResponse { doc_id: String }

#[derive(Deserialize)]
struct MemorySearchRequest {
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
}
fn default_top_k() -> usize { 5 }

/// POST /api/memory/store  — requires admin.all
async fn memory_store(
    auth:         AuthUser,
    State(state): State<AppState>,
    Json(req):    Json<MemoryStoreRequest>,
) -> Result<Json<MemoryStoreResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "admin.all").await?;

    let vs = state.vector_store.as_ref()
        .ok_or_else(|| make_error(StatusCode::SERVICE_UNAVAILABLE, "NO_VECTOR_STORE",
            "Vector store not configured"))?;

    let doc = MemoryDocument {
        id:       req.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        content:  req.content,
        source:   req.source,
        agent_id: req.agent_id,
        metadata: req.metadata,
    };
    let doc_id = vs.store(doc)
        .await
        .map_err(|e| internal_error(format!("Memory store error: {e}")))?;

    Ok(Json(MemoryStoreResponse { doc_id }))
}

/// POST /api/memory/search  — requires agent.delegate
async fn memory_search(
    auth:         AuthUser,
    State(state): State<AppState>,
    Json(req):    Json<MemorySearchRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "agent.delegate").await?;

    let vs = state.vector_store.as_ref()
        .ok_or_else(|| make_error(StatusCode::SERVICE_UNAVAILABLE, "NO_VECTOR_STORE",
            "Vector store not configured"))?;

    let results = vs.search(&req.query, req.top_k)
        .await
        .map_err(|e| internal_error(format!("Memory search error: {e}")))?;

    Ok(Json(results))
}

/// DELETE /api/memory/:doc_id  — requires admin.all
async fn memory_delete(
    auth:           AuthUser,
    Path(doc_id):   Path<String>,
    State(state):   State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "admin.all").await?;

    let vs = state.vector_store.as_ref()
        .ok_or_else(|| make_error(StatusCode::SERVICE_UNAVAILABLE, "NO_VECTOR_STORE",
            "Vector store not configured"))?;

    vs.delete(&doc_id)
        .await
        .map_err(|e| match e {
            pekko_agent_core::MemoryError::NotFound(m) =>
                make_error(StatusCode::NOT_FOUND, "NOT_FOUND", m),
            other => internal_error(other.to_string()),
        })?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Workflow endpoints ────────────────────────────────────────────────────────

/// POST /api/workflows  — execute a workflow synchronously; requires agent.delegate
async fn execute_workflow(
    auth:         AuthUser,
    State(state): State<AppState>,
    Json(workflow): Json<Workflow>,
) -> Result<Json<WorkflowResult>, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "agent.delegate").await?;

    info!(
        workflow_id = %workflow.id,
        name = %workflow.name,
        steps = workflow.steps.len(),
        user = %auth.0.sub,
        "ExecuteWorkflow → OrchestratorActor"
    );

    let (reply_tx, reply_rx) = oneshot::channel();
    state.orchestrator_ref
        .tell(OrchestratorMessage::ExecuteWorkflow {
            workflow,
            reply_to: reply_tx,
        })
        .map_err(|_| service_unavailable("Orchestrator actor unavailable"))?;

    let result = reply_rx.await
        .map_err(|_| internal_error("Orchestrator did not reply"))?
        .map_err(|e| internal_error(format!("Workflow failed: {e}")))?;

    Ok(Json(result))
}

/// POST /api/workflows/stream  — stream workflow execution events via SSE
async fn stream_workflow(
    auth:           AuthUser,
    State(state):   State<AppState>,
    Json(workflow): Json<Workflow>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "agent.delegate").await?;

    info!(
        workflow_id = %workflow.id,
        name = %workflow.name,
        user = %auth.0.sub,
        "StreamWorkflow → OrchestratorActor"
    );

    let (event_tx, event_rx) = mpsc::channel::<String>(256);

    let result = state.orchestrator_ref.tell(OrchestratorMessage::StreamWorkflow {
        workflow,
        event_tx: event_tx.clone(),
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

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// GET /api/workflows/:workflow_id  — get current workflow status
async fn get_workflow_status(
    auth:               AuthUser,
    Path(workflow_id):  Path<Uuid>,
    State(state):       State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "memory.read").await?;

    let (reply_tx, reply_rx) = oneshot::channel();
    state.orchestrator_ref
        .tell(OrchestratorMessage::GetWorkflowStatus {
            workflow_id,
            reply_to: reply_tx,
        })
        .map_err(|_| service_unavailable("Orchestrator actor unavailable"))?;

    let status = reply_rx.await
        .map_err(|_| internal_error("Orchestrator did not reply"))?;

    match status {
        Some(s) => Ok(Json(s).into_response()),
        None => Err(make_error(StatusCode::NOT_FOUND, "NOT_FOUND",
            format!("Workflow {workflow_id} not found"))),
    }
}

// ── Agent registration endpoint ───────────────────────────────────────────────

#[derive(Deserialize)]
struct AgentRegistrationRequest {
    info:    AgentInfo,
    profile: AgentProfile,
}

/// POST /api/agents/register  — requires admin role
///
/// Called by EHS microservices at startup to register their AgentInfo
/// and declare their tool whitelist / token limits.
async fn register_agent(
    auth:         AuthUser,
    State(state): State<AppState>,
    Json(req):    Json<AgentRegistrationRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    require_permission(&state, &auth.0, "admin.all").await?;

    info!(
        agent_id = %req.info.agent_id,
        tool_whitelist = ?req.profile.tool_whitelist,
        "Agent self-registration"
    );

    state.orchestrator_ref
        .tell(OrchestratorMessage::RegisterAgent {
            info:    req.info,
            profile: req.profile,
        })
        .map_err(|_| service_unavailable("Orchestrator actor unavailable"))?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Main ──────────────────────────────────────────────────────────────────────
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // Tracing: JSON logs + optional OTLP export (set OTLP_ENDPOINT to enable)
    let _otel_provider: Option<TracerProvider> =
        pekko_agent_observability::tracing::init("api-gateway");

    info!("Starting pekko-agent API Gateway");

    // Prometheus metrics
    let metrics = MetricsRegistry::new()
        .expect("Failed to create MetricsRegistry");
    info!("Prometheus metrics registry initialised");

    // ── JWT + Auth ────────────────────────────────────────────────────────────
    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| {
            warn!("JWT_SECRET not set — using insecure default (set it in production!)");
            "change-me-in-production-32-chars!!".to_string()
        });

    let jwt_ttl: u64 = std::env::var("JWT_TTL_SECONDS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(3600);

    let jwt_manager = Arc::new(JwtManager::new(jwt_secret.as_bytes()).with_ttl(jwt_ttl));

    // API keys — keyed by raw key string.
    // Production: store hashed keys in DB.
    let mut api_keys: HashMap<String, ApiKeyEntry> = HashMap::new();

    if let Ok(key) = std::env::var("ADMIN_API_KEY") {
        api_keys.insert(key, ApiKeyEntry {
            user_id:   "admin".to_string(),
            tenant_id: "default".to_string(),
            roles:     vec!["admin".to_string()],
        });
    } else {
        warn!("ADMIN_API_KEY not set — no admin access possible");
    }

    if let Ok(key) = std::env::var("AGENT_API_KEY") {
        api_keys.insert(key, ApiKeyEntry {
            user_id:   "agent-service".to_string(),
            tenant_id: "default".to_string(),
            roles:     vec!["agent".to_string()],
        });
    }

    let api_keys = Arc::new(api_keys);
    let rbac     = Arc::new(RwLock::new(RbacManager::new()));

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

    // ── Rate limiter ──────────────────────────────────────────────────────────
    let rl_config = RateLimitConfig {
        admin_rpm:   std::env::var("RATE_LIMIT_ADMIN_RPM")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(1000),
        agent_rpm:   std::env::var("RATE_LIMIT_AGENT_RPM")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(300),
        default_rpm: std::env::var("RATE_LIMIT_DEFAULT_RPM")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(60),
    };
    let rate_limiter = Arc::new(RateLimiter::new(rl_config));
    info!(
        admin_rpm = rl_config.admin_rpm,
        agent_rpm = rl_config.agent_rpm,
        default_rpm = rl_config.default_rpm,
        "Rate limiter initialised"
    );

    // ── Circuit breakers (one per LLM provider) ───────────────────────────────
    let cb_timeout = Duration::from_secs(
        std::env::var("CB_CALL_TIMEOUT_SECS")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(30)
    );
    let cb_reset = Duration::from_secs(
        std::env::var("CB_RESET_TIMEOUT_SECS")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(60)
    );
    let cb_max_failures: u32 = std::env::var("CB_MAX_FAILURES")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(5);

    let make_cb = || {
        CircuitBreakerBuilder::default()
            .max_failures(cb_max_failures)
            .call_timeout(cb_timeout)
            .reset_timeout(cb_reset)
            .exponential_backoff(2.0)
            .build()
    };

    let mut circuit_breakers = std::collections::HashMap::new();
    circuit_breakers.insert("claude".to_string(),  make_cb());
    circuit_breakers.insert("gemini".to_string(),  make_cb());
    circuit_breakers.insert("openai".to_string(),  make_cb());
    info!(
        max_failures = cb_max_failures,
        call_timeout_secs = cb_timeout.as_secs(),
        reset_timeout_secs = cb_reset.as_secs(),
        "Circuit breakers initialised"
    );

    // ── Vector store (long-term memory / RAG) ────────────────────────────────
    let embedder = std::env::var("OPENAI_API_KEY").ok().map(|key| {
        let model = std::env::var("EMBEDDING_MODEL").ok();
        Arc::new(EmbeddingClient::new(key, model)) as Arc<dyn pekko_agent_core::Embedder>
    });

    let vector_store: Option<Arc<PgVectorStore>> = if std::env::var("DISABLE_VECTOR_STORE")
        .map(|v| v == "true").unwrap_or(false)
    {
        info!("Vector store disabled by DISABLE_VECTOR_STORE=true");
        None
    } else {
        match PgVectorStore::migrate(&pg_pool).await {
            Ok(()) => {
                let has_embedder = embedder.is_some();
                let vs = Arc::new(PgVectorStore::new(pg_pool.clone(), embedder));
                info!(has_embedder, "PgVectorStore ready");
                Some(vs)
            }
            Err(e) => {
                warn!(error = %e, "pgvector migration failed — vector store disabled. \
                    Install the pgvector extension to enable RAG.");
                None
            }
        }
    };

    // ── Orchestrator persistence ──────────────────────────────────────────────
    let orchestrator_persistence = if std::env::var("DISABLE_ORCHESTRATOR_PERSISTENCE")
        .map(|v| v == "true").unwrap_or(false)
    {
        info!("Orchestrator persistence disabled by DISABLE_ORCHESTRATOR_PERSISTENCE=true");
        None
    } else {
        match OrchestratorPersistence::migrate(&pg_pool).await {
            Ok(()) => {
                let store = OrchestratorPersistence::new(pg_pool.clone());
                info!("OrchestratorPersistence ready");
                Some(store)
            }
            Err(e) => {
                warn!(error = %e, "Orchestrator persistence migration failed — running without persistence");
                None
            }
        }
    };

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
        vector_store:       vector_store.clone(),
        metrics:            Some(metrics.clone()),
        circuit_breakers,
        persistence:        orchestrator_persistence,
    };

    let orchestrator_ref = actor_system
        .spawn(OrchestratorActor::new(deps), "orchestrator").await
        .expect("Failed to spawn OrchestratorActor");

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
        orchestrator_ref.tell(OrchestratorMessage::RegisterAgent {
            info:    agent.clone(),
            profile: AgentProfile::default(), // EHS services override this at startup
        }).expect("Failed to send RegisterAgent");
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
        jwt_manager,
        rbac,
        api_keys,
        vector_store,
        metrics,
        rate_limiter,
    };

    // ── Router ────────────────────────────────────────────────────────────────
    //
    // Public:    /api/health, /api/auth/token
    // Protected: everything else
    //
    // Blocking routes wrapped in TimeoutLayer(60s).
    // Streaming routes (SSE, WS) have no response timeout.
    //
    let public_routes = Router::new()
        .route("/api/health",      get(health_check))
        .route("/api/auth/token",  post(issue_token))
        .route("/metrics",         get(prometheus_metrics)); // Prometheus scrape

    let blocking_protected = Router::new()
        .route("/api/agents",                        get(list_agents))
        .route("/api/agents/register",               post(register_agent))
        .route("/api/agents/:agent_id/query",        post(query_agent))
        .route("/api/sessions/:session_id/history",  get(get_session_history))
        .route("/api/tools",                         get(list_tools))
        .route("/api/workflows",                     post(execute_workflow))
        .route("/api/workflows/:workflow_id",        get(get_workflow_status))
        .route("/api/memory/store",                  post(memory_store))
        .route("/api/memory/search",                 post(memory_search))
        .route("/api/memory/:doc_id",                axum::routing::delete(memory_delete))
        .layer(TimeoutLayer::new(BLOCKING_TIMEOUT));

    let streaming_protected = Router::new()
        .route("/api/agents/:agent_id/query/stream", post(stream_query_agent))
        .route("/api/agents/:agent_id/ws",           get(ws_agent))
        .route("/api/workflows/stream",              post(stream_workflow));

    let port: u16 = std::env::var("API_GATEWAY_PORT")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(8080);

    let app = Router::new()
        .merge(public_routes)
        .merge(blocking_protected)
        .merge(streaming_protected)
        // Rate limiting runs before metrics so rejected requests are still counted
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit_middleware))
        .layer(middleware::from_fn_with_state(state.clone(), http_metrics_middleware))
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
    // OTel flush is handled by _otel_provider drop at end of main()
}
