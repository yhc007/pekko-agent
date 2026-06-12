use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};

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
    /// Optional LLM prompt that undoes this step's side-effects.
    /// Executed when a later step fails and the saga compensates.
    #[serde(default)]
    pub compensation_action: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum WorkflowStatus {
    Created,
    Running { current_step: usize },
    Paused { at_step: usize },
    Completed,
    Failed { at_step: usize, error: String },
    Cancelled,
    /// Saga compensation in progress after a step failure.
    Compensating { failed_at: usize, compensating_step: usize },
    /// All compensation steps completed successfully.
    Compensated,
    /// One or more compensation steps also failed.
    CompensationFailed { error: String },
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

// ─── Execution result ─────────────────────────────────────────────────────────

/// Returned by `ExecuteWorkflow` and `StreamWorkflow` when the run finishes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub workflow_id:     Uuid,
    pub name:            String,
    pub status:          WorkflowStatus,
    /// Accumulated outputs from every step (keyed by `output_key`).
    pub context:         HashMap<String, serde_json::Value>,
    pub completed_steps: Vec<String>,
    pub failed_step:     Option<String>,
    pub error:           Option<String>,
}

// ─── Topological sort (Kahn's algorithm) ─────────────────────────────────────

/// Returns step indices in dependency order.
/// Errors on unknown dependency references or cycles.
pub fn topological_sort(steps: &[WorkflowStep]) -> Result<Vec<usize>, String> {
    let n = steps.len();
    let id_to_idx: HashMap<&str, usize> = steps.iter()
        .enumerate().map(|(i, s)| (s.step_id.as_str(), i)).collect();

    let mut in_degree = vec![0usize; n];
    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];

    for (i, step) in steps.iter().enumerate() {
        for dep in &step.depends_on {
            let j = id_to_idx.get(dep.as_str())
                .copied()
                .ok_or_else(|| format!("Step '{}' depends on unknown step '{dep}'", step.step_id))?;
            adj[j].push(i);
            in_degree[i] += 1;
        }
    }

    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);

    while let Some(u) = queue.pop_front() {
        order.push(u);
        for &v in &adj[u] {
            in_degree[v] -= 1;
            if in_degree[v] == 0 { queue.push_back(v); }
        }
    }

    if order.len() == n {
        Ok(order)
    } else {
        Err("Workflow dependency graph has a cycle".to_string())
    }
}

// ─── Step content builder ─────────────────────────────────────────────────────

/// Build the LLM query string for a step.
///
/// The step's `action` becomes the base query.
/// Inputs resolved from `context` via `input_mapping` are appended as
/// structured context so the LLM can reference them without hallucination.
pub fn build_step_content(
    step:    &WorkflowStep,
    context: &HashMap<String, serde_json::Value>,
) -> String {
    let mut content = step.action.clone();

    let resolved: serde_json::Map<String, serde_json::Value> = step.input_mapping
        .iter()
        .filter_map(|(param, ctx_key)| {
            context.get(ctx_key).map(|v| (param.clone(), v.clone()))
        })
        .collect();

    if !resolved.is_empty() {
        content.push_str("\n\n[이전 단계 컨텍스트]\n");
        content.push_str(
            &serde_json::to_string_pretty(&resolved).unwrap_or_default()
        );
    }

    content
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_step(id: &str) -> WorkflowStep {
        WorkflowStep {
            step_id:           id.into(),
            agent_type:        "ehs".into(),
            action:            "do something".into(),
            input_mapping:     HashMap::new(),
            output_key:        "result".into(),
            depends_on:        vec![],
            timeout_ms:        5_000,
            compensation_action: None,
        }
    }

    #[test]
    fn new_workflow_starts_in_created_status() {
        let wf = Workflow::new("permit-flow", "EHS permit workflow");
        assert_eq!(wf.status, WorkflowStatus::Created);
        assert_eq!(wf.name, "permit-flow");
        assert!(wf.steps.is_empty());
    }

    #[test]
    fn add_step_appends_to_steps_vec() {
        let mut wf = Workflow::new("w", "d");
        wf.add_step(make_step("s1"));
        wf.add_step(make_step("s2"));
        assert_eq!(wf.steps.len(), 2);
        assert_eq!(wf.steps[0].step_id, "s1");
    }

    #[test]
    fn workflow_status_all_variants_serialize_round_trip() {
        use WorkflowStatus::*;
        let variants: Vec<WorkflowStatus> = vec![
            Created,
            Running { current_step: 3 },
            Paused  { at_step: 1 },
            Completed,
            Failed  { at_step: 2, error: "timeout".into() },
            Cancelled,
            Compensating { failed_at: 2, compensating_step: 1 },
            Compensated,
            CompensationFailed { error: "db down".into() },
        ];
        for v in variants {
            let json = serde_json::to_string(&v).expect("serialize");
            let rt: WorkflowStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(v, rt);
        }
    }

    #[test]
    fn workflow_step_compensation_action_optional_defaults_none() {
        let json = r#"{
            "step_id":       "s1",
            "agent_type":    "ehs",
            "action":        "check",
            "input_mapping": {},
            "output_key":    "r",
            "depends_on":    [],
            "timeout_ms":    1000
        }"#;
        let step: WorkflowStep = serde_json::from_str(json).unwrap();
        assert!(step.compensation_action.is_none());
    }

    #[test]
    fn workflow_step_with_compensation_serializes() {
        let mut step = make_step("s1");
        step.compensation_action = Some("undo permit request".into());
        let json = serde_json::to_string(&step).unwrap();
        let rt: WorkflowStep = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.compensation_action.as_deref(), Some("undo permit request"));
    }

    #[test]
    fn workflow_serializes_round_trip() {
        let mut wf = Workflow::new("flow", "desc");
        wf.add_step(make_step("step-1"));
        wf.status = WorkflowStatus::Running { current_step: 0 };
        let json = serde_json::to_string(&wf).unwrap();
        let rt: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.id, wf.id);
        assert_eq!(rt.name, "flow");
        assert_eq!(rt.steps.len(), 1);
    }
}
