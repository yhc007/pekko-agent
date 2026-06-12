use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

use pekko_agent_core::AgentProfile;
use pekko_agent_llm::{ClaudeClient, GeminiClient, LlmConfig, OpenAIClient};
use pekko_agent_memory::PgConversationStore;
use pekko_agent_tools::ToolRegistry;

// ─── Data structures ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SagaDefinition {
    pub saga_id: Uuid,
    pub name: String,
    /// Linear steps; executed in order and compensated in reverse on failure.
    pub steps: Vec<SagaStep>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SagaStep {
    pub step_name: String,
    pub agent_type: String,
    /// Forward LLM action.
    pub action: String,
    /// Compensating LLM action (runs in reverse on failure).
    pub compensation_action: String,
    /// Milliseconds before the step is considered timed out.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 { 30_000 }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SagaExecution {
    pub execution_id:          Uuid,
    pub saga:                  SagaDefinition,
    pub completed_steps:       Vec<usize>,
    pub compensation_results:  Vec<String>,
    pub status:                SagaStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SagaStatus {
    Running,
    Completed,
    /// A step failed; compensating completed steps in reverse.
    Compensating { failed_at: usize },
    CompensationCompleted,
    CompensationFailed { error: String },
    Failed { error: String },
}

// ─── SagaManager (in-memory, embedded in OrchestratorActor) ───────────────────

pub struct SagaManager {
    sagas:      HashMap<Uuid, SagaDefinition>,
    executions: HashMap<Uuid, SagaExecution>,
}

impl SagaManager {
    pub fn new() -> Self {
        Self { sagas: HashMap::new(), executions: HashMap::new() }
    }

    pub fn register(&mut self, saga: SagaDefinition) {
        info!(saga_id = %saga.saga_id, name = %saga.name, "Saga registered");
        self.sagas.insert(saga.saga_id, saga);
    }

    pub fn get_definition(&self, saga_id: &Uuid) -> Option<&SagaDefinition> {
        self.sagas.get(saga_id)
    }

    pub fn all_definitions(&self) -> Vec<&SagaDefinition> {
        self.sagas.values().collect()
    }

    pub fn insert_execution(&mut self, exec: SagaExecution) {
        self.executions.insert(exec.execution_id, exec);
    }

    pub fn update_execution(&mut self, exec: SagaExecution) {
        self.executions.insert(exec.execution_id, exec);
    }

    pub fn get_execution(&self, id: &Uuid) -> Option<&SagaExecution> {
        self.executions.get(id)
    }
}

// ─── Execution result ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SagaResult {
    pub execution_id:         Uuid,
    pub saga_id:              Uuid,
    pub name:                 String,
    pub status:               SagaStatus,
    pub completed_steps:      Vec<usize>,
    pub compensation_results: Vec<String>,
    pub failed_step:          Option<String>,
    pub error:                Option<String>,
}

// ─── Standalone saga execution engine ─────────────────────────────────────────

/// Execute a `SagaDefinition` linearly; compensate on failure.
///
/// Each step and compensation step is a separate LLM call via `run_query_loop`.
/// If `event_tx` is provided, SSE events are emitted throughout.
#[allow(clippy::too_many_arguments)]
pub async fn run_saga_engine(
    saga:     SagaDefinition,
    conv:     Arc<PgConversationStore>,
    tools:    Arc<RwLock<ToolRegistry>>,
    claude:   Arc<ClaudeClient>,
    gemini:   Option<Arc<GeminiClient>>,
    openai:   Option<Arc<OpenAIClient>>,
    cfg:      LlmConfig,
    provider: String,
    profiles: HashMap<String, AgentProfile>,
    event_tx: Option<mpsc::Sender<String>>,
) -> SagaResult {
    let execution_id = Uuid::new_v4();
    let saga_id      = saga.saga_id;
    let saga_name    = saga.name.clone();

    if let Some(tx) = &event_tx {
        emit_event(tx, serde_json::json!({
            "type":         "saga_start",
            "execution_id": execution_id,
            "saga_id":      saga_id,
            "name":         saga_name,
            "total_steps":  saga.steps.len()
        })).await;
    }

    let mut completed: Vec<usize> = Vec::new();
    let mut step_outputs: HashMap<String, String> = HashMap::new();

    for (idx, step) in saga.steps.iter().enumerate() {
        if let Some(tx) = &event_tx {
            emit_event(tx, serde_json::json!({
                "type":      "saga_step_start",
                "step_name": step.step_name,
                "step_index": idx
            })).await;
        }

        let profile = profiles.get(&step.agent_type).cloned().unwrap_or_default();
        let timeout = Duration::from_millis(step.timeout_ms);

        let result = tokio::time::timeout(
            timeout,
            super::orchestrator::run_query_loop_pub(
                step.agent_type.clone(),
                step.action.clone(),
                Uuid::new_v4(),
                "saga".to_string(),
                "system".to_string(),
                conv.clone(), tools.clone(),
                claude.clone(), gemini.clone(), openai.clone(),
                cfg.clone(), provider.clone(), profile,
                None, None, None,
            ),
        ).await;

        let err_msg_opt: Option<String> = match result {
            Ok(Ok(qr)) => {
                step_outputs.insert(step.step_name.clone(), qr.response.clone());
                completed.push(idx);
                if let Some(tx) = &event_tx {
                    emit_event(tx, serde_json::json!({
                        "type":       "saga_step_complete",
                        "step_name":  step.step_name,
                        "step_index": idx,
                        "response":   qr.response
                    })).await;
                }
                None
            }
            Ok(Err(e)) => Some(e),
            Err(_) => Some(format!("Step '{}' timed out after {}ms", step.step_name, step.timeout_ms)),
        };

        if let Some(err_msg) = err_msg_opt {
            warn!(execution_id = %execution_id, step = %step.step_name, error = %err_msg, "Saga step failed");
            if let Some(tx) = &event_tx {
                emit_event(tx, serde_json::json!({
                    "type":      "saga_step_failed",
                    "step_name": step.step_name,
                    "error":     err_msg
                })).await;
            }

            let comp_results = run_compensations(
                &saga.steps, &completed, &step_outputs,
                &conv, &tools, &claude, &gemini, &openai,
                &cfg, &provider, &profiles, &event_tx,
                execution_id,
            ).await;

            let comp_failed = comp_results.iter().any(|r| r.starts_with("FAILED:"));
            let status = if comp_failed {
                SagaStatus::CompensationFailed { error: err_msg.clone() }
            } else {
                SagaStatus::CompensationCompleted
            };

            if let Some(tx) = &event_tx {
                emit_event(tx, serde_json::json!({
                    "type": if comp_failed { "saga_compensation_failed" } else { "saga_compensated" },
                    "execution_id":      execution_id,
                    "compensated_steps": comp_results.len()
                })).await;
            }

            return SagaResult {
                execution_id,
                saga_id,
                name:                 saga_name,
                status,
                completed_steps:      completed,
                compensation_results: comp_results,
                failed_step:          Some(step.step_name.clone()),
                error:                Some(err_msg),
            };
        }
    }

    info!(execution_id = %execution_id, "Saga completed successfully");
    if let Some(tx) = &event_tx {
        emit_event(tx, serde_json::json!({
            "type":            "saga_complete",
            "execution_id":    execution_id,
            "completed_steps": completed.len()
        })).await;
    }

    SagaResult {
        execution_id,
        saga_id,
        name: saga_name,
        status: SagaStatus::Completed,
        completed_steps: completed,
        compensation_results: Vec::new(),
        failed_step: None,
        error: None,
    }
}

// ─── Shared compensation runner ───────────────────────────────────────────────

/// Run compensating actions for all completed steps in reverse order.
/// Returns one string per compensated step; prefixed with "FAILED:" on error.
#[allow(clippy::too_many_arguments)]
pub async fn run_compensations(
    steps:        &[SagaStep],
    completed:    &[usize],
    step_outputs: &HashMap<String, String>,
    conv:         &Arc<PgConversationStore>,
    tools:        &Arc<RwLock<ToolRegistry>>,
    claude:       &Arc<ClaudeClient>,
    gemini:       &Option<Arc<GeminiClient>>,
    openai:       &Option<Arc<OpenAIClient>>,
    cfg:          &LlmConfig,
    provider:     &str,
    profiles:     &HashMap<String, AgentProfile>,
    event_tx:     &Option<mpsc::Sender<String>>,
    execution_id: Uuid,
) -> Vec<String> {
    let mut results = Vec::new();

    for &idx in completed.iter().rev() {
        let step = &steps[idx];
        let prior_output = step_outputs.get(&step.step_name)
            .map(|s| s.as_str()).unwrap_or("(결과 없음)");

        let comp_content = format!(
            "[보상 트랜잭션] 단계 '{}' 의 작업을 취소/보상합니다.\n\n\
             보상 지침: {}\n\n\
             이전 단계 출력:\n{}",
            step.step_name,
            step.compensation_action,
            prior_output,
        );

        if let Some(tx) = event_tx {
            emit_event(tx, serde_json::json!({
                "type":         "compensation_step_start",
                "execution_id": execution_id,
                "step_name":    step.step_name,
                "step_index":   idx
            })).await;
        }

        let profile = profiles.get(&step.agent_type).cloned().unwrap_or_default();
        let timeout = Duration::from_millis(step.timeout_ms);

        let comp_result = tokio::time::timeout(
            timeout,
            super::orchestrator::run_query_loop_pub(
                step.agent_type.clone(),
                comp_content,
                Uuid::new_v4(),
                "saga-compensation".to_string(),
                "system".to_string(),
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
                        "type":      "compensation_step_complete",
                        "step_name": step.step_name,
                        "response":  qr.response
                    })).await;
                }
                qr.response
            }
            Ok(Err(e)) => {
                warn!(step = %step.step_name, error = %e, "Compensation step failed");
                if let Some(tx) = event_tx {
                    emit_event(tx, serde_json::json!({
                        "type":      "compensation_step_failed",
                        "step_name": step.step_name,
                        "error":     e
                    })).await;
                }
                format!("FAILED: {e}")
            }
            Err(_) => {
                let msg = format!("Compensation for '{}' timed out", step.step_name);
                warn!("{msg}");
                format!("FAILED: {msg}")
            }
        };
        results.push(entry);
    }

    results
}

// ─── Helper ───────────────────────────────────────────────────────────────────

pub async fn emit_event(tx: &mpsc::Sender<String>, v: serde_json::Value) {
    let _ = tx.send(serde_json::to_string(&v).unwrap_or_default()).await;
}
