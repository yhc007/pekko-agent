use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub steps: Vec<WorkflowStep>,
    pub status: WorkflowStatus,
    pub created_at: DateTime<Utc>,
    pub context: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub step_id: String,
    pub agent_type: String,
    pub action: String,
    pub input_mapping: HashMap<String, String>,
    pub output_key: String,
    pub depends_on: Vec<String>,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum WorkflowStatus {
    Created,
    Running { current_step: usize },
    Paused { at_step: usize },
    Completed,
    Failed { at_step: usize, error: String },
    Cancelled,
}

impl Workflow {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            steps: Vec::new(),
            status: WorkflowStatus::Created,
            created_at: Utc::now(),
            context: HashMap::new(),
        }
    }

    pub fn add_step(&mut self, step: WorkflowStep) {
        self.steps.push(step);
    }

    pub fn current_step(&self) -> Option<&WorkflowStep> {
        match &self.status {
            WorkflowStatus::Running { current_step } => self.steps.get(*current_step),
            _ => None,
        }
    }

    pub fn advance(&mut self) -> bool {
        match &self.status {
            WorkflowStatus::Running { current_step } => {
                let next = current_step + 1;
                if next < self.steps.len() {
                    self.status = WorkflowStatus::Running { current_step: next };
                    true
                } else {
                    self.status = WorkflowStatus::Completed;
                    false
                }
            }
            WorkflowStatus::Created => {
                if !self.steps.is_empty() {
                    self.status = WorkflowStatus::Running { current_step: 0 };
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

// ─── pekko_actor FSM-backed executor ─────────────────────────────────────────
//
// Wraps a Workflow in a `pekko_actor::FsmStateMachine` so that each lifecycle
// transition is governed by the FSM engine (entry/exit actions, history,
// timeout support) rather than ad-hoc `match` arms.

use pekko_actor::{FsmStateMachine, StateMachineBuilder, TransitionResult};

/// Events that drive the FSM (mirrors WorkflowStatus transitions).
#[derive(Debug, Clone)]
pub enum WorkflowEvent {
    Start,
    Advance,
    Pause,
    Resume,
    Complete,
    Cancel,
    Fail { at_step: usize, error: String },
}

/// Build a fresh FSM for the given workflow.
/// The FSM data is the step index (`usize`).
pub fn build_workflow_fsm(
    workflow: &Workflow,
) -> FsmStateMachine<WorkflowStatus, usize, WorkflowEvent> {
    use WorkflowStatus::*;
    let total = workflow.steps.len();

    StateMachineBuilder::new(WorkflowStatus::Created, 0usize)
        // Created → Running on Start
        .state(WorkflowStatus::Created)
            .on_event(
                |e| matches!(e, WorkflowEvent::Start),
                |_s, _d, _e| TransitionResult::GoTo(WorkflowStatus::Running { current_step: 0 }, Some(0)),
            )
            .on_event(
                |e| matches!(e, WorkflowEvent::Cancel),
                |_s, _d, _e| TransitionResult::GoTo(WorkflowStatus::Cancelled, None),
            )
            .end_state()
        // Running → next step / Completed / Paused / Failed
        .state(WorkflowStatus::Running { current_step: 0 })
            .on_event(
                |e| matches!(e, WorkflowEvent::Advance),
                move |s, d, _e| {
                    let next = d + 1;
                    if next < total {
                        TransitionResult::GoTo(
                            Running { current_step: next },
                            Some(next),
                        )
                    } else {
                        TransitionResult::GoTo(Completed, Some(next))
                    }
                },
            )
            .on_event(
                |e| matches!(e, WorkflowEvent::Pause),
                |_s, d, _e| TransitionResult::GoTo(Paused { at_step: *d }, Some(*d)),
            )
            .on_event(
                |e| matches!(e, WorkflowEvent::Cancel),
                |_s, _d, _e| TransitionResult::GoTo(Cancelled, None),
            )
            .on_event(
                |e| matches!(e, WorkflowEvent::Fail { .. }),
                |_s, d, e| {
                    if let WorkflowEvent::Fail { at_step, error } = e.clone() {
                        TransitionResult::GoTo(
                            Failed { at_step, error },
                            Some(*d),
                        )
                    } else {
                        TransitionResult::Stay(None)
                    }
                },
            )
            .end_state()
        // Paused → Running on Resume
        .state(WorkflowStatus::Paused { at_step: 0 })
            .on_event(
                |e| matches!(e, WorkflowEvent::Resume),
                |_s, d, _e| TransitionResult::GoTo(Running { current_step: *d }, Some(*d)),
            )
            .end_state()
        .state(WorkflowStatus::Completed)
            .end_state()
        .state(WorkflowStatus::Cancelled)
            .end_state()
        .state(WorkflowStatus::Failed { at_step: 0, error: String::new() })
            .end_state()
        .build()
}
