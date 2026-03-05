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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
