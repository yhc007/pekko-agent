//! OpenAPI 3.0 documentation — spec served at `/api/openapi.json`,
//! interactive Swagger UI at `/api/docs`.

use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

// ── Doc-only mirrors for types defined in external crates ────────────────────

/// pekko_agent_core::TokenUsage
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
pub(crate) struct TokenUsageDoc {
    pub(crate) input_tokens: u32,
    pub(crate) output_tokens: u32,
}

/// pekko_agent_core::AgentStatus
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
pub(crate) enum AgentStatusDoc { Available, Busy, Offline }

/// pekko_agent_core::AgentInfo
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
pub(crate) struct AgentInfoDoc {
    pub(crate) agent_id: String,
    pub(crate) agent_type: String,
    pub(crate) description: String,
    pub(crate) capabilities: Vec<String>,
    #[schema(value_type = AgentStatusDoc)]
    pub(crate) status: String,
}

/// pekko_agent_core::AgentProfile
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
pub(crate) struct AgentProfileDoc {
    pub(crate) tool_whitelist: Option<Vec<String>>,
    pub(crate) max_tokens_override: Option<u32>,
}

/// pekko_agent_core::MemoryDocument
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
pub(crate) struct MemoryDocumentDoc {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) source: String,
    pub(crate) agent_id: String,
}

/// pekko_agent_orchestrator::CollaborationResult (simplified)
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
pub(crate) struct CollaborationResultDoc {
    pub(crate) session_id: String,
    pub(crate) synthesis: String,
    pub(crate) total_in_tokens: u32,
    pub(crate) total_out_tokens: u32,
}

// ── Security modifier ─────────────────────────────────────────────────────────

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}

// ── OpenAPI document definition ───────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Pekko Agent API",
        version = "0.1.0",
        description = "EHS AI Agent Platform — REST API for multi-agent queries, \
                       collaboration, workflows, and long-term memory (RAG).",
        license(name = "MIT")
    ),
    paths(
        crate::health_check,
        crate::issue_token,
        crate::list_agents,
        crate::register_agent,
        crate::query_agent,
        crate::stream_query_agent,
        crate::get_session_history,
        crate::list_tools,
        crate::collaborate_agents,
        crate::memory_store,
        crate::memory_search,
        crate::memory_delete,
        crate::event_history,
    ),
    components(schemas(
        crate::AuthRequest,
        crate::AuthResponse,
        crate::QueryRequest,
        crate::QueryResponse,
        crate::HealthResponse,
        crate::ServiceStatus,
        crate::ErrorResponse,
        crate::MemoryStoreRequest,
        crate::MemoryStoreResponse,
        crate::MemorySearchRequest,
        crate::CollaborateRequest,
        crate::AgentRegistrationRequest,
        TokenUsageDoc,
        AgentInfoDoc,
        AgentProfileDoc,
        MemoryDocumentDoc,
        CollaborationResultDoc,
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "system",      description = "Health check and Prometheus metrics"),
        (name = "auth",        description = "Authentication — exchange API key for JWT"),
        (name = "agents",      description = "Agent listing, registration, and query execution"),
        (name = "collaborate", description = "Multi-agent parallel collaboration"),
        (name = "memory",      description = "Long-term vector memory (RAG store)"),
        (name = "events",      description = "System event history"),
    )
)]
pub struct ApiDoc;

// ── Axum routes ───────────────────────────────────────────────────────────────

/// Returns a stateless `Router` serving:
/// - `GET /api/docs` — Swagger UI
/// - `GET /api/openapi.json` — raw OpenAPI 3.0 spec
pub fn swagger_routes() -> axum::Router {
    axum::Router::from(
        SwaggerUi::new("/api/docs")
            .url("/api/openapi.json", ApiDoc::openapi()),
    )
}
