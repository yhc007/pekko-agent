//! OrchestratorActor — wired into the real pekko_actor::ActorSystem.
//!
//! Usage:
//! ```rust,ignore
//! let system = ActorSystem::new("ehs-system");
//! let orch   = OrchestratorActor::new(deps);
//! let orch_ref = system.spawn(orch, "orchestrator").await?;
//!
//! // Query (request-reply via oneshot)
//! let (tx, rx) = tokio::sync::oneshot::channel();
//! orch_ref.tell(OrchestratorMessage::QueryAgent { agent_id: "ehs-permit-agent".into(), ..., reply_to: tx })?;
//! let result = rx.await??;
//!
//! // Stream (events via mpsc)
//! let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
//! orch_ref.tell(OrchestratorMessage::StreamAgent { ..., event_tx: tx })?;
//! while let Some(json) = rx.recv().await { /* SSE */ }
//! ```

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::{error, info, warn};
use uuid::Uuid;

use pekko_actor::{Actor, ActorContext};
use pekko_agent_core::{
    AgentInfo, AgentStatus, AgentTask, Message, MessageRole,
    ShortTermMemory, ToolContext, ToolDefinition,
};
use pekko_agent_llm::{
    ClaudeClient, GeminiClient, LlmConfig, LlmRequest,
    ClaudeMessage, ContentBlock, ClaudeTool, OpenAIClient,
};
use pekko_agent_memory::PgConversationStore;
use pekko_agent_tools::ToolRegistry;

use crate::workflow::Workflow;
use crate::saga::SagaManager;

// ─── Service dependencies ────────────────────────────────────────────────────

/// All external service dependencies the orchestrator needs to process queries.
pub struct OrchestratorDeps {
    pub conversation_store: Arc<PgConversationStore>,
    pub tool_registry:      Arc<RwLock<ToolRegistry>>,
    pub claude_client:      Arc<ClaudeClient>,
    pub gemini_client:      Option<Arc<GeminiClient>>,
    pub openai_client:      Option<Arc<OpenAIClient>>,
    pub llm_config:         LlmConfig,
    pub llm_provider:       String,
}

// ─── Result type ─────────────────────────────────────────────────────────────

/// The result returned by a `QueryAgent` message.
#[derive(Debug)]
pub struct QueryResult {
    pub session_id:    Uuid,
    pub agent_id:      String,
    pub response:      String,
    pub tools_used:    Vec<String>,
    pub input_tokens:  u32,
    pub output_tokens: u32,
}

// ─── Messages ────────────────────────────────────────────────────────────────

pub enum OrchestratorMessage {
    // ── Existing lifecycle messages ──
    RegisterAgent(AgentInfo),
    CreateWorkflow(Workflow),
    SubmitTask(AgentTask),
    AssignNextTask,
    CompleteTask { task_id: Uuid, result: serde_json::Value },
    FailTask      { task_id: Uuid, error: String },

    // ── Query messages (new) ──

    /// Blocking query: result returned through `reply_to` oneshot channel.
    QueryAgent {
        agent_id:   String,
        content:    String,
        session_id: Uuid,
        tenant_id:  String,
        user_id:    String,
        reply_to:   oneshot::Sender<Result<QueryResult, String>>,
    },

    /// Streaming query: SSE JSON strings are sent through `event_tx`.
    StreamAgent {
        agent_id:   String,
        content:    String,
        session_id: Uuid,
        tenant_id:  String,
        user_id:    String,
        event_tx:   mpsc::UnboundedSender<String>,
    },
}

impl fmt::Debug for OrchestratorMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegisterAgent(a)     => write!(f, "RegisterAgent({})", a.agent_id),
            Self::CreateWorkflow(w)    => write!(f, "CreateWorkflow({})", w.name),
            Self::SubmitTask(t)        => write!(f, "SubmitTask({})", t.task_id),
            Self::AssignNextTask       => write!(f, "AssignNextTask"),
            Self::CompleteTask { task_id, .. } => write!(f, "CompleteTask({task_id})"),
            Self::FailTask { task_id, .. }     => write!(f, "FailTask({task_id})"),
            Self::QueryAgent  { agent_id, session_id, .. } =>
                write!(f, "QueryAgent(agent={agent_id}, session={session_id})"),
            Self::StreamAgent { agent_id, session_id, .. } =>
                write!(f, "StreamAgent(agent={agent_id}, session={session_id})"),
        }
    }
}

// ─── Actor state ─────────────────────────────────────────────────────────────

pub struct OrchestratorActor {
    workflows:      HashMap<Uuid, Workflow>,
    agent_registry: HashMap<String, AgentInfo>,
    task_queue:     VecDeque<AgentTask>,
    active_tasks:   HashMap<Uuid, TaskExecution>,
    #[allow(dead_code)]
    saga_manager:   SagaManager,
    deps:           OrchestratorDeps,
}

#[derive(Debug, Clone)]
pub struct TaskExecution {
    pub task:           AgentTask,
    pub assigned_agent: String,
    pub status:         TaskExecutionStatus,
    pub started_at:     DateTime<Utc>,
    pub completed_at:   Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskExecutionStatus {
    Queued,
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

// ─── pekko_actor::Actor impl ──────────────────────────────────────────────────

impl Actor for OrchestratorActor {
    type Message = OrchestratorMessage;

    fn pre_start(&mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {
            info!("OrchestratorActor started in ActorSystem");
        })
    }

    fn receive(
        &mut self,
        msg: Self::Message,
        _ctx: &mut ActorContext<Self>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        // Handle synchronous messages immediately and return an empty future.
        // For async messages, clone the deps (Arc clones are cheap) so the
        // returned future is 'static and Send.
        match msg {
            OrchestratorMessage::RegisterAgent(agent) => {
                self.register_agent(agent);
                Box::pin(async {})
            }
            OrchestratorMessage::CreateWorkflow(wf) => {
                self.create_workflow(wf);
                Box::pin(async {})
            }
            OrchestratorMessage::SubmitTask(task) => {
                self.submit_task(task);
                Box::pin(async {})
            }
            OrchestratorMessage::AssignNextTask => {
                self.assign_next_task();
                Box::pin(async {})
            }
            OrchestratorMessage::CompleteTask { task_id, result } => {
                self.complete_task(&task_id, result);
                Box::pin(async {})
            }
            OrchestratorMessage::FailTask { task_id, error } => {
                self.fail_task(&task_id, error);
                Box::pin(async {})
            }

            // ── Async query messages ──
            OrchestratorMessage::QueryAgent {
                agent_id, content, session_id, tenant_id, user_id, reply_to,
            } => {
                // Clone Arc deps so the future owns them (no borrow of self)
                let conv   = self.deps.conversation_store.clone();
                let tools  = self.deps.tool_registry.clone();
                let claude = self.deps.claude_client.clone();
                let gemini = self.deps.gemini_client.clone();
                let openai = self.deps.openai_client.clone();
                let cfg    = self.deps.llm_config.clone();
                let prov   = self.deps.llm_provider.clone();

                Box::pin(async move {
                    let result = run_query_loop(
                        agent_id, content, session_id, tenant_id, user_id,
                        conv, tools, claude, gemini, openai, cfg, prov,
                    ).await;
                    let _ = reply_to.send(result);
                })
            }

            OrchestratorMessage::StreamAgent {
                agent_id, content, session_id, tenant_id, user_id, event_tx,
            } => {
                let conv   = self.deps.conversation_store.clone();
                let tools  = self.deps.tool_registry.clone();
                let claude = self.deps.claude_client.clone();
                let gemini = self.deps.gemini_client.clone();
                let openai = self.deps.openai_client.clone();
                let cfg    = self.deps.llm_config.clone();
                let prov   = self.deps.llm_provider.clone();

                Box::pin(async move {
                    run_stream_loop(
                        agent_id, content, session_id, tenant_id, user_id,
                        conv, tools, claude, gemini, openai, cfg, prov,
                        event_tx,
                    ).await;
                    // event_tx dropped here → SSE stream closes
                })
            }
        }
    }

    fn post_stop(&mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async { info!("OrchestratorActor stopped"); })
    }
}

// ─── Business logic ───────────────────────────────────────────────────────────

impl OrchestratorActor {
    pub fn new(deps: OrchestratorDeps) -> Self {
        Self {
            workflows:      HashMap::new(),
            agent_registry: HashMap::new(),
            task_queue:     VecDeque::new(),
            active_tasks:   HashMap::new(),
            saga_manager:   SagaManager::new(),
            deps,
        }
    }

    pub fn register_agent(&mut self, agent: AgentInfo) {
        info!(agent_id = %agent.agent_id, "Agent registered");
        self.agent_registry.insert(agent.agent_id.clone(), agent);
    }

    pub fn create_workflow(&mut self, workflow: Workflow) -> Uuid {
        let id = workflow.id;
        info!(workflow_id = %id, "Workflow created");
        self.workflows.insert(id, workflow);
        id
    }

    pub fn submit_task(&mut self, task: AgentTask) {
        info!(task_id = %task.task_id, "Task submitted");
        self.task_queue.push_back(task);
    }

    pub fn assign_next_task(&mut self) -> Option<(String, AgentTask)> {
        let task = self.task_queue.pop_front()?;
        let available = self.agent_registry.values()
            .find(|a| matches!(a.status, AgentStatus::Available))
            .map(|a| a.agent_id.clone());

        if let Some(agent_id) = available {
            let exec = TaskExecution {
                task: task.clone(),
                assigned_agent: agent_id.clone(),
                status: TaskExecutionStatus::Running,
                started_at: Utc::now(),
                completed_at: None,
            };
            self.active_tasks.insert(task.task_id, exec);
            if let Some(a) = self.agent_registry.get_mut(&agent_id) {
                a.status = AgentStatus::Busy;
            }
            info!(task_id = %task.task_id, agent = %agent_id, "Task assigned");
            Some((agent_id, task))
        } else {
            warn!(task_id = %task.task_id, "No available agent, re-queuing");
            self.task_queue.push_front(task);
            None
        }
    }

    pub fn complete_task(&mut self, task_id: &Uuid, _result: serde_json::Value) {
        if let Some(exec) = self.active_tasks.get_mut(task_id) {
            exec.status       = TaskExecutionStatus::Completed;
            exec.completed_at = Some(Utc::now());
            if let Some(a) = self.agent_registry.get_mut(&exec.assigned_agent) {
                a.status = AgentStatus::Available;
            }
            info!(task_id = %task_id, "Task completed");
        }
    }

    pub fn fail_task(&mut self, task_id: &Uuid, error: String) {
        if let Some(exec) = self.active_tasks.get_mut(task_id) {
            exec.status       = TaskExecutionStatus::Failed(error.clone());
            exec.completed_at = Some(Utc::now());
            if let Some(a) = self.agent_registry.get_mut(&exec.assigned_agent) {
                a.status = AgentStatus::Available;
            }
            warn!(task_id = %task_id, error = %error, "Task failed");
        }
    }

    pub fn list_agents(&self)       -> Vec<&AgentInfo> { self.agent_registry.values().collect() }
    pub fn get_workflow(&self, id: &Uuid) -> Option<&Workflow> { self.workflows.get(id) }
    pub fn pending_tasks(&self)     -> usize { self.task_queue.len() }
    pub fn active_task_count(&self) -> usize {
        self.active_tasks.values().filter(|t| t.status == TaskExecutionStatus::Running).count()
    }
}

// ─── Tool-use loop helpers ────────────────────────────────────────────────────

const MAX_TOOL_ROUNDS: usize = 10;
const MAX_TOOL_RESULT_CHARS: usize = 30_000;

fn truncate_tool_result(content: &str) -> String {
    if content.len() <= MAX_TOOL_RESULT_CHARS {
        content.to_string()
    } else {
        format!(
            "{}...\n\n[결과가 너무 큽니다. {}자 중 {}자만 표시됩니다.]",
            &content[..MAX_TOOL_RESULT_CHARS],
            content.len(),
            MAX_TOOL_RESULT_CHARS
        )
    }
}

fn tool_definitions_to_claude_tools(tools: &[ToolDefinition]) -> Vec<ClaudeTool> {
    tools.iter().map(|t| ClaudeTool {
        name:         t.name.clone(),
        description:  t.description.clone(),
        input_schema: t.input_schema.clone(),
    }).collect()
}

/// Build a system prompt tailored to the selected agent.
pub fn build_system_prompt(agent_id: &str) -> String {
    let agent_desc = match agent_id {
        "ehs-permit-agent" =>
            "당신은 EHS(환경안전보건) 허가 관리 전문 AI 에이전트입니다.\n\
             역할: 위험작업 허가(PTW) 현황 조회/분석, 작업 허가 프로세스 안내, 위험작업 유형별 통계, 허가 승인/반려 현황 추적\n\
             주요 테이블: dangerousworkmanagement(위험작업허가), tbmmanagement(TBM), accidentfreemanagement(무재해)",
        "ehs-inspection-agent" =>
            "당신은 EHS(환경안전보건) 점검 관리 전문 AI 에이전트입니다.\n\
             역할: 안전점검 일정/결과 조회, 점검 현황 분석/보고, 위험성 평가 관리, 아차사고 분석\n\
             주요 테이블: riskassessmentmanagement(위험성평가), nearmissmanagement(아차사고), safetyeducationmanagement(안전교육)",
        "ehs-compliance-agent" =>
            "당신은 EHS(환경안전보건) 규정 준수 전문 AI 에이전트입니다.\n\
             역할: 법규/규정 준수 확인, MSDS 관리, 안전보건위원회 운영 현황, 컴플라이언스 보고서 지원\n\
             주요 테이블: msdsmanagement(MSDS), safetyhealthcommittee(안전보건위원회), employeeinfo(직원정보)",
        _ =>
            "당신은 EHS(환경안전보건) 통합 AI 에이전트입니다.\n\
             안전, 보건, 환경 관련 질문에 답변할 수 있습니다.",
    };

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let year  = chrono::Local::now().format("%Y").to_string();

    format!(
        "{agent_desc}\n\n\
         [현재 날짜] 오늘은 {today} 입니다. 날짜 관련 쿼리 시 이 날짜를 기준으로 하세요.\n\
         \"이번 달\", \"올해\", \"최근\" 등의 표현은 이 날짜 기준으로 해석하세요.\n\
         \n\
         [절대 규칙] 도구 사용:\n\
         당신은 ehs_query 도구를 통해 PostgreSQL 데이터베이스에 직접 접근할 수 있습니다.\n\
         - 데이터가 필요한 질문을 받으면 반드시 ehs_query 도구를 호출하여 실제 데이터를 조회하세요.\n\
         - 절대로 SQL 쿼리 예시를 사용자에게 보여주지 마세요. 직접 도구를 호출하세요.\n\
         - 도구 호출이 실패하면, 오류를 분석하고 수정된 쿼리로 즉시 재시도하세요.\n\
         \n\
         [필수] PostgreSQL 문법 (MySQL 문법 사용 금지):\n\
         - 날짜 추출: EXTRACT(YEAR FROM col), EXTRACT(MONTH FROM col)\n\
         - 문자열 비교: col ILIKE '%검색어%'\n\
         - 조건부 집계: COUNT(*) FILTER (WHERE 조건)\n\
         - 날짜 캐스팅: col::date, CURRENT_DATE\n\
         - 현재 날짜: CURRENT_DATE, CURRENT_DATE - INTERVAL '30 days'\n\
         - 올해 시작: '{year}-01-01'::date\n\
         - YEAR(), MONTH(), IFNULL(), NOW() 등 MySQL 전용 함수는 사용 금지\n\
         \n\
         [필수] 테이블 스키마 탐색 절차:\n\
         처음 접하는 테이블은 반드시 먼저 컬럼명을 확인하세요:\n\
         SELECT column_name, data_type FROM information_schema.columns WHERE table_name = '테이블명' ORDER BY ordinal_position\n\
         \n\
         [필수] 쿼리 작성 규칙:\n\
         - SELECT *는 피하고, 필요한 컬럼만 선택하세요.\n\
         - 큰 테이블은 반드시 LIMIT을 사용하세요.\n\
         - 쿼리 오류 시 오류 메시지를 분석하여 컬럼명/타입을 수정하고 재시도하세요.\n\
         \n\
         응답 규칙:\n\
         - 한국어로 답변하세요.\n\
         - 반드시 실제 데이터 기반으로 정확한 수치를 제공하세요.\n\
         \n\
         [필수] 응답 형식:\n\
         - 조회 결과가 여러 행이면 반드시 마크다운 테이블로 정리하세요.\n\
         - 숫자는 천 단위 쉼표를 사용하세요 (예: 1,234).\n\
         - 날짜는 YYYY-MM-DD 형식으로 표시하세요."
    )
}

// ─── Shared LLM call helper ───────────────────────────────────────────────────

async fn call_llm(
    provider:     &str,
    request:      &LlmRequest,
    claude:       &ClaudeClient,
    gemini:       Option<&GeminiClient>,
    openai:       Option<&OpenAIClient>,
) -> Result<pekko_agent_llm::LlmResponse, String> {
    match provider {
        "openai" => match openai {
            Some(c) => c.send_message(request).await.map_err(|e| format!("OpenAI 오류: {e}")),
            None    => Err("OpenAI API 키가 없습니다".into()),
        },
        "gemini" => match gemini {
            Some(c) => c.send_message(request).await.map_err(|e| format!("Gemini 오류: {e}")),
            None    => Err("Gemini API 키가 없습니다".into()),
        },
        _ => claude.send_message(request).await.map_err(|e| format!("Claude 오류: {e}")),
    }
}

// ─── Blocking query loop ──────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_query_loop(
    agent_id:   String,
    content:    String,
    session_id: Uuid,
    tenant_id:  String,
    user_id:    String,
    conv:       Arc<PgConversationStore>,
    tools:      Arc<RwLock<ToolRegistry>>,
    claude:     Arc<ClaudeClient>,
    gemini:     Option<Arc<GeminiClient>>,
    openai:     Option<Arc<OpenAIClient>>,
    cfg:        LlmConfig,
    provider:   String,
) -> Result<QueryResult, String> {
    // Persist user message
    let _ = conv.append_message(&session_id, Message::user(&content)).await;

    // Build history
    let history: Vec<Message> = conv.get_conversation(&session_id).await.unwrap_or_default();
    let mut messages: Vec<ClaudeMessage> = history.iter().map(|m| ClaudeMessage {
        role: match m.role {
            MessageRole::Assistant => "assistant".to_string(),
            _ => "user".to_string(),
        },
        content: vec![ContentBlock::Text { text: m.content.clone() }],
    }).collect();

    let tool_defs = { tools.read().await.list_tools() };
    let claude_tools = tool_definitions_to_claude_tools(&tool_defs);
    let system_prompt = build_system_prompt(&agent_id);

    let mut all_tools_used: Vec<String> = Vec::new();
    let mut total_input:  u32 = 0;
    let mut total_output: u32 = 0;
    let mut final_text = String::new();

    for _round in 0..MAX_TOOL_ROUNDS {
        let req = LlmRequest {
            system_prompt: system_prompt.clone(),
            messages: messages.clone(),
            tools: claude_tools.clone(),
            max_tokens: cfg.max_tokens,
            temperature: Some(cfg.temperature),
            cacheable: false,
        };

        let resp = call_llm(
            &provider, &req,
            &claude,
            gemini.as_deref(),
            openai.as_deref(),
        ).await?;

        total_input  += resp.usage.input_tokens;
        total_output += resp.usage.output_tokens;

        if resp.stop_reason == "tool_use" {
            let mut assistant_content = Vec::new();
            let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new();

            for block in &resp.content {
                match block {
                    ContentBlock::Text { text } => {
                        assistant_content.push(ContentBlock::Text { text: text.clone() });
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        assistant_content.push(ContentBlock::ToolUse {
                            id: id.clone(), name: name.clone(), input: input.clone(),
                        });
                        tool_uses.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }
            messages.push(ClaudeMessage { role: "assistant".to_string(), content: assistant_content });

            let ctx = ToolContext {
                tenant_id: tenant_id.clone(),
                user_id:   user_id.clone(),
                session_id,
                credentials: std::collections::HashMap::new(),
                timeout: Duration::from_secs(10),
            };
            let mut tool_results = Vec::new();

            for (tool_use_id, tool_name, tool_input) in &tool_uses {
                info!(tool_name = %tool_name, "Executing tool");
                all_tools_used.push(tool_name.clone());

                let result = {
                    let mut reg = tools.write().await;
                    reg.execute(tool_name, tool_input.clone(), &ctx).await
                };

                let (content_str, is_error) = match result {
                    Ok(out) => {
                        let s = truncate_tool_result(
                            &serde_json::to_string(&out.content).unwrap_or_default()
                        );
                        (s, if out.is_error { Some(true) } else { None })
                    }
                    Err(e) => {
                        warn!(tool = %tool_name, error = %e, "Tool failed");
                        (format!("{{\"error\":\"{e}\"}}"), Some(true))
                    }
                };

                tool_results.push(ContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: content_str,
                    is_error,
                });
            }
            messages.push(ClaudeMessage { role: "user".to_string(), content: tool_results });
            continue;
        }

        // end_turn
        for block in &resp.content {
            if let ContentBlock::Text { text } = block {
                if !final_text.is_empty() { final_text.push('\n'); }
                final_text.push_str(text);
            }
        }
        break;
    }

    let _ = conv.append_message(&session_id, Message::assistant(&final_text)).await;
    all_tools_used.sort(); all_tools_used.dedup();

    Ok(QueryResult {
        session_id,
        agent_id,
        response: final_text,
        tools_used: all_tools_used,
        input_tokens: total_input,
        output_tokens: total_output,
    })
}

// ─── Streaming query loop ─────────────────────────────────────────────────────

macro_rules! emit {
    ($tx:expr, $($tt:tt)*) => {
        let _ = $tx.send(serde_json::json!($($tt)*).to_string());
    };
}

#[allow(clippy::too_many_arguments)]
async fn run_stream_loop(
    agent_id:   String,
    content:    String,
    session_id: Uuid,
    tenant_id:  String,
    user_id:    String,
    conv:       Arc<PgConversationStore>,
    tools:      Arc<RwLock<ToolRegistry>>,
    claude:     Arc<ClaudeClient>,
    gemini:     Option<Arc<GeminiClient>>,
    openai:     Option<Arc<OpenAIClient>>,
    cfg:        LlmConfig,
    provider:   String,
    tx:         mpsc::UnboundedSender<String>,
) {
    let _ = conv.append_message(&session_id, Message::user(&content)).await;

    let history: Vec<Message> = conv.get_conversation(&session_id).await.unwrap_or_default();
    let mut messages: Vec<ClaudeMessage> = history.iter().map(|m| ClaudeMessage {
        role: match m.role {
            MessageRole::Assistant => "assistant".to_string(),
            _ => "user".to_string(),
        },
        content: vec![ContentBlock::Text { text: m.content.clone() }],
    }).collect();

    let tool_defs = { tools.read().await.list_tools() };
    let claude_tools = tool_definitions_to_claude_tools(&tool_defs);
    let system_prompt = build_system_prompt(&agent_id);

    let mut all_tools_used: Vec<String> = Vec::new();
    let mut total_input:  u32 = 0;
    let mut total_output: u32 = 0;
    let mut final_text = String::new();

    'outer: for round in 0..MAX_TOOL_ROUNDS {
        emit!(tx, {"type": "thinking", "round": round});

        let req = LlmRequest {
            system_prompt: system_prompt.clone(),
            messages: messages.clone(),
            tools: claude_tools.clone(),
            max_tokens: cfg.max_tokens,
            temperature: Some(cfg.temperature),
            cacheable: false,
        };

        let resp = match call_llm(
            &provider, &req,
            &claude,
            gemini.as_deref(),
            openai.as_deref(),
        ).await {
            Ok(r)  => r,
            Err(e) => { emit!(tx, {"type":"error","message": e}); break 'outer; }
        };

        total_input  += resp.usage.input_tokens;
        total_output += resp.usage.output_tokens;

        if resp.stop_reason == "tool_use" {
            let mut assistant_content = Vec::new();
            let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new();

            for block in &resp.content {
                match block {
                    ContentBlock::Text { text } => {
                        assistant_content.push(ContentBlock::Text { text: text.clone() });
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        assistant_content.push(ContentBlock::ToolUse {
                            id: id.clone(), name: name.clone(), input: input.clone(),
                        });
                        tool_uses.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }
            messages.push(ClaudeMessage { role: "assistant".to_string(), content: assistant_content });

            let ctx = ToolContext {
                tenant_id: tenant_id.clone(),
                user_id:   user_id.clone(),
                session_id,
                credentials: std::collections::HashMap::new(),
                timeout: Duration::from_secs(10),
            };
            let mut tool_results = Vec::new();

            for (tool_use_id, tool_name, tool_input) in &tool_uses {
                emit!(tx, {"type": "tool_use", "tool": tool_name, "round": round});
                all_tools_used.push(tool_name.clone());

                let result = {
                    let mut reg = tools.write().await;
                    reg.execute(tool_name, tool_input.clone(), &ctx).await
                };

                let (content_str, is_error, ok) = match result {
                    Ok(out) => {
                        let s = truncate_tool_result(
                            &serde_json::to_string(&out.content).unwrap_or_default()
                        );
                        let ok = !out.is_error;
                        (s, if out.is_error { Some(true) } else { None }, ok)
                    }
                    Err(e) => {
                        warn!(tool = %tool_name, error = %e, "Tool failed in stream");
                        (format!("{{\"error\":\"{e}\"}}"), Some(true), false)
                    }
                };

                emit!(tx, {"type": "tool_result", "tool": tool_name, "ok": ok});
                tool_results.push(ContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: content_str,
                    is_error,
                });
            }
            messages.push(ClaudeMessage { role: "user".to_string(), content: tool_results });
            continue;
        }

        for block in &resp.content {
            if let ContentBlock::Text { text } = block {
                if !final_text.is_empty() { final_text.push('\n'); }
                final_text.push_str(text);
            }
        }
        break;
    }

    let _ = conv.append_message(&session_id, Message::assistant(&final_text)).await;
    all_tools_used.sort(); all_tools_used.dedup();

    emit!(tx, {"type": "text_chunk", "text": final_text});
    emit!(tx, {
        "type": "done",
        "session_id": session_id,
        "tools_used": all_tools_used,
        "input_tokens": total_input,
        "output_tokens": total_output
    });
}
