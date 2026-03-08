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
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use tracing::{info, error, warn};

use pekko_agent_core::{Message, TokenUsage, ShortTermMemory, ToolContext, AgentInfo, AgentStatus};
use pekko_agent_llm::{LlmConfig, ClaudeClient, GeminiClient, OpenAIClient, LlmRequest, ClaudeMessage, ContentBlock, ClaudeTool};
use pekko_agent_tools::{ToolRegistry, builtin::{PermitSearchTool, ComplianceCheckTool, EhsQueryTool}};
use pekko_agent_memory::{InMemoryConversationStore, InMemoryVectorStore, InMemoryEpisodicStore};
use pekko_agent_orchestrator::OrchestratorActor;
use pekko_agent_events::EventPublisher;
use pekko_agent_security::{RbacManager, TenantManager, AuditLogger};
use sqlx::PgPool;

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
    claude_client: Arc<ClaudeClient>,
    gemini_client: Option<Arc<GeminiClient>>,
    openai_client: Option<Arc<OpenAIClient>>,
    llm_provider: String,
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

/// Build the system prompt based on the selected agent
fn build_system_prompt(agent_id: &str) -> String {
    let agent_desc = match agent_id {
        "ehs-permit-agent" => {
            "당신은 EHS(환경안전보건) 허가 관리 전문 AI 에이전트입니다.\n\
             역할: 위험작업 허가(PTW) 현황 조회/분석, 작업 허가 프로세스 안내, 위험작업 유형별 통계, 허가 승인/반려 현황 추적\n\
             주요 테이블: dangerousworkmanagement(위험작업허가), tbmmanagement(TBM), accidentfreemanagement(무재해)"
        }
        "ehs-inspection-agent" => {
            "당신은 EHS(환경안전보건) 점검 관리 전문 AI 에이전트입니다.\n\
             역할: 안전점검 일정/결과 조회, 점검 현황 분석/보고, 위험성 평가 관리, 아차사고 분석\n\
             주요 테이블: riskassessmentmanagement(위험성평가), nearmissmanagement(아차사고), safetyeducationmanagement(안전교육)"
        }
        "ehs-compliance-agent" => {
            "당신은 EHS(환경안전보건) 규정 준수 전문 AI 에이전트입니다.\n\
             역할: 법규/규정 준수 확인, MSDS 관리, 안전보건위원회 운영 현황, 컴플라이언스 보고서 지원\n\
             주요 테이블: msdsmanagement(MSDS), safetyhealthcommittee(안전보건위원회), employeeinfo(직원정보)"
        }
        _ => {
            "당신은 EHS(환경안전보건) 통합 AI 에이전트입니다.\n\
             안전, 보건, 환경 관련 질문에 답변할 수 있습니다."
        }
    };

    // 현재 날짜를 동적으로 주입
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let year = chrono::Local::now().format("%Y").to_string();

    format!(
        "{agent_desc}\n\n\
         [현재 날짜] 오늘은 {today} 입니다. \
         날짜 관련 쿼리 시 이 날짜를 기준으로 하세요. \
         \"이번 달\", \"올해\", \"최근\" 등의 표현은 이 날짜 기준으로 해석하세요.\n\
         \n\
         [절대 규칙] 도구 사용:\n\
         당신은 ehs_query 도구를 통해 PostgreSQL 데이터베이스에 직접 접근할 수 있습니다.\n\
         - 데이터가 필요한 질문을 받으면 반드시 ehs_query 도구를 호출하여 실제 데이터를 조회하세요.\n\
         - 절대로 SQL 쿼리 예시를 사용자에게 보여주지 마세요. 직접 도구를 호출하세요.\n\
         - 도구 호출이 실패하면, 오류를 분석하고 수정된 쿼리로 즉시 재시도하세요.\n\
         - \"기능이 제한되어 있습니다\", \"직접 조회할 수 없습니다\" 같은 말은 절대 하지 마세요.\n\
         \n\
         [필수] PostgreSQL 문법 (MySQL 문법 사용 금지):\n\
         - 날짜 추출: EXTRACT(YEAR FROM col), EXTRACT(MONTH FROM col)\n\
         - 문자열 비교: col ILIKE '%검색어%'\n\
         - 조건부 집계: COUNT(*) FILTER (WHERE 조건)\n\
         - 날짜 캐스팅: col::date, CURRENT_DATE\n\
         - 현재 날짜: CURRENT_DATE (오늘), CURRENT_DATE - INTERVAL '30 days' (30일 전)\n\
         - 올해 시작: '{year}-01-01'::date\n\
         - YEAR(), MONTH(), IFNULL(), NOW() 등 MySQL 전용 함수는 사용 금지\n\
         \n\
         [필수] 테이블 스키마 탐색 절차:\n\
         처음 접하는 테이블은 반드시 먼저 컬럼명을 확인하세요:\n\
         SELECT column_name, data_type FROM information_schema.columns WHERE table_name = '테이블명' ORDER BY ordinal_position\n\
         컬럼명을 확인한 후, 정확한 컬럼명으로 데이터를 조회하세요. 컬럼명을 추측하지 마세요.\n\
         \n\
         [필수] 쿼리 작성 규칙:\n\
         - SELECT *는 피하고, 필요한 컬럼만 선택하세요.\n\
         - 큰 테이블은 반드시 LIMIT을 사용하세요.\n\
         - 통계가 필요하면 COUNT, SUM, AVG 등 집계 함수를 활용하세요.\n\
         - 쿼리 오류 시 오류 메시지를 분석하여 컬럼명/타입을 수정하고 재시도하세요.\n\
         \n\
         응답 규칙:\n\
         - 한국어로 답변하세요.\n\
         - 반드시 실제 데이터 기반으로 정확한 수치를 제공하세요.\n\
         - 필요시 개선 제안을 추가하세요.\n\
         \n\
         [필수] 응답 형식:\n\
         - 조회 결과가 여러 행이면 반드시 마크다운 테이블로 정리하세요.\n\
         - 테이블 형식: | 컬럼1 | 컬럼2 | ... |\\n|---|---|...|\\n| 값1 | 값2 | ... |\n\
         - 통계 요약도 테이블로 보여주세요.\n\
         - 테이블 앞뒤에 간단한 설명을 붙이세요.\n\
         - 숫자는 천 단위 쉼표를 사용하세요 (예: 1,234).\n\
         - 날짜는 YYYY-MM-DD 형식으로 표시하세요."
    )
}

/// Convert ToolDefinition to ClaudeTool format
fn tool_definitions_to_claude_tools(tools: &[pekko_agent_core::ToolDefinition]) -> Vec<ClaudeTool> {
    tools
        .iter()
        .map(|t| ClaudeTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect()
}

/// Maximum number of tool use rounds to prevent infinite loops
const MAX_TOOL_ROUNDS: usize = 10;

/// Maximum characters for tool result content to stay within Claude's context limits
const MAX_TOOL_RESULT_CHARS: usize = 30_000;

/// Truncate tool result content if it exceeds the limit
fn truncate_tool_result(content: &str) -> String {
    if content.len() <= MAX_TOOL_RESULT_CHARS {
        content.to_string()
    } else {
        let truncated = &content[..MAX_TOOL_RESULT_CHARS];
        format!(
            "{}...\n\n[결과가 너무 큽니다. {}자 중 {}자만 표시됩니다. 더 구체적인 SELECT 컬럼이나 WHERE 조건을 사용해주세요.]",
            truncated,
            content.len(),
            MAX_TOOL_RESULT_CHARS
        )
    }
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

    // Build conversation history for LLM context
    let history = conv_store
        .get_conversation(&session_id)
        .await
        .unwrap_or_default();

    let claude_messages: Vec<ClaudeMessage> = history
        .iter()
        .map(|m| ClaudeMessage {
            role: match m.role {
                pekko_agent_core::MessageRole::User => "user".to_string(),
                pekko_agent_core::MessageRole::Assistant => "assistant".to_string(),
                _ => "user".to_string(),
            },
            content: vec![ContentBlock::Text { text: m.content.clone() }],
        })
        .collect();

    // Get tool definitions from registry and convert to Claude format
    let tool_defs = {
        let registry = state.tool_registry.read().await;
        registry.list_tools()
    };
    let claude_tools = tool_definitions_to_claude_tools(&tool_defs);

    info!(tool_count = claude_tools.len(), "Sending tools to Claude");

    // Build system prompt
    let system_prompt = build_system_prompt(&agent_id);

    // Tool use loop - keep calling Claude until it gives a final text response
    let mut messages = claude_messages;
    let mut all_tools_used: Vec<String> = Vec::new();
    let mut total_input_tokens: u32 = 0;
    let mut total_output_tokens: u32 = 0;
    let mut final_text = String::new();

    for round in 0..MAX_TOOL_ROUNDS {
        let llm_request = LlmRequest {
            system_prompt: system_prompt.clone(),
            messages: messages.clone(),
            tools: claude_tools.clone(),
            max_tokens: state.llm_config.max_tokens,
            temperature: Some(state.llm_config.temperature),
            cacheable: false,
        };

        // Call LLM based on provider
        let resp = if state.llm_provider == "openai" {
            if let Some(ref openai) = state.openai_client {
                match openai.send_message(&llm_request).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        error!(error = %e, round = round, "OpenAI call failed");
                        final_text = format!("죄송합니다. AI 응답 생성 중 오류가 발생했습니다: {}", e);
                        break;
                    }
                }
            } else {
                error!("OpenAI client not configured");
                final_text = "OpenAI API 키가 설정되지 않았습니다.".to_string();
                break;
            }
        } else if state.llm_provider == "gemini" {
            if let Some(ref gemini) = state.gemini_client {
                match gemini.send_message(&llm_request).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        error!(error = %e, round = round, "Gemini call failed");
                        final_text = format!("죄송합니다. AI 응답 생성 중 오류가 발생했습니다: {}", e);
                        break;
                    }
                }
            } else {
                error!("Gemini client not configured");
                final_text = "Gemini API 키가 설정되지 않았습니다.".to_string();
                break;
            }
        } else {
            match state.claude_client.send_message(&llm_request).await {
                Ok(resp) => resp,
                Err(e) => {
                    error!(error = %e, round = round, "Claude call failed");
                    final_text = format!("죄송합니다. AI 응답 생성 중 오류가 발생했습니다: {}", e);
                    break;
                }
            }
        };

        total_input_tokens += resp.usage.input_tokens;
        total_output_tokens += resp.usage.output_tokens;

        info!(
            round = round,
            stop_reason = %resp.stop_reason,
            content_blocks = resp.content.len(),
            "Claude response received"
        );

        // Check if Claude wants to use tools
        if resp.stop_reason == "tool_use" {
            // Extract text and tool_use blocks from response
            let mut assistant_content: Vec<ContentBlock> = Vec::new();
            let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new();

            for block in &resp.content {
                match block {
                    ContentBlock::Text { text } => {
                        assistant_content.push(ContentBlock::Text { text: text.clone() });
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        assistant_content.push(ContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        });
                        tool_uses.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

            // Add assistant message with tool_use to history
            messages.push(ClaudeMessage {
                role: "assistant".to_string(),
                content: assistant_content,
            });

            // Execute each tool and collect results
            let mut tool_results: Vec<ContentBlock> = Vec::new();
            let tool_ctx = ToolContext {
                tenant_id: req.tenant_id.clone(),
                user_id: req.user_id.clone(),
                session_id,
                credentials: HashMap::new(),
                timeout: Duration::from_secs(10),
            };

            for (tool_use_id, tool_name, tool_input) in &tool_uses {
                info!(tool_name = %tool_name, tool_use_id = %tool_use_id, "Executing tool");
                all_tools_used.push(tool_name.clone());

                let result = {
                    let mut registry = state.tool_registry.write().await;
                    registry.execute(tool_name, tool_input.clone(), &tool_ctx).await
                };

                let (content_str, is_error) = match result {
                    Ok(output) => {
                        let content = serde_json::to_string(&output.content)
                            .unwrap_or_else(|_| "{}".to_string());
                        let content = truncate_tool_result(&content);
                        info!(tool_name = %tool_name, is_error = output.is_error, content_len = content.len(), "Tool executed");
                        (content, if output.is_error { Some(true) } else { None })
                    }
                    Err(e) => {
                        warn!(tool_name = %tool_name, error = %e, "Tool execution failed");
                        (format!("{{\"error\": \"{}\"}}", e), Some(true))
                    }
                };

                tool_results.push(ContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: content_str,
                    is_error,
                });
            }

            // Add tool results as user message
            messages.push(ClaudeMessage {
                role: "user".to_string(),
                content: tool_results,
            });

            // Continue loop to get Claude's next response
            continue;
        }

        // stop_reason is "end_turn" or other — extract final text
        for block in &resp.content {
            if let ContentBlock::Text { text } = block {
                if !final_text.is_empty() {
                    final_text.push('\n');
                }
                final_text.push_str(text);
            }
        }
        break;
    }

    // Store the assistant response
    let assistant_msg = Message::assistant(&final_text);
    let _ = conv_store.append_message(&session_id, assistant_msg).await;

    // Deduplicate tools_used
    all_tools_used.sort();
    all_tools_used.dedup();

    Ok(Json(QueryResponse {
        session_id,
        agent_id,
        response: final_text,
        tools_used: all_tools_used,
        token_usage: TokenUsage {
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
        },
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
    // Load .env file
    dotenvy::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        )
        .json()
        .init();

    info!("Starting pekko-agent API Gateway");

    // Build LLM config from environment
    let llm_config = LlmConfig {
        api_key: std::env::var("CLAUDE_API_KEY")
            .expect("CLAUDE_API_KEY must be set in .env or environment"),
        model: std::env::var("CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
        max_tokens: std::env::var("CLAUDE_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4096),
        ..LlmConfig::default()
    };

    info!(model = %llm_config.model, "LLM configured");

    // Create Claude client
    let claude_client = Arc::new(ClaudeClient::new(llm_config.clone()));

    // Connect to PostgreSQL database
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://paulyu@localhost:5432/astgroup".to_string());

    let pg_pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Set DATABASE_URL env var.");

    info!("Connected to PostgreSQL database");
    let pg_pool = Arc::new(pg_pool);

    // Initialize tool registry with built-in tools
    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(Arc::new(PermitSearchTool));
    tool_registry.register(Arc::new(ComplianceCheckTool));
    tool_registry.register(Arc::new(EhsQueryTool::new(pg_pool.clone())));

    info!(tools = tool_registry.list_tools().len(), "Tool registry initialized");

    // Create orchestrator and register agents
    let orchestrator = OrchestratorActor::new();
    let orchestrator = Arc::new(RwLock::new(orchestrator));

    // Register EHS agents
    {
        let mut orch = orchestrator.write().await;
        orch.register_agent(AgentInfo {
            agent_id: "ehs-permit-agent".to_string(),
            agent_type: "ehs".to_string(),
            description: "위험작업 허가(PTW) 관리 에이전트".to_string(),
            capabilities: vec!["permit_search".to_string(), "ehs_query".to_string()],
            status: AgentStatus::Available,
        });
        orch.register_agent(AgentInfo {
            agent_id: "ehs-inspection-agent".to_string(),
            agent_type: "ehs".to_string(),
            description: "안전점검 및 위험성 평가 에이전트".to_string(),
            capabilities: vec!["ehs_query".to_string()],
            status: AgentStatus::Available,
        });
        orch.register_agent(AgentInfo {
            agent_id: "ehs-compliance-agent".to_string(),
            agent_type: "ehs".to_string(),
            description: "법규/규정 준수 확인 에이전트".to_string(),
            capabilities: vec!["compliance_check".to_string(), "ehs_query".to_string()],
            status: AgentStatus::Available,
        });
        info!("Registered {} EHS agents", orch.list_agents().len());
    }

    // Determine LLM provider
    let llm_provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "claude".to_string());
    info!(provider = %llm_provider, "LLM provider selected");

    // Create Gemini client if API key available
    let gemini_client = std::env::var("GOOGLE_API_KEY").ok().map(|key| {
        let model = std::env::var("GEMINI_MODEL").ok();
        Arc::new(GeminiClient::new(key, model))
    });

    // Create OpenAI client if API key available
    let openai_client = std::env::var("OPENAI_API_KEY").ok().map(|key| {
        let model = std::env::var("OPENAI_MODEL").ok();
        Arc::new(OpenAIClient::new(key, model))
    });

    // Create application state
    let state = AppState {
        tool_registry: Arc::new(RwLock::new(tool_registry)),
        conversation_store: Arc::new(InMemoryConversationStore::new(100)),
        vector_store: Arc::new(InMemoryVectorStore::new()),
        episodic_store: Arc::new(InMemoryEpisodicStore::new()),
        orchestrator,
        event_publisher: Arc::new(EventPublisher::new("pekko-agent", 1024)),
        rbac: Arc::new(RwLock::new(RbacManager::new())),
        tenant_manager: Arc::new(RwLock::new(TenantManager::new())),
        audit_logger: Arc::new(AuditLogger::new(10000)),
        llm_config,
        claude_client,
        gemini_client,
        openai_client,
        llm_provider,
    };

    // Build the router
    let port: u16 = std::env::var("API_GATEWAY_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);

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
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("API Gateway listening on {}", addr);

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
