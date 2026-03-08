//! Orchestrator — now a first-class pekko_actor::Actor.
//!
//! The `OrchestratorActor` can be spawned into an `ActorSystem`:
//!
//! ```rust,ignore
//! let system = ActorSystem::new("ehs-system");
//! let orch   = OrchestratorActor::new();
//! let orch_ref = system.spawn(orch, "orchestrator").await?;
//! orch_ref.tell(OrchestratorMessage::RegisterAgent(info));
//! ```

use async_trait::async_trait;
use pekko_agent_core::{AgentInfo, AgentStatus, AgentTask};
use pekko_actor::{Actor, ActorContext};
use crate::workflow::Workflow;
use crate::saga::SagaManager;
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use tracing::{info, warn};

// ─── Messages ────────────────────────────────────────────────────────────────

/// All messages the OrchestratorActor can receive through its mailbox.
#[derive(Debug)]
pub enum OrchestratorMessage {
    RegisterAgent(AgentInfo),
    CreateWorkflow(Workflow),
    SubmitTask(AgentTask),
    AssignNextTask,
    CompleteTask { task_id: Uuid, result: serde_json::Value },
    FailTask   { task_id: Uuid, error: String },
}

// ─── Actor state ─────────────────────────────────────────────────────────────

pub struct OrchestratorActor {
    workflows:      HashMap<Uuid, Workflow>,
    agent_registry: HashMap<String, AgentInfo>,
    task_queue:     VecDeque<AgentTask>,
    active_tasks:   HashMap<Uuid, TaskExecution>,
    #[allow(dead_code)]
    saga_manager:   SagaManager,
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

#[async_trait]
impl Actor for OrchestratorActor {
    type Message = OrchestratorMessage;

    async fn pre_start(&mut self) {
        info!("OrchestratorActor started inside ActorSystem");
    }

    async fn receive(&mut self, msg: Self::Message, _ctx: &mut ActorContext<Self>) {
        match msg {
            OrchestratorMessage::RegisterAgent(agent) => {
                self.register_agent(agent);
            }
            OrchestratorMessage::CreateWorkflow(wf) => {
                self.create_workflow(wf);
            }
            OrchestratorMessage::SubmitTask(task) => {
                self.submit_task(task);
            }
            OrchestratorMessage::AssignNextTask => {
                self.assign_next_task();
            }
            OrchestratorMessage::CompleteTask { task_id, result } => {
                self.complete_task(&task_id, result);
            }
            OrchestratorMessage::FailTask { task_id, error } => {
                self.fail_task(&task_id, error);
            }
        }
    }

    async fn post_stop(&mut self) {
        info!("OrchestratorActor stopped");
    }
}

// ─── Business logic (unchanged from original) ────────────────────────────────

impl OrchestratorActor {
    pub fn new() -> Self {
        Self {
            workflows:      HashMap::new(),
            agent_registry: HashMap::new(),
            task_queue:     VecDeque::new(),
            active_tasks:   HashMap::new(),
            saga_manager:   SagaManager::new(),
        }
    }

    pub fn register_agent(&mut self, agent: AgentInfo) {
        info!(agent_id = %agent.agent_id, agent_type = %agent.agent_type, "Agent registered");
        self.agent_registry.insert(agent.agent_id.clone(), agent);
    }

    pub fn create_workflow(&mut self, workflow: Workflow) -> Uuid {
        let id = workflow.id;
        info!(workflow_id = %id, name = %workflow.name, "Workflow created");
        self.workflows.insert(id, workflow);
        id
    }

    pub fn submit_task(&mut self, task: AgentTask) {
        info!(task_id = %task.task_id, description = %task.description, "Task submitted");
        self.task_queue.push_back(task);
    }

    pub fn assign_next_task(&mut self) -> Option<(String, AgentTask)> {
        let task = self.task_queue.pop_front()?;

        let available_agent = self.agent_registry.values()
            .find(|a| matches!(a.status, AgentStatus::Available))
            .map(|a| a.agent_id.clone());

        if let Some(agent_id) = available_agent {
            let execution = TaskExecution {
                task:           task.clone(),
                assigned_agent: agent_id.clone(),
                status:         TaskExecutionStatus::Running,
                started_at:     Utc::now(),
                completed_at:   None,
            };
            self.active_tasks.insert(task.task_id, execution);

            if let Some(agent) = self.agent_registry.get_mut(&agent_id) {
                agent.status = AgentStatus::Busy;
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
        if let Some(execution) = self.active_tasks.get_mut(task_id) {
            execution.status      = TaskExecutionStatus::Completed;
            execution.completed_at = Some(Utc::now());

            if let Some(agent) = self.agent_registry.get_mut(&execution.assigned_agent) {
                agent.status = AgentStatus::Available;
            }
            info!(task_id = %task_id, "Task completed");
        }
    }

    pub fn fail_task(&mut self, task_id: &Uuid, error: String) {
        if let Some(execution) = self.active_tasks.get_mut(task_id) {
            execution.status      = TaskExecutionStatus::Failed(error.clone());
            execution.completed_at = Some(Utc::now());

            if let Some(agent) = self.agent_registry.get_mut(&execution.assigned_agent) {
                agent.status = AgentStatus::Available;
            }
            warn!(task_id = %task_id, error = %error, "Task failed");
        }
    }

    pub fn get_workflow(&self, id: &Uuid) -> Option<&Workflow> { self.workflows.get(id) }
    pub fn list_agents(&self)  -> Vec<&AgentInfo>              { self.agent_registry.values().collect() }
    pub fn pending_tasks(&self) -> usize                        { self.task_queue.len() }
    pub fn active_task_count(&self) -> usize {
        self.active_tasks.values().filter(|t| t.status == TaskExecutionStatus::Running).count()
    }
}
