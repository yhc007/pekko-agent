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
//! // Stream (events via bounded mpsc — backpressure built-in)
//! let (tx, rx) = tokio::sync::mpsc::channel(256);
//! orch_ref.tell(OrchestratorMessage::StreamAgent { ..., event_tx: tx })?;
//! while let Some(json) = rx.recv().await { /* SSE / WebSocket */ }
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
    AgentInfo, AgentProfile, AgentStatus, AgentTask, LongTermMemory, Message, MessageRole,
    ShortTermMemory, ToolContext, ToolDefinition,
};
use pekko_agent_llm::{
    ClaudeClient, GeminiClient, LlmConfig, LlmRequest,
    ClaudeMessage, ContentBlock, ClaudeTool, OpenAIClient,
};
use pekko_actor::{CircuitBreaker, CircuitBreakerBuilder, CircuitBreakerError};
use pekko_agent_memory::{PgConversationStore, PgVectorStore};
use pekko_agent_observability::MetricsRegistry;
use pekko_agent_tools::ToolRegistry;

use crate::workflow::{build_step_content, topological_sort, Workflow, WorkflowResult, WorkflowStatus};
use crate::saga::{run_saga_engine, SagaDefinition, SagaExecution, SagaManager, SagaResult, SagaStatus};
use crate::persistence::{OrchestratorPersistence, spawn_save_agent, spawn_save_workflow};

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
    /// Optional long-term memory for RAG context injection.
    pub vector_store:       Option<Arc<PgVectorStore>>,
    /// Optional Prometheus metrics registry.
    pub metrics:            Option<Arc<MetricsRegistry>>,
    /// Per-provider circuit breakers (key = provider name: "claude", "gemini", "openai").
    pub circuit_breakers:   HashMap<String, CircuitBreaker>,
    /// Optional PostgreSQL-backed state persistence.
    pub persistence:        Option<Arc<OrchestratorPersistence>>,
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
    RegisterAgent {
        info:    AgentInfo,
        /// Declares which tools this agent may use and optional token limit.
        profile: AgentProfile,
    },
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
        /// Bounded — provides backpressure so slow consumers don't inflate memory.
        event_tx:   mpsc::Sender<String>,
    },

    /// Read-only query: returns a snapshot of the registered agent list.
    GetAgents {
        reply_to: oneshot::Sender<Vec<AgentInfo>>,
    },

    // ── Workflow execution ──

    /// Create + execute a workflow; result returned through oneshot when done.
    ExecuteWorkflow {
        workflow: Workflow,
        reply_to: oneshot::Sender<Result<WorkflowResult, String>>,
    },

    /// Create + execute a workflow with SSE-style event streaming.
    StreamWorkflow {
        workflow: Workflow,
        event_tx: mpsc::Sender<String>,
    },

    /// Return the current `WorkflowStatus` for a previously-started workflow.
    GetWorkflowStatus {
        workflow_id: Uuid,
        reply_to:    oneshot::Sender<Option<WorkflowStatus>>,
    },

    // ── Saga pattern ──

    /// Register a `SagaDefinition` so it can be executed by ID later.
    RegisterSaga(SagaDefinition),

    /// Execute a saga by definition ID; result returned via oneshot.
    ExecuteSaga {
        saga_id:  Uuid,
        reply_to: oneshot::Sender<Result<SagaResult, String>>,
    },

    /// Stream a saga execution; SSE events go through `event_tx`.
    StreamSaga {
        saga_id:  Uuid,
        event_tx: mpsc::Sender<String>,
    },

    /// Return a snapshot of a running/completed saga execution.
    GetSagaExecution {
        execution_id: Uuid,
        reply_to:     oneshot::Sender<Option<SagaExecution>>,
    },

    /// List all registered saga definitions.
    ListSagas {
        reply_to: oneshot::Sender<Vec<SagaDefinition>>,
    },
}

impl fmt::Debug for OrchestratorMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegisterAgent { info, .. } => write!(f, "RegisterAgent({})", info.agent_id),
            Self::CreateWorkflow(w)    => write!(f, "CreateWorkflow({})", w.name),
            Self::SubmitTask(t)        => write!(f, "SubmitTask({})", t.task_id),
            Self::AssignNextTask       => write!(f, "AssignNextTask"),
            Self::CompleteTask { task_id, .. } => write!(f, "CompleteTask({task_id})"),
            Self::FailTask { task_id, .. }     => write!(f, "FailTask({task_id})"),
            Self::QueryAgent  { agent_id, session_id, .. } =>
                write!(f, "QueryAgent(agent={agent_id}, session={session_id})"),
            Self::StreamAgent { agent_id, session_id, .. } =>
                write!(f, "StreamAgent(agent={agent_id}, session={session_id})"),
            Self::GetAgents { .. } =>
                write!(f, "GetAgents"),
            Self::ExecuteWorkflow { workflow, .. } =>
                write!(f, "ExecuteWorkflow({})", workflow.name),
            Self::StreamWorkflow { workflow, .. } =>
                write!(f, "StreamWorkflow({})", workflow.name),
            Self::GetWorkflowStatus { workflow_id, .. } =>
                write!(f, "GetWorkflowStatus({workflow_id})"),
            Self::RegisterSaga(s) =>
                write!(f, "RegisterSaga({})", s.saga_id),
            Self::ExecuteSaga { saga_id, .. } =>
                write!(f, "ExecuteSaga({saga_id})"),
            Self::StreamSaga { saga_id, .. } =>
                write!(f, "StreamSaga({saga_id})"),
            Self::GetSagaExecution { execution_id, .. } =>
                write!(f, "GetSagaExecution({execution_id})"),
            Self::ListSagas { .. } =>
                write!(f, "ListSagas"),
        }
    }
}

// ─── Actor state ─────────────────────────────────────────────────────────────

pub struct OrchestratorActor {
    workflows:      HashMap<Uuid, Workflow>,
    agent_registry: HashMap<String, AgentInfo>,
    agent_profiles: HashMap<String, AgentProfile>,
    task_queue:     VecDeque<AgentTask>,
    active_tasks:   HashMap<Uuid, TaskExecution>,
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

            let Some(store) = self.deps.persistence.clone() else { return; };

            // Restore agent registry
            match store.load_agents().await {
                Ok(agents) => {
                    let count = agents.len();
                    for (info, profile) in agents {
                        self.agent_profiles.insert(info.agent_id.clone(), profile);
                        self.agent_registry.insert(info.agent_id.clone(), info);
                    }
                    info!(count, "Restored agents from persistence");
                }
                Err(e) => warn!(error = %e, "Failed to load agents from persistence"),
            }

            // Restore workflow history; interrupted Running → Failed
            match store.load_workflows().await {
                Ok(mut workflows) => {
                    let count = workflows.len();
                    for wf in &mut workflows {
                        if let WorkflowStatus::Running { current_step } = &wf.status {
                            let step = *current_step;
                            wf.status = WorkflowStatus::Failed {
                                at_step: step,
                                error: "서버 재시작으로 인해 워크플로우가 중단되었습니다.".to_string(),
                            };
                            // Persist the updated (Failed) status back to DB
                            let wf_clone = wf.clone();
                            let s = store.clone();
                            tokio::spawn(async move {
                                if let Err(e) = s.save_workflow(&wf_clone).await {
                                    warn!(error = %e, "Failed to update interrupted workflow status");
                                }
                            });
                        }
                        self.workflows.insert(wf.id, wf.clone());
                    }
                    info!(count, "Restored workflows from persistence");
                }
                Err(e) => warn!(error = %e, "Failed to load workflows from persistence"),
            }
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
            OrchestratorMessage::RegisterAgent { info, profile } => {
                self.agent_profiles.insert(info.agent_id.clone(), profile.clone());
                self.register_agent(info.clone());
                if let Some(store) = self.deps.persistence.clone() {
                    spawn_save_agent(store, info, profile);
                }
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

            // ── Query messages: spawned in separate tokio tasks so the actor
            //    mailbox is never blocked during long LLM calls ──

            OrchestratorMessage::QueryAgent {
                agent_id, content, session_id, tenant_id, user_id, reply_to,
            } => {
                let profile  = self.agent_profiles.get(&agent_id).cloned().unwrap_or_default();
                let conv     = self.deps.conversation_store.clone();
                let tools    = self.deps.tool_registry.clone();
                let claude   = self.deps.claude_client.clone();
                let gemini   = self.deps.gemini_client.clone();
                let openai   = self.deps.openai_client.clone();
                let cfg      = self.deps.llm_config.clone();
                let prov     = self.deps.llm_provider.clone();
                let vs       = self.deps.vector_store.clone();
                let metrics  = self.deps.metrics.clone();
                let cb       = self.deps.circuit_breakers.get(&self.deps.llm_provider).cloned();

                tokio::spawn(async move {
                    let result = run_query_loop(
                        agent_id, content, session_id, tenant_id, user_id,
                        conv, tools, claude, gemini, openai, cfg, prov,
                        profile, vs, metrics, cb,
                    ).await;
                    let _ = reply_to.send(result);
                });
                Box::pin(async {})
            }

            OrchestratorMessage::StreamAgent {
                agent_id, content, session_id, tenant_id, user_id, event_tx,
            } => {
                let profile  = self.agent_profiles.get(&agent_id).cloned().unwrap_or_default();
                let conv     = self.deps.conversation_store.clone();
                let tools    = self.deps.tool_registry.clone();
                let claude   = self.deps.claude_client.clone();
                let gemini   = self.deps.gemini_client.clone();
                let openai   = self.deps.openai_client.clone();
                let cfg      = self.deps.llm_config.clone();
                let prov     = self.deps.llm_provider.clone();
                let vs       = self.deps.vector_store.clone();
                let metrics  = self.deps.metrics.clone();
                let cb       = self.deps.circuit_breakers.get(&self.deps.llm_provider).cloned();

                tokio::spawn(async move {
                    run_stream_loop(
                        agent_id, content, session_id, tenant_id, user_id,
                        conv, tools, claude, gemini, openai, cfg, prov,
                        event_tx, profile, vs, metrics, cb,
                    ).await;
                });
                Box::pin(async {})
            }

            // ── Read-only: snapshot of agent registry ──
            OrchestratorMessage::GetAgents { reply_to } => {
                let agents: Vec<AgentInfo> = self.agent_registry.values().cloned().collect();
                let _ = reply_to.send(agents);
                Box::pin(async {})
            }

            // ── Workflow execution ──
            OrchestratorMessage::ExecuteWorkflow { workflow, reply_to } => {
                let wf_id = workflow.id;
                self.workflows.insert(wf_id, workflow.clone());
                if let Some(store) = self.deps.persistence.clone() {
                    spawn_save_workflow(store, workflow.clone());
                }
                let store   = self.deps.persistence.clone();
                let profiles = self.agent_profiles.clone();
                let conv   = self.deps.conversation_store.clone();
                let tools  = self.deps.tool_registry.clone();
                let claude = self.deps.claude_client.clone();
                let gemini = self.deps.gemini_client.clone();
                let openai = self.deps.openai_client.clone();
                let cfg    = self.deps.llm_config.clone();
                let prov   = self.deps.llm_provider.clone();

                tokio::spawn(async move {
                    let result = run_workflow_engine(
                        workflow, conv, tools, claude, gemini, openai, cfg, prov,
                        profiles, None,
                    ).await;
                    // Persist final status
                    if let (Some(s), Ok(ref wf_result)) = (store, &result) {
                        if let Err(e) = s.update_workflow_status(
                            wf_id, &wf_result.status, None
                        ).await {
                            warn!(workflow_id = %wf_id, error = %e, "Failed to update workflow status");
                        }
                    }
                    let _ = reply_to.send(result);
                });
                Box::pin(async {})
            }

            OrchestratorMessage::StreamWorkflow { workflow, event_tx } => {
                let wf_id = workflow.id;
                self.workflows.insert(wf_id, workflow.clone());
                if let Some(store) = self.deps.persistence.clone() {
                    spawn_save_workflow(store, workflow.clone());
                }
                let store   = self.deps.persistence.clone();
                let profiles = self.agent_profiles.clone();
                let conv   = self.deps.conversation_store.clone();
                let tools  = self.deps.tool_registry.clone();
                let claude = self.deps.claude_client.clone();
                let gemini = self.deps.gemini_client.clone();
                let openai = self.deps.openai_client.clone();
                let cfg    = self.deps.llm_config.clone();
                let prov   = self.deps.llm_provider.clone();

                tokio::spawn(async move {
                    let result = run_workflow_engine(
                        workflow, conv, tools, claude, gemini, openai, cfg, prov,
                        profiles, Some(event_tx),
                    ).await;
                    if let (Some(s), Ok(ref wf_result)) = (store, &result) {
                        if let Err(e) = s.update_workflow_status(
                            wf_id, &wf_result.status, None
                        ).await {
                            warn!(workflow_id = %wf_id, error = %e, "Failed to update workflow status");
                        }
                    }
                });
                Box::pin(async {})
            }

            OrchestratorMessage::GetWorkflowStatus { workflow_id, reply_to } => {
                let status = self.workflows.get(&workflow_id).map(|w| w.status.clone());
                let _ = reply_to.send(status);
                Box::pin(async {})
            }

            // ── Saga ──
            OrchestratorMessage::RegisterSaga(saga) => {
                self.saga_manager.register(saga);
                Box::pin(async {})
            }

            OrchestratorMessage::ExecuteSaga { saga_id, reply_to } => {
                let Some(saga) = self.saga_manager.get_definition(&saga_id).cloned() else {
                    let _ = reply_to.send(Err(format!("Saga '{saga_id}' not registered")));
                    return Box::pin(async {});
                };
                let conv   = self.deps.conversation_store.clone();
                let tools  = self.deps.tool_registry.clone();
                let claude = self.deps.claude_client.clone();
                let gemini = self.deps.gemini_client.clone();
                let openai = self.deps.openai_client.clone();
                let cfg    = self.deps.llm_config.clone();
                let prov   = self.deps.llm_provider.clone();
                let prof   = self.agent_profiles.clone();

                tokio::spawn(async move {
                    let result = run_saga_engine(
                        saga, conv, tools, claude, gemini, openai, cfg, prov, prof, None,
                    ).await;
                    let _ = reply_to.send(Ok(result));
                });
                Box::pin(async {})
            }

            OrchestratorMessage::StreamSaga { saga_id, event_tx } => {
                let Some(saga) = self.saga_manager.get_definition(&saga_id).cloned() else {
                    let msg = format!("Saga '{saga_id}' not registered");
                    let _ = event_tx.try_send(
                        serde_json::json!({"type":"error","message": msg}).to_string()
                    );
                    return Box::pin(async {});
                };
                let conv   = self.deps.conversation_store.clone();
                let tools  = self.deps.tool_registry.clone();
                let claude = self.deps.claude_client.clone();
                let gemini = self.deps.gemini_client.clone();
                let openai = self.deps.openai_client.clone();
                let cfg    = self.deps.llm_config.clone();
                let prov   = self.deps.llm_provider.clone();
                let prof   = self.agent_profiles.clone();

                tokio::spawn(async move {
                    run_saga_engine(
                        saga, conv, tools, claude, gemini, openai, cfg, prov, prof, Some(event_tx),
                    ).await;
                });
                Box::pin(async {})
            }

            OrchestratorMessage::GetSagaExecution { execution_id, reply_to } => {
                let exec = self.saga_manager.get_execution(&execution_id).cloned();
                let _ = reply_to.send(exec);
                Box::pin(async {})
            }

            OrchestratorMessage::ListSagas { reply_to } => {
                let defs: Vec<SagaDefinition> = self.saga_manager
                    .all_definitions().into_iter().cloned().collect();
                let _ = reply_to.send(defs);
                Box::pin(async {})
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
            agent_profiles: HashMap::new(),
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

/// Wrapper around `call_llm` that applies a circuit breaker when available.
/// Also updates CB state metrics.
async fn call_llm_protected(
    provider:  &str,
    request:   &LlmRequest,
    claude:    &ClaudeClient,
    gemini:    Option<&GeminiClient>,
    openai:    Option<&OpenAIClient>,
    cb:        Option<&CircuitBreaker>,
    metrics:   Option<&MetricsRegistry>,
) -> Result<pekko_agent_llm::LlmResponse, String> {
    if let Some(cb) = cb {
        let result = cb.call(|| call_llm(provider, request, claude, gemini, openai)).await;

        // Update CB state metric
        if let Some(m) = metrics {
            let state_val = match cb.state() {
                pekko_actor::CircuitBreakerState::Closed   => 0.0,
                pekko_actor::CircuitBreakerState::Open     => 1.0,
                pekko_actor::CircuitBreakerState::HalfOpen => 2.0,
            };
            m.circuit_breaker_state
                .with_label_values(&[provider])
                .set(state_val);
        }

        result.map_err(|e| match e {
            CircuitBreakerError::Open => {
                if let Some(m) = metrics {
                    m.circuit_breaker_rejections
                        .with_label_values(&[provider])
                        .inc();
                }
                format!("LLM 회로 차단기 열림 — '{provider}' 공급자가 일시적으로 차단되었습니다")
            }
            CircuitBreakerError::Timeout =>
                format!("LLM 호출 타임아웃 (provider: {provider})"),
            CircuitBreakerError::CallFailed(e) => e,
        })
    } else {
        call_llm(provider, request, claude, gemini, openai).await
    }
}

// ─── Blocking query loop ──────────────────────────────────────────────────────

/// Public re-export so `saga.rs` can call into the query loop.
#[allow(clippy::too_many_arguments)]
pub async fn run_query_loop_pub(
    agent_id: String, content: String, session_id: Uuid,
    tenant_id: String, user_id: String,
    conv: Arc<PgConversationStore>, tools: Arc<RwLock<ToolRegistry>>,
    claude: Arc<ClaudeClient>, gemini: Option<Arc<GeminiClient>>,
    openai: Option<Arc<OpenAIClient>>, cfg: LlmConfig, provider: String,
    profile: AgentProfile,
    vs: Option<Arc<PgVectorStore>>,
    metrics: Option<Arc<MetricsRegistry>>,
    cb: Option<CircuitBreaker>,
) -> Result<QueryResult, String> {
    run_query_loop(
        agent_id, content, session_id, tenant_id, user_id,
        conv, tools, claude, gemini, openai, cfg, provider,
        profile, vs, metrics, cb,
    ).await
}

#[allow(clippy::too_many_arguments)]
async fn run_query_loop(
    agent_id:     String,
    content:      String,
    session_id:   Uuid,
    tenant_id:    String,
    user_id:      String,
    conv:         Arc<PgConversationStore>,
    tools:        Arc<RwLock<ToolRegistry>>,
    claude:       Arc<ClaudeClient>,
    gemini:       Option<Arc<GeminiClient>>,
    openai:       Option<Arc<OpenAIClient>>,
    cfg:          LlmConfig,
    provider:     String,
    profile:      AgentProfile,
    vector_store: Option<Arc<PgVectorStore>>,
    metrics:      Option<Arc<MetricsRegistry>>,
    cb:           Option<CircuitBreaker>,
) -> Result<QueryResult, String> {
    let _query_start = std::time::Instant::now();
    // Persist user message
    let _ = conv.append_message(&session_id, Message::user(&content)).await;

    // RAG: inject relevant long-term memory as context prefix
    let rag_prefix = build_rag_prefix(&content, vector_store.as_deref()).await;

    // Build history
    let history: Vec<Message> = conv.get_conversation(&session_id).await.unwrap_or_default();
    let mut messages: Vec<ClaudeMessage> = history.iter().map(|m| ClaudeMessage {
        role: match m.role {
            MessageRole::Assistant => "assistant".to_string(),
            _ => "user".to_string(),
        },
        content: vec![ContentBlock::Text { text: m.content.clone() }],
    }).collect();

    // Prepend RAG context as the first user message when history is empty,
    // or inject it as a standalone context block before the latest query.
    if let Some(prefix) = rag_prefix {
        if messages.is_empty() {
            // history is just the current user message we just appended — replace it
            // with [context, query] pair so the LLM sees the context first.
            messages.push(ClaudeMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::Text { text: prefix }],
            });
            messages.push(ClaudeMessage {
                role: "assistant".to_string(),
                content: vec![ContentBlock::Text { text: "관련 컨텍스트를 확인했습니다. 질문에 답변하겠습니다.".to_string() }],
            });
        } else if let Some(last) = messages.last_mut() {
            // Append RAG context to the final user message
            if last.role == "user" {
                if let Some(ContentBlock::Text { text }) = last.content.last_mut() {
                    text.push_str("\n\n");
                    text.push_str(&prefix);
                }
            }
        }
    }

    let all_tool_defs = tools.read().await.list_tools();
    let tool_defs: Vec<ToolDefinition> = match &profile.tool_whitelist {
        Some(wl) => all_tool_defs.into_iter().filter(|t| wl.contains(&t.name)).collect(),
        None     => all_tool_defs,
    };
    let claude_tools  = tool_definitions_to_claude_tools(&tool_defs);
    let system_prompt = build_system_prompt(&agent_id);
    let max_tokens    = profile.max_tokens_override.unwrap_or(cfg.max_tokens);

    let mut all_tools_used: Vec<String> = Vec::new();
    let mut total_input:  u32 = 0;
    let mut total_output: u32 = 0;
    let mut final_text = String::new();

    // Record agent query metric
    if let Some(m) = &metrics {
        m.agent_queries_total
            .with_label_values(&[&agent_id, &tenant_id])
            .inc();
    }

    for _round in 0..MAX_TOOL_ROUNDS {
        let req = LlmRequest {
            system_prompt: system_prompt.clone(),
            messages: messages.clone(),
            tools: claude_tools.clone(),
            max_tokens,
            temperature: Some(cfg.temperature),
            cacheable: false,
        };

        let llm_start = std::time::Instant::now();
        let resp = call_llm_protected(
            &provider, &req,
            &claude,
            gemini.as_deref(),
            openai.as_deref(),
            cb.as_ref(),
            metrics.as_deref(),
        ).await;
        let llm_elapsed = llm_start.elapsed().as_secs_f64();

        let resp = match resp {
            Ok(r) => {
                if let Some(m) = &metrics {
                    m.llm_requests_total
                        .with_label_values(&[&provider, &agent_id, "ok"])
                        .inc();
                    m.llm_request_duration_secs
                        .with_label_values(&[&provider, &agent_id])
                        .observe(llm_elapsed);
                    m.llm_input_tokens_total
                        .with_label_values(&[&provider])
                        .inc_by(r.usage.input_tokens as f64);
                    m.llm_output_tokens_total
                        .with_label_values(&[&provider])
                        .inc_by(r.usage.output_tokens as f64);
                }
                r
            }
            Err(e) => {
                if let Some(m) = &metrics {
                    m.llm_requests_total
                        .with_label_values(&[&provider, &agent_id, "error"])
                        .inc();
                }
                return Err(e);
            }
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
                info!(tool_name = %tool_name, "Executing tool");
                all_tools_used.push(tool_name.clone());

                let tool_start = std::time::Instant::now();
                let result = {
                    let mut reg = tools.write().await;
                    reg.execute(tool_name, tool_input.clone(), &ctx).await
                };
                let tool_elapsed = tool_start.elapsed().as_secs_f64();

                let (content_str, is_error) = match result {
                    Ok(out) => {
                        let ok = !out.is_error;
                        if let Some(m) = &metrics {
                            m.tool_executions_total
                                .with_label_values(&[tool_name, if ok { "ok" } else { "error" }])
                                .inc();
                            m.tool_duration_secs
                                .with_label_values(&[tool_name])
                                .observe(tool_elapsed);
                        }
                        let s = truncate_tool_result(
                            &serde_json::to_string(&out.content).unwrap_or_default()
                        );
                        (s, if out.is_error { Some(true) } else { None })
                    }
                    Err(e) => {
                        warn!(tool = %tool_name, error = %e, "Tool failed");
                        if let Some(m) = &metrics {
                            m.tool_executions_total
                                .with_label_values(&[tool_name, "error"])
                                .inc();
                        }
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

// ─── RAG helper ───────────────────────────────────────────────────────────────

/// Search the vector store for relevant documents and format them as a context
/// prefix that can be prepended to the user query.
/// Returns `None` when the store is absent or returns no results.
async fn build_rag_prefix(query: &str, vs: Option<&PgVectorStore>) -> Option<String> {
    let store = vs?;
    let results = store.search(query, 3).await.ok()?;
    if results.is_empty() {
        return None;
    }

    let mut prefix = String::from("[장기 기억 — 관련 참고 문서]\n");
    for (i, r) in results.iter().enumerate() {
        prefix.push_str(&format!(
            "{}. [출처: {}] (유사도: {:.2})\n{}\n\n",
            i + 1,
            r.source,
            r.score,
            r.content,
        ));
    }
    Some(prefix)
}

// ─── Streaming helpers ────────────────────────────────────────────────────────

/// Send one JSON event into the bounded channel.
/// Awaits when the buffer is full, providing natural backpressure.
/// Silently drops if the consumer (SSE/WS client) has disconnected.
async fn emit_event(tx: &mpsc::Sender<String>, v: serde_json::Value) {
    let _ = tx.send(v.to_string()).await;
}

// ─── Streaming query loop ─────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_stream_loop(
    agent_id:     String,
    content:      String,
    session_id:   Uuid,
    tenant_id:    String,
    user_id:      String,
    conv:         Arc<PgConversationStore>,
    tools:        Arc<RwLock<ToolRegistry>>,
    claude:       Arc<ClaudeClient>,
    gemini:       Option<Arc<GeminiClient>>,
    openai:       Option<Arc<OpenAIClient>>,
    cfg:          LlmConfig,
    provider:     String,
    tx:           mpsc::Sender<String>,
    profile:      AgentProfile,
    vector_store: Option<Arc<PgVectorStore>>,
    metrics:      Option<Arc<MetricsRegistry>>,
    cb:           Option<CircuitBreaker>,
) {
    let _ = conv.append_message(&session_id, Message::user(&content)).await;

    let rag_prefix = build_rag_prefix(&content, vector_store.as_deref()).await;

    let history: Vec<Message> = conv.get_conversation(&session_id).await.unwrap_or_default();
    let mut messages: Vec<ClaudeMessage> = history.iter().map(|m| ClaudeMessage {
        role: match m.role {
            MessageRole::Assistant => "assistant".to_string(),
            _ => "user".to_string(),
        },
        content: vec![ContentBlock::Text { text: m.content.clone() }],
    }).collect();

    if let Some(prefix) = rag_prefix {
        if messages.is_empty() {
            messages.push(ClaudeMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::Text { text: prefix }],
            });
            messages.push(ClaudeMessage {
                role: "assistant".to_string(),
                content: vec![ContentBlock::Text { text: "관련 컨텍스트를 확인했습니다. 질문에 답변하겠습니다.".to_string() }],
            });
        } else if let Some(last) = messages.last_mut() {
            if last.role == "user" {
                if let Some(ContentBlock::Text { text }) = last.content.last_mut() {
                    text.push_str("\n\n");
                    text.push_str(&prefix);
                }
            }
        }
    }

    let all_tool_defs = tools.read().await.list_tools();
    let tool_defs: Vec<ToolDefinition> = match &profile.tool_whitelist {
        Some(wl) => all_tool_defs.into_iter().filter(|t| wl.contains(&t.name)).collect(),
        None     => all_tool_defs,
    };
    let claude_tools  = tool_definitions_to_claude_tools(&tool_defs);
    let system_prompt = build_system_prompt(&agent_id);
    let max_tokens    = profile.max_tokens_override.unwrap_or(cfg.max_tokens);

    let mut all_tools_used: Vec<String> = Vec::new();
    let mut total_input:  u32 = 0;
    let mut total_output: u32 = 0;
    let mut final_text = String::new();

    if let Some(m) = &metrics {
        m.agent_queries_total
            .with_label_values(&[&agent_id, &tenant_id])
            .inc();
    }

    'outer: for round in 0..MAX_TOOL_ROUNDS {
        emit_event(&tx, serde_json::json!({"type": "thinking", "round": round})).await;

        let req = LlmRequest {
            system_prompt: system_prompt.clone(),
            messages: messages.clone(),
            tools: claude_tools.clone(),
            max_tokens,
            temperature: Some(cfg.temperature),
            cacheable: false,
        };

        let llm_start = std::time::Instant::now();
        let resp = match call_llm_protected(
            &provider, &req,
            &claude,
            gemini.as_deref(),
            openai.as_deref(),
            cb.as_ref(),
            metrics.as_deref(),
        ).await {
            Ok(r) => {
                let elapsed = llm_start.elapsed().as_secs_f64();
                if let Some(m) = &metrics {
                    m.llm_requests_total
                        .with_label_values(&[&provider, &agent_id, "ok"])
                        .inc();
                    m.llm_request_duration_secs
                        .with_label_values(&[&provider, &agent_id])
                        .observe(elapsed);
                    m.llm_input_tokens_total
                        .with_label_values(&[&provider])
                        .inc_by(r.usage.input_tokens as f64);
                    m.llm_output_tokens_total
                        .with_label_values(&[&provider])
                        .inc_by(r.usage.output_tokens as f64);
                }
                r
            }
            Err(e) => {
                if let Some(m) = &metrics {
                    m.llm_requests_total
                        .with_label_values(&[&provider, &agent_id, "error"])
                        .inc();
                }
                emit_event(&tx, serde_json::json!({"type":"error","message": e})).await;
                break 'outer;
            }
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
                emit_event(&tx, serde_json::json!({"type": "tool_use", "tool": tool_name, "round": round})).await;
                all_tools_used.push(tool_name.clone());

                let tool_start = std::time::Instant::now();
                let result = {
                    let mut reg = tools.write().await;
                    reg.execute(tool_name, tool_input.clone(), &ctx).await
                };
                let tool_elapsed = tool_start.elapsed().as_secs_f64();

                let (content_str, is_error, ok) = match result {
                    Ok(out) => {
                        let ok = !out.is_error;
                        if let Some(m) = &metrics {
                            m.tool_executions_total
                                .with_label_values(&[tool_name, if ok { "ok" } else { "error" }])
                                .inc();
                            m.tool_duration_secs
                                .with_label_values(&[tool_name])
                                .observe(tool_elapsed);
                        }
                        let s = truncate_tool_result(
                            &serde_json::to_string(&out.content).unwrap_or_default()
                        );
                        (s, if out.is_error { Some(true) } else { None }, ok)
                    }
                    Err(e) => {
                        warn!(tool = %tool_name, error = %e, "Tool failed in stream");
                        if let Some(m) = &metrics {
                            m.tool_executions_total
                                .with_label_values(&[tool_name, "error"])
                                .inc();
                        }
                        (format!("{{\"error\":\"{e}\"}}"), Some(true), false)
                    }
                };

                emit_event(&tx, serde_json::json!({"type": "tool_result", "tool": tool_name, "ok": ok})).await;
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

    emit_event(&tx, serde_json::json!({"type": "text_chunk", "text": final_text})).await;
    emit_event(&tx, serde_json::json!({
        "type": "done",
        "session_id": session_id,
        "tools_used": all_tools_used,
        "input_tokens": total_input,
        "output_tokens": total_output
    })).await;
}

// ─── Workflow execution engine ────────────────────────────────────────────────

/// Execute all steps in topological (dependency) order.
/// Each step gets its own LLM call via `run_query_loop`.
/// If `event_tx` is `Some`, SSE events are emitted throughout.
#[allow(clippy::too_many_arguments)]
async fn run_workflow_engine(
    mut workflow: Workflow,
    conv:         Arc<PgConversationStore>,
    tools:        Arc<RwLock<ToolRegistry>>,
    claude:       Arc<ClaudeClient>,
    gemini:       Option<Arc<GeminiClient>>,
    openai:       Option<Arc<OpenAIClient>>,
    cfg:          LlmConfig,
    provider:     String,
    profiles:     HashMap<String, AgentProfile>,
    event_tx:     Option<mpsc::Sender<String>>,
) -> Result<WorkflowResult, String> {
    let wf_id   = workflow.id;
    let wf_name = workflow.name.clone();

    if let Some(tx) = &event_tx {
        emit_event(tx, serde_json::json!({
            "type": "workflow_start",
            "workflow_id": wf_id,
            "name": wf_name,
            "total_steps": workflow.steps.len()
        })).await;
    }

    let order = match topological_sort(&workflow.steps) {
        Ok(o)  => o,
        Err(e) => {
            if let Some(tx) = &event_tx {
                emit_event(tx, serde_json::json!({"type":"error","message": e})).await;
            }
            return Err(e);
        }
    };

    let mut context: HashMap<String, serde_json::Value> = workflow.context.clone();
    let mut completed_steps: Vec<String> = Vec::new();

    for (exec_idx, &step_idx) in order.iter().enumerate() {
        let step      = workflow.steps[step_idx].clone();
        let step_id   = step.step_id.clone();
        let agent_type = step.agent_type.clone();

        workflow.status = WorkflowStatus::Running { current_step: exec_idx };

        if let Some(tx) = &event_tx {
            emit_event(tx, serde_json::json!({
                "type": "step_start",
                "step_id": step_id,
                "agent_type": agent_type,
                "step_index": exec_idx
            })).await;
        }

        let content  = build_step_content(&step, &context);
        let profile  = profiles.get(&agent_type).cloned().unwrap_or_default();
        let step_sid = Uuid::new_v4();
        let timeout  = Duration::from_millis(step.timeout_ms);

        let result = tokio::time::timeout(
            timeout,
            run_query_loop(
                agent_type.clone(), content, step_sid,
                "workflow".to_string(), "system".to_string(),
                conv.clone(), tools.clone(),
                claude.clone(), gemini.clone(), openai.clone(),
                cfg.clone(), provider.clone(), profile, None, None, None,
            ),
        ).await;

        match result {
            Ok(Ok(qr)) => {
                context.insert(
                    step.output_key.clone(),
                    serde_json::Value::String(qr.response.clone()),
                );
                completed_steps.push(step_id.clone());

                if let Some(tx) = &event_tx {
                    emit_event(tx, serde_json::json!({
                        "type": "step_complete",
                        "step_id": step_id,
                        "output_key": step.output_key,
                        "response": qr.response,
                        "tools_used": qr.tools_used
                    })).await;
                }
            }
            Ok(Err(e)) => {
                let err_msg = e;
                if let Some(tx) = &event_tx {
                    emit_event(tx, serde_json::json!({
                        "type": "step_failed", "step_id": step_id, "error": err_msg
                    })).await;
                }
                workflow.status = step_failure_status(
                    &workflow.steps, &order[..exec_idx], &context,
                    &conv, &tools, &claude, &gemini, &openai,
                    &cfg, &provider, &profiles, &event_tx,
                    exec_idx, &err_msg, &wf_id,
                ).await;

                return Ok(WorkflowResult {
                    workflow_id: wf_id, name: wf_name,
                    status: workflow.status.clone(),
                    context, completed_steps,
                    failed_step: Some(step_id),
                    error: Some(err_msg),
                });
            }
            Err(_) => {
                let err_msg = format!("Step '{step_id}' timed out after {}ms", step.timeout_ms);
                if let Some(tx) = &event_tx {
                    emit_event(tx, serde_json::json!({
                        "type": "step_timeout", "step_id": step_id, "error": err_msg
                    })).await;
                }
                workflow.status = step_failure_status(
                    &workflow.steps, &order[..exec_idx], &context,
                    &conv, &tools, &claude, &gemini, &openai,
                    &cfg, &provider, &profiles, &event_tx,
                    exec_idx, &err_msg, &wf_id,
                ).await;

                return Ok(WorkflowResult {
                    workflow_id: wf_id, name: wf_name,
                    status: workflow.status.clone(),
                    context, completed_steps,
                    failed_step: Some(step_id),
                    error: Some(err_msg),
                });
            }
        }
    }

    workflow.status = WorkflowStatus::Completed;
    if let Some(tx) = &event_tx {
        emit_event(tx, serde_json::json!({
            "type": "workflow_complete",
            "workflow_id": wf_id,
            "completed_steps": completed_steps
        })).await;
    }

    Ok(WorkflowResult {
        workflow_id: wf_id,
        name:        wf_name,
        status:      WorkflowStatus::Completed,
        context,
        completed_steps,
        failed_step: None,
        error:       None,
    })
}

// ─── Workflow compensation helpers ───────────────────────────────────────────

/// Determine the final `WorkflowStatus` after a step fails.
/// Runs compensating actions in reverse for any completed steps that declared
/// `compensation_action`; returns the appropriate terminal status.
#[allow(clippy::too_many_arguments)]
async fn step_failure_status(
    steps:           &[crate::workflow::WorkflowStep],
    completed_order: &[usize],
    context:         &HashMap<String, serde_json::Value>,
    conv:            &Arc<PgConversationStore>,
    tools:           &Arc<RwLock<ToolRegistry>>,
    claude:          &Arc<ClaudeClient>,
    gemini:          &Option<Arc<GeminiClient>>,
    openai:          &Option<Arc<OpenAIClient>>,
    cfg:             &LlmConfig,
    provider:        &str,
    profiles:        &HashMap<String, AgentProfile>,
    event_tx:        &Option<mpsc::Sender<String>>,
    failed_at:       usize,
    err_msg:         &str,
    wf_id:           &Uuid,
) -> WorkflowStatus {
    let comp_results = run_workflow_compensations(
        steps, completed_order, context,
        conv, tools, claude, gemini, openai,
        cfg, provider, profiles, event_tx, failed_at,
    ).await;

    if comp_results.is_empty() {
        return WorkflowStatus::Failed { at_step: failed_at, error: err_msg.to_string() };
    }

    let comp_failed = comp_results.iter().any(|r| r.starts_with("FAILED:"));
    if let Some(tx) = event_tx {
        emit_event(tx, serde_json::json!({
            "type": if comp_failed { "workflow_compensation_failed" } else { "workflow_compensated" },
            "workflow_id":       wf_id,
            "compensated_steps": comp_results.len()
        })).await;
    }

    if comp_failed {
        WorkflowStatus::CompensationFailed { error: err_msg.to_string() }
    } else {
        WorkflowStatus::Compensated
    }
}

/// Run compensating LLM calls for completed steps in reverse order.
/// Returns one String per compensated step (prefixed "FAILED:" on error).
#[allow(clippy::too_many_arguments)]
async fn run_workflow_compensations(
    steps:           &[crate::workflow::WorkflowStep],
    completed_order: &[usize],
    context:         &HashMap<String, serde_json::Value>,
    conv:            &Arc<PgConversationStore>,
    tools:           &Arc<RwLock<ToolRegistry>>,
    claude:          &Arc<ClaudeClient>,
    gemini:          &Option<Arc<GeminiClient>>,
    openai:          &Option<Arc<OpenAIClient>>,
    cfg:             &LlmConfig,
    provider:        &str,
    profiles:        &HashMap<String, AgentProfile>,
    event_tx:        &Option<mpsc::Sender<String>>,
    failed_at:       usize,
) -> Vec<String> {
    let mut results = Vec::new();

    for &step_idx in completed_order.iter().rev() {
        let step = &steps[step_idx];
        let Some(comp_action) = &step.compensation_action else { continue; };

        let prior_output = context.get(&step.output_key)
            .and_then(|v| v.as_str())
            .unwrap_or("(결과 없음)");

        let comp_content = format!(
            "[보상 트랜잭션] 워크플로우 단계 '{}' 의 작업을 취소/보상합니다.\n\
             실패 위치: step {failed_at}\n\n\
             보상 지침: {comp_action}\n\n\
             이전 단계 출력:\n{prior_output}",
            step.step_id,
        );

        if let Some(tx) = event_tx {
            emit_event(tx, serde_json::json!({
                "type":     "compensation_step_start",
                "step_id":  step.step_id,
                "step_idx": step_idx
            })).await;
        }

        let profile = profiles.get(&step.agent_type).cloned().unwrap_or_default();
        let timeout = Duration::from_millis(step.timeout_ms);

        let comp_result = tokio::time::timeout(
            timeout,
            run_query_loop(
                step.agent_type.clone(), comp_content, Uuid::new_v4(),
                "workflow-compensation".to_string(), "system".to_string(),
                conv.clone(), tools.clone(),
                claude.clone(), gemini.clone(), openai.clone(),
                cfg.clone(), provider.to_string(), profile,
                None, None, None,
            ),
        ).await;

        let entry = match comp_result {
            Ok(Ok(qr)) => {
                if let Some(tx) = event_tx {
                    emit_event(tx, serde_json::json!({
                        "type":     "compensation_step_complete",
                        "step_id":  step.step_id,
                        "response": qr.response
                    })).await;
                }
                qr.response
            }
            Ok(Err(e)) => {
                warn!(step_id = %step.step_id, error = %e, "Workflow compensation step failed");
                if let Some(tx) = event_tx {
                    emit_event(tx, serde_json::json!({
                        "type":    "compensation_step_failed",
                        "step_id": step.step_id,
                        "error":   e
                    })).await;
                }
                format!("FAILED: {e}")
            }
            Err(_) => {
                let msg = format!("Compensation for '{}' timed out", step.step_id);
                warn!("{msg}");
                format!("FAILED: {msg}")
            }
        };
        results.push(entry);
    }

    results
}
