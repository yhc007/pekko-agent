use async_trait::async_trait;
use pekko_agent_core::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// EHS Permit Agent - manages environmental permits
pub struct PermitAgentActor {
    agent_id: String,
    state: AgentState,
    permit_state: PermitAgentState,
    execution_context: ExecutionContext,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PermitAgentState {
    Idle,
    AnalyzingRequest {
        request_id: String,
        industry: String,
    },
    CheckingRegulations {
        regulations: Vec<String>,
    },
    GeneratingDocument {
        doc_type: String,
        facility_id: String,
    },
    ReviewingChecklist {
        items: Vec<ChecklistItem>,
    },
    AwaitingApproval {
        approver: String,
        permit_id: String,
    },
    Completed {
        permit_id: String,
        issued_date: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub item: String,
    pub checked: bool,
    pub notes: String,
}

#[derive(Clone, Debug)]
struct ExecutionContext {
    current_facility: Option<String>,
    industry_type: Option<String>,
    active_permits: Vec<String>,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            current_facility: None,
            industry_type: None,
            active_permits: vec![],
        }
    }
}

impl PermitAgentActor {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            state: AgentState::Idle,
            permit_state: PermitAgentState::Idle,
            execution_context: ExecutionContext::default(),
        }
    }

    pub fn permit_state(&self) -> &PermitAgentState {
        &self.permit_state
    }

    fn parse_facility_from_query(query: &str) -> Option<String> {
        // Extract facility ID patterns like FAC-001, FACILITY-123, etc.
        let parts: Vec<&str> = query.split_whitespace().collect();
        for part in parts {
            if part.contains("FAC-") || part.contains("FACILITY-") {
                return Some(part.to_string());
            }
        }
        None
    }

    fn parse_industry_from_query(query: &str) -> Option<String> {
        let query_lower = query.to_lowercase();
        if query_lower.contains("manufacturing") {
            Some("Manufacturing".to_string())
        } else if query_lower.contains("chemical") {
            Some("Chemical".to_string())
        } else if query_lower.contains("pharma") || query_lower.contains("pharmaceutical") {
            Some("Pharmaceutical".to_string())
        } else if query_lower.contains("oil") || query_lower.contains("gas") {
            Some("Oil & Gas".to_string())
        } else {
            None
        }
    }
}

#[async_trait]
impl AgentActor for PermitAgentActor {
    fn agent_id(&self) -> &str {
        &self.agent_id
    }

    fn available_tools(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "permit_search".to_string(),
                description: "Search for existing permits by facility or permit type".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "facility_id": {"type": "string"},
                        "permit_type": {"type": "string"}
                    },
                    "required": ["facility_id"]
                }),
                required_permissions: vec!["ehs.permit.read".to_string()],
                timeout_ms: 5000,
                idempotent: true,
            },
            ToolDefinition {
                name: "document_generate".to_string(),
                description: "Generate permit documents from templates".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "template": {"type": "string"},
                        "facility_id": {"type": "string"},
                        "industry": {"type": "string"}
                    },
                    "required": ["template", "facility_id"]
                }),
                required_permissions: vec!["ehs.permit.write".to_string()],
                timeout_ms: 15000,
                idempotent: false,
            },
            ToolDefinition {
                name: "compliance_check".to_string(),
                description: "Verify regulatory compliance for a facility".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "facility_id": {"type": "string"},
                        "regulation_id": {"type": "string"}
                    },
                    "required": ["facility_id"]
                }),
                required_permissions: vec!["ehs.compliance.read".to_string()],
                timeout_ms: 10000,
                idempotent: true,
            },
            ToolDefinition {
                name: "approval_request".to_string(),
                description: "Request approval for a permit".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "permit_id": {"type": "string"},
                        "approver_role": {"type": "string"}
                    },
                    "required": ["permit_id"]
                }),
                required_permissions: vec!["ehs.permit.approve".to_string()],
                timeout_ms: 5000,
                idempotent: false,
            },
        ]
    }

    fn system_prompt(&self) -> String {
        "You are an EHS Permit Agent specializing in environmental permit management and regulatory compliance. \
         You help users search for existing permits, verify compliance with regulations, generate permit documents, \
         and manage the permit approval workflow. Always check regulatory requirements before issuing any permits. \
         Be thorough in compliance checks and ensure all necessary documentation is in place.".to_string()
    }

    fn max_iterations(&self) -> u32 {
        8
    }

    async fn reason(&mut self, query: &UserQuery) -> Result<AgentAction, AgentError> {
        self.state = AgentState::Reasoning {
            query: query.content.clone(),
            iteration: 0,
            thought_chain: vec![
                "Analyzing permit request...".to_string(),
            ],
        };

        // Extract facility and industry information from query
        if let Some(facility) = Self::parse_facility_from_query(&query.content) {
            self.execution_context.current_facility = Some(facility.clone());
        }

        if let Some(industry) = Self::parse_industry_from_query(&query.content) {
            self.execution_context.industry_type = Some(industry.clone());
        }

        let content_lower = query.content.to_lowercase();

        if content_lower.contains("search") || content_lower.contains("find") {
            // Search for existing permits
            let facility = self.execution_context.current_facility.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());
            self.permit_state = PermitAgentState::AnalyzingRequest {
                request_id: Uuid::new_v4().to_string(),
                industry: self.execution_context.industry_type.clone().unwrap_or_else(|| "General".to_string()),
            };

            Ok(AgentAction::UseTool(vec![ToolCall {
                id: Uuid::new_v4().to_string(),
                name: "permit_search".to_string(),
                input: serde_json::json!({
                    "facility_id": facility,
                    "permit_type": "all"
                }),
            }]))
        } else if content_lower.contains("compliance") || content_lower.contains("check") {
            // Check compliance
            let facility = self.execution_context.current_facility.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());
            self.permit_state = PermitAgentState::CheckingRegulations {
                regulations: vec![
                    "EPA-40-CFR".to_string(),
                    "OSHA-29-CFR".to_string(),
                    "State-Environmental".to_string(),
                ],
            };

            Ok(AgentAction::UseTool(vec![ToolCall {
                id: Uuid::new_v4().to_string(),
                name: "compliance_check".to_string(),
                input: serde_json::json!({
                    "facility_id": facility,
                    "regulation_id": "EPA-40-CFR"
                }),
            }]))
        } else if content_lower.contains("generate") || content_lower.contains("create") {
            // Generate permit document
            let facility = self.execution_context.current_facility.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());
            self.permit_state = PermitAgentState::GeneratingDocument {
                doc_type: "Permit Application".to_string(),
                facility_id: facility.clone(),
            };

            Ok(AgentAction::UseTool(vec![ToolCall {
                id: Uuid::new_v4().to_string(),
                name: "document_generate".to_string(),
                input: serde_json::json!({
                    "template": "permit_application",
                    "facility_id": facility,
                    "industry": self.execution_context.industry_type.clone().unwrap_or_else(|| "General".to_string())
                }),
            }]))
        } else {
            // Default helpful response
            Ok(AgentAction::Respond(
                "I can help you with permit management. I support:\n\
                 - Searching for existing permits\n\
                 - Checking regulatory compliance\n\
                 - Generating permit documents\n\
                 - Managing approval workflows\n\n\
                 What would you like to do?".to_string()
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
                        "permit_search" => {
                            serde_json::json!({
                                "status": "success",
                                "permits_found": 3,
                                "permits": [
                                    {
                                        "permit_id": "PERMIT-2024-001",
                                        "type": "Air Discharge",
                                        "status": "Active",
                                        "expiry": "2025-12-31"
                                    },
                                    {
                                        "permit_id": "PERMIT-2024-002",
                                        "type": "Wastewater",
                                        "status": "Active",
                                        "expiry": "2026-06-30"
                                    },
                                    {
                                        "permit_id": "PERMIT-2023-001",
                                        "type": "Hazardous Waste",
                                        "status": "Expired",
                                        "expiry": "2023-12-31"
                                    }
                                ]
                            })
                        }
                        "compliance_check" => {
                            serde_json::json!({
                                "status": "success",
                                "facility_id": self.execution_context.current_facility.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string()),
                                "compliance_score": 92,
                                "violations": 1,
                                "critical_items": 0,
                                "last_inspection": "2024-09-15",
                                "next_scheduled": "2025-03-15"
                            })
                        }
                        "document_generate" => {
                            serde_json::json!({
                                "status": "success",
                                "document_id": Uuid::new_v4().to_string(),
                                "document_type": "Permit Application",
                                "template_version": "2.1",
                                "pages": 12,
                                "generated_at": chrono::Utc::now().to_rfc3339()
                            })
                        }
                        "approval_request" => {
                            serde_json::json!({
                                "status": "pending",
                                "approval_id": Uuid::new_v4().to_string(),
                                "permit_id": call.input.get("permit_id").unwrap_or(&serde_json::json!("")).as_str().unwrap_or(""),
                                "approver_role": "Facility Manager",
                                "created_at": chrono::Utc::now().to_rfc3339()
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
                        duration_ms: 150,
                    });
                }

                // Update permit state based on tools used
                if let Some(first_call) = calls.first() {
                    match first_call.name.as_str() {
                        "document_generate" => {
                            self.permit_state = PermitAgentState::ReviewingChecklist {
                                items: vec![
                                    ChecklistItem {
                                        item: "Facility information complete".to_string(),
                                        checked: true,
                                        notes: "All facility details provided".to_string(),
                                    },
                                    ChecklistItem {
                                        item: "Environmental impact assessment".to_string(),
                                        checked: true,
                                        notes: "EIA completed and approved".to_string(),
                                    },
                                    ChecklistItem {
                                        item: "Public notification".to_string(),
                                        checked: false,
                                        notes: "Pending public comment period".to_string(),
                                    },
                                ],
                            };
                        }
                        "approval_request" => {
                            if let Some(permit_id) = calls.first().and_then(|c| c.input.get("permit_id")).and_then(|v| v.as_str()) {
                                self.permit_state = PermitAgentState::AwaitingApproval {
                                    approver: "Regional EHS Manager".to_string(),
                                    permit_id: permit_id.to_string(),
                                };
                            }
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
            "No tools were needed for this request.".to_string()
        } else {
            let mut response = String::from("I have processed your permit request. Here's a summary:\n\n");

            for obs in observations {
                response.push_str(&format!("**{}**: ", obs.tool_name));

                if let Some(status) = obs.result.get("status").and_then(|s| s.as_str()) {
                    response.push_str(&format!("Status - {}\n", status));
                }

                if let Some(permits) = obs.result.get("permits_found").and_then(|p| p.as_u64()) {
                    response.push_str(&format!("Found {} permits\n", permits));
                }

                if let Some(score) = obs.result.get("compliance_score").and_then(|s| s.as_u64()) {
                    response.push_str(&format!("Compliance Score: {}%\n", score));
                }

                response.push('\n');
            }

            response.push_str("Please review the detailed results above. Do you need any additional assistance with permits, compliance, or documentation?");
            response
        };

        self.state = AgentState::Idle;

        Ok(AgentResponse {
            content,
            citations: vec![
                Citation { source: "EPA Title 40 CFR".to_string(), text: "Air and Radiation".to_string() },
                Citation { source: "OSHA 29 CFR".to_string(), text: "Safety and Health Regulations".to_string() },
            ],
            suggested_actions: vec![
                "Review expired permits".to_string(),
                "Schedule compliance audit".to_string(),
                "Request permit renewal".to_string(),
            ],
            token_usage: TokenUsage {
                input_tokens: 250,
                output_tokens: 180,
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
