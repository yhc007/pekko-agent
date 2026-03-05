use async_trait::async_trait;
use pekko_agent_core::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// EHS Inspection Agent - manages safety inspections and audits
pub struct InspectionAgentActor {
    agent_id: String,
    state: AgentState,
    inspection_state: InspectionAgentState,
    execution_context: ExecutionContext,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum InspectionAgentState {
    Idle,
    PreparingInspection {
        facility_id: String,
        inspection_type: String,
    },
    SchedulingInspection {
        proposed_dates: Vec<String>,
        inspector_ids: Vec<String>,
    },
    ConductingInspection {
        inspection_id: String,
        findings: Vec<Finding>,
    },
    AssessingRisk {
        risk_level: String,
        hazard_count: usize,
    },
    GeneratingReport {
        report_id: String,
    },
    AwaitingCorrectiveAction {
        inspection_id: String,
        deadline: String,
    },
    Completed {
        inspection_id: String,
        report_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub finding_id: String,
    pub category: String,
    pub severity: String,
    pub description: String,
    pub location: String,
}

#[derive(Clone, Debug)]
struct ExecutionContext {
    facility_id: Option<String>,
    inspection_type: Option<String>,
    current_findings: Vec<Finding>,
    inspector_assignment: HashMap<String, String>,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            facility_id: None,
            inspection_type: None,
            current_findings: vec![],
            inspector_assignment: HashMap::new(),
        }
    }
}

impl InspectionAgentActor {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            state: AgentState::Idle,
            inspection_state: InspectionAgentState::Idle,
            execution_context: ExecutionContext::default(),
        }
    }

    pub fn inspection_state(&self) -> &InspectionAgentState {
        &self.inspection_state
    }

    fn parse_facility_from_query(query: &str) -> Option<String> {
        let parts: Vec<&str> = query.split_whitespace().collect();
        for part in parts {
            if part.contains("FAC-") || part.contains("FACILITY-") {
                return Some(part.to_string());
            }
        }
        None
    }

    fn parse_inspection_type_from_query(query: &str) -> Option<String> {
        let query_lower = query.to_lowercase();
        if query_lower.contains("routine") || query_lower.contains("scheduled") {
            Some("Routine".to_string())
        } else if query_lower.contains("safety") || query_lower.contains("osha") {
            Some("Safety".to_string())
        } else if query_lower.contains("hazmat") || query_lower.contains("hazardous") {
            Some("Hazardous Materials".to_string())
        } else if query_lower.contains("environmental") || query_lower.contains("epa") {
            Some("Environmental".to_string())
        } else if query_lower.contains("follow") || query_lower.contains("followup") {
            Some("Follow-up".to_string())
        } else {
            None
        }
    }
}

#[async_trait]
impl AgentActor for InspectionAgentActor {
    fn agent_id(&self) -> &str {
        &self.agent_id
    }

    fn available_tools(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "inspection_create".to_string(),
                description: "Create a new inspection record for a facility".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "facility_id": {"type": "string"},
                        "inspection_type": {"type": "string"},
                        "priority": {"type": "string"}
                    },
                    "required": ["facility_id", "inspection_type"]
                }),
                required_permissions: vec!["ehs.inspection.write".to_string()],
                timeout_ms: 5000,
                idempotent: false,
            },
            ToolDefinition {
                name: "inspection_schedule".to_string(),
                description: "Schedule an inspection with available inspectors".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "inspection_id": {"type": "string"},
                        "start_date": {"type": "string"},
                        "end_date": {"type": "string"},
                        "inspector_count": {"type": "integer"}
                    },
                    "required": ["inspection_id"]
                }),
                required_permissions: vec!["ehs.inspection.schedule".to_string()],
                timeout_ms: 8000,
                idempotent: false,
            },
            ToolDefinition {
                name: "risk_assessment".to_string(),
                description: "Perform risk assessment based on facility history".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "facility_id": {"type": "string"},
                        "assessment_scope": {"type": "string"}
                    },
                    "required": ["facility_id"]
                }),
                required_permissions: vec!["ehs.risk.read".to_string()],
                timeout_ms: 6000,
                idempotent: true,
            },
            ToolDefinition {
                name: "findings_document".to_string(),
                description: "Document inspection findings and observations".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "inspection_id": {"type": "string"},
                        "findings": {"type": "array"}
                    },
                    "required": ["inspection_id"]
                }),
                required_permissions: vec!["ehs.inspection.write".to_string()],
                timeout_ms: 10000,
                idempotent: false,
            },
        ]
    }

    fn system_prompt(&self) -> String {
        "You are an EHS Inspection Agent specialized in conducting environmental, health, and safety inspections. \
         Your responsibilities include: preparing inspection plans, scheduling inspections with qualified inspectors, \
         identifying hazards and safety violations, assessing risk levels, documenting findings, and generating \
         comprehensive inspection reports. You prioritize worker safety, regulatory compliance, and corrective action tracking. \
         Always use risk-based inspection strategies and ensure thoroughness in hazard identification.".to_string()
    }

    fn max_iterations(&self) -> u32 {
        10
    }

    async fn reason(&mut self, query: &UserQuery) -> Result<AgentAction, AgentError> {
        self.state = AgentState::Reasoning {
            query: query.content.clone(),
            iteration: 0,
            thought_chain: vec![
                "Analyzing inspection request...".to_string(),
                "Determining inspection scope...".to_string(),
            ],
        };

        // Extract facility and inspection type
        if let Some(facility) = Self::parse_facility_from_query(&query.content) {
            self.execution_context.facility_id = Some(facility);
        }

        if let Some(insp_type) = Self::parse_inspection_type_from_query(&query.content) {
            self.execution_context.inspection_type = Some(insp_type);
        }

        let content_lower = query.content.to_lowercase();

        if content_lower.contains("schedule") || content_lower.contains("plan") {
            // Schedule inspection
            let facility = self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());
            let insp_type = self.execution_context.inspection_type.clone().unwrap_or_else(|| "Routine".to_string());

            self.inspection_state = InspectionAgentState::PreparingInspection {
                facility_id: facility.clone(),
                inspection_type: insp_type,
            };

            Ok(AgentAction::UseTool(vec![
                ToolCall {
                    id: Uuid::new_v4().to_string(),
                    name: "inspection_create".to_string(),
                    input: serde_json::json!({
                        "facility_id": facility,
                        "inspection_type": self.execution_context.inspection_type.clone().unwrap_or_else(|| "Routine".to_string()),
                        "priority": "Normal"
                    }),
                },
                ToolCall {
                    id: Uuid::new_v4().to_string(),
                    name: "risk_assessment".to_string(),
                    input: serde_json::json!({
                        "facility_id": facility,
                        "assessment_scope": "Full facility risk evaluation"
                    }),
                },
            ]))
        } else if content_lower.contains("risk") || content_lower.contains("hazard") {
            // Risk assessment focused
            let facility = self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());

            self.inspection_state = InspectionAgentState::AssessingRisk {
                risk_level: "Pending".to_string(),
                hazard_count: 0,
            };

            Ok(AgentAction::UseTool(vec![ToolCall {
                id: Uuid::new_v4().to_string(),
                name: "risk_assessment".to_string(),
                input: serde_json::json!({
                    "facility_id": facility,
                    "assessment_scope": "Complete risk analysis"
                }),
            }]))
        } else if content_lower.contains("findings") || content_lower.contains("document") {
            // Document findings
            let _facility = self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());

            self.inspection_state = InspectionAgentState::GeneratingReport {
                report_id: Uuid::new_v4().to_string(),
            };

            Ok(AgentAction::UseTool(vec![ToolCall {
                id: Uuid::new_v4().to_string(),
                name: "findings_document".to_string(),
                input: serde_json::json!({
                    "inspection_id": format!("INSP-{}", Uuid::new_v4()),
                    "findings": [
                        {
                            "category": "Housekeeping",
                            "severity": "Minor",
                            "description": "Debris accumulation in work area"
                        },
                        {
                            "category": "PPE",
                            "severity": "Major",
                            "description": "Missing hard hats in construction zone"
                        }
                    ]
                }),
            }]))
        } else {
            // Default helpful response
            Ok(AgentAction::Respond(
                "I can assist with EHS inspections. I support:\n\
                 - Creating and scheduling inspections\n\
                 - Conducting risk assessments\n\
                 - Documenting findings and hazards\n\
                 - Generating inspection reports\n\
                 - Tracking corrective actions\n\n\
                 What inspection activity would you like to perform?".to_string()
            ))
        }
    }

    async fn act(&mut self, action: &AgentAction) -> Result<Vec<Observation>, AgentError> {
        match action {
            AgentAction::UseTool(calls) => {
                self.state = AgentState::Acting {
                    tool_calls: calls.clone(),
                    pending: calls.len(),
                };

                let mut observations = Vec::new();

                for call in calls {
                    let result = match call.name.as_str() {
                        "inspection_create" => {
                            let inspection_id = Uuid::new_v4().to_string();
                            serde_json::json!({
                                "status": "created",
                                "inspection_id": inspection_id,
                                "facility_id": self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string()),
                                "type": self.execution_context.inspection_type.clone().unwrap_or_else(|| "Routine".to_string()),
                                "created_at": chrono::Utc::now().to_rfc3339(),
                                "status_reason": "Inspection record initialized"
                            })
                        }
                        "inspection_schedule" => {
                            serde_json::json!({
                                "status": "scheduled",
                                "inspection_id": call.input.get("inspection_id").unwrap_or(&serde_json::json!("")).to_string(),
                                "scheduled_start": "2026-03-15",
                                "scheduled_end": "2026-03-17",
                                "assigned_inspectors": [
                                    {"id": "INS-001", "name": "John Smith", "specialization": "Safety"},
                                    {"id": "INS-002", "name": "Maria Garcia", "specialization": "Environmental"}
                                ],
                                "team_lead": "INS-001"
                            })
                        }
                        "risk_assessment" => {
                            serde_json::json!({
                                "status": "completed",
                                "facility_id": self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string()),
                                "overall_risk_level": "Medium",
                                "high_risk_areas": 2,
                                "medium_risk_areas": 5,
                                "low_risk_areas": 8,
                                "primary_hazards": [
                                    "Chemical exposure",
                                    "Fall hazards",
                                    "Electrical hazards"
                                ],
                                "risk_score": 68,
                                "assessment_date": chrono::Utc::now().to_rfc3339()
                            })
                        }
                        "findings_document" => {
                            serde_json::json!({
                                "status": "documented",
                                "inspection_id": call.input.get("inspection_id").unwrap_or(&serde_json::json!("")).to_string(),
                                "findings_count": 12,
                                "critical_findings": 1,
                                "major_findings": 3,
                                "minor_findings": 8,
                                "documented_at": chrono::Utc::now().to_rfc3339(),
                                "report_available": true
                            })
                        }
                        _ => {
                            serde_json::json!({
                                "status": "error",
                                "message": format!("Unknown tool: {}", call.name)
                            })
                        }
                    };

                    observations.push(Observation {
                        tool_call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        result,
                        is_error: false,
                        duration_ms: 200,
                    });
                }

                // Update inspection state based on tools executed
                if let Some(first_call) = calls.first() {
                    match first_call.name.as_str() {
                        "inspection_schedule" => {
                            self.inspection_state = InspectionAgentState::SchedulingInspection {
                                proposed_dates: vec![
                                    "2026-03-15".to_string(),
                                    "2026-03-16".to_string(),
                                    "2026-03-17".to_string(),
                                ],
                                inspector_ids: vec![
                                    "INS-001".to_string(),
                                    "INS-002".to_string(),
                                ],
                            };
                        }
                        "findings_document" => {
                            self.inspection_state = InspectionAgentState::AwaitingCorrectiveAction {
                                inspection_id: Uuid::new_v4().to_string(),
                                deadline: "2026-04-05".to_string(),
                            };
                        }
                        _ => {}
                    }
                }

                Ok(observations)
            }
            AgentAction::Respond(_) => {
                self.state = AgentState::Idle;
                Ok(vec![])
            }
            _ => Ok(vec![]),
        }
    }

    async fn respond(&mut self, observations: &[Observation]) -> Result<AgentResponse, AgentError> {
        self.state = AgentState::Responding {
            draft: String::new(),
        };

        let content = if observations.is_empty() {
            "No inspection activities were needed.".to_string()
        } else {
            let mut response = String::from("Inspection activities completed. Summary:\n\n");

            for obs in observations {
                response.push_str(&format!("**{}**: ", obs.tool_name));

                if let Some(status) = obs.result.get("status").and_then(|s| s.as_str()) {
                    response.push_str(&format!("{}", status));

                    if let Some(reason) = obs.result.get("status_reason").and_then(|r| r.as_str()) {
                        response.push_str(&format!(" - {}", reason));
                    }
                    response.push('\n');
                }

                if let Some(risk_level) = obs.result.get("overall_risk_level").and_then(|r| r.as_str()) {
                    response.push_str(&format!("Risk Level: {}\n", risk_level));
                }

                if let Some(findings) = obs.result.get("findings_count").and_then(|f| f.as_u64()) {
                    response.push_str(&format!("Total Findings: {}\n", findings));

                    if let Some(critical) = obs.result.get("critical_findings").and_then(|c| c.as_u64()) {
                        if critical > 0 {
                            response.push_str(&format!("  - Critical: {}\n", critical));
                        }
                    }
                    if let Some(major) = obs.result.get("major_findings").and_then(|m| m.as_u64()) {
                        if major > 0 {
                            response.push_str(&format!("  - Major: {}\n", major));
                        }
                    }
                }

                response.push('\n');
            }

            response.push_str("Next steps: Review findings, assign corrective actions, and schedule follow-up inspection if needed.");
            response
        };

        self.state = AgentState::Idle;

        Ok(AgentResponse {
            content,
            citations: vec![
                Citation { source: "OSHA 1910".to_string(), text: "Occupational Safety and Health Standards".to_string() },
                Citation { source: "EPA".to_string(), text: "Environmental Protection Standards".to_string() },
                Citation { source: "ANSI".to_string(), text: "Safety Standards".to_string() },
            ],
            suggested_actions: vec![
                "Schedule follow-up inspection".to_string(),
                "Assign corrective action tasks".to_string(),
                "Notify facility management".to_string(),
                "Generate inspection report".to_string(),
            ],
            token_usage: TokenUsage {
                input_tokens: 300,
                output_tokens: 220,
            },
        })
    }

    fn current_state(&self) -> &AgentState {
        &self.state
    }

    fn transition(&mut self, new_state: AgentState) {
        self.state = new_state;
    }
}
