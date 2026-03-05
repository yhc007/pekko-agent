use async_trait::async_trait;
use pekko_agent_core::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// EHS Compliance Agent - manages regulatory compliance verification and gap analysis
pub struct ComplianceAgentActor {
    agent_id: String,
    state: AgentState,
    compliance_state: ComplianceAgentState,
    execution_context: ExecutionContext,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ComplianceAgentState {
    Idle,
    IdentifyingRequirements {
        facility_id: String,
        regulations: Vec<String>,
    },
    CheckingCompliance {
        audit_id: String,
        check_count: usize,
    },
    AnalyzingGaps {
        gap_count: usize,
        severity_breakdown: HashMap<String, usize>,
    },
    DevelopingRemediationPlan {
        plan_id: String,
        action_items: usize,
    },
    MonitoringComplianceStatus {
        facility_id: String,
        conformance_percentage: f64,
    },
    GeneratingComplianceReport {
        report_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComplianceGap {
    pub gap_id: String,
    pub regulation: String,
    pub requirement: String,
    pub current_status: String,
    pub severity: String,
    pub remediation_deadline: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemediationAction {
    pub action_id: String,
    pub gap_id: String,
    pub action_description: String,
    pub owner: String,
    pub due_date: String,
    pub status: String,
}

#[derive(Clone, Debug)]
struct ExecutionContext {
    facility_id: Option<String>,
    applicable_regulations: Vec<String>,
    identified_gaps: Vec<ComplianceGap>,
    remediation_actions: Vec<RemediationAction>,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            facility_id: None,
            applicable_regulations: vec![],
            identified_gaps: vec![],
            remediation_actions: vec![],
        }
    }
}

impl ComplianceAgentActor {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            state: AgentState::Idle,
            compliance_state: ComplianceAgentState::Idle,
            execution_context: ExecutionContext::default(),
        }
    }

    pub fn compliance_state(&self) -> &ComplianceAgentState {
        &self.compliance_state
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

    fn determine_applicable_regulations(query: &str) -> Vec<String> {
        let mut regulations = vec![
            "EPA Title 40 CFR".to_string(),
            "OSHA 29 CFR 1910".to_string(),
        ];

        let query_lower = query.to_lowercase();
        if query_lower.contains("air") || query_lower.contains("emission") {
            regulations.push("EPA Clean Air Act".to_string());
            regulations.push("State Air Quality Standards".to_string());
        }
        if query_lower.contains("water") || query_lower.contains("wastewater") {
            regulations.push("EPA Clean Water Act".to_string());
            regulations.push("State Water Quality Standards".to_string());
        }
        if query_lower.contains("hazard") || query_lower.contains("waste") {
            regulations.push("EPA RCRA Hazardous Waste".to_string());
            regulations.push("DOT Hazmat Transportation".to_string());
        }
        if query_lower.contains("chemical") {
            regulations.push("EPA EPCRA Chemical Reporting".to_string());
            regulations.push("OSHA Process Safety Management".to_string());
        }
        if query_lower.contains("noise") {
            regulations.push("OSHA Noise Exposure Standards".to_string());
        }

        regulations
    }
}

#[async_trait]
impl AgentActor for ComplianceAgentActor {
    fn agent_id(&self) -> &str {
        &self.agent_id
    }

    fn available_tools(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "compliance_check".to_string(),
                description: "Check facility compliance against specific regulations".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "facility_id": {"type": "string"},
                        "regulation_id": {"type": "string"},
                        "check_scope": {"type": "string"}
                    },
                    "required": ["facility_id"]
                }),
                required_permissions: vec!["ehs.compliance.read".to_string()],
                timeout_ms: 8000,
                idempotent: true,
            },
            ToolDefinition {
                name: "regulation_lookup".to_string(),
                description: "Look up specific regulatory requirements and latest updates".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "regulation_code": {"type": "string"},
                        "section": {"type": "string"}
                    },
                    "required": ["regulation_code"]
                }),
                required_permissions: vec!["ehs.regulations.read".to_string()],
                timeout_ms: 5000,
                idempotent: true,
            },
            ToolDefinition {
                name: "gap_analysis".to_string(),
                description: "Perform gap analysis to identify compliance gaps".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "facility_id": {"type": "string"},
                        "regulations": {"type": "array"},
                        "analysis_type": {"type": "string"}
                    },
                    "required": ["facility_id"]
                }),
                required_permissions: vec!["ehs.compliance.analyze".to_string()],
                timeout_ms: 15000,
                idempotent: true,
            },
            ToolDefinition {
                name: "remediation_plan".to_string(),
                description: "Create or update remediation plan for identified gaps".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "facility_id": {"type": "string"},
                        "gaps": {"type": "array"},
                        "priority": {"type": "string"}
                    },
                    "required": ["facility_id", "gaps"]
                }),
                required_permissions: vec!["ehs.compliance.write".to_string()],
                timeout_ms: 10000,
                idempotent: false,
            },
            ToolDefinition {
                name: "compliance_report".to_string(),
                description: "Generate comprehensive compliance status report".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "facility_id": {"type": "string"},
                        "report_type": {"type": "string"},
                        "include_metrics": {"type": "boolean"}
                    },
                    "required": ["facility_id"]
                }),
                required_permissions: vec!["ehs.compliance.read".to_string(), "ehs.report.write".to_string()],
                timeout_ms: 12000,
                idempotent: false,
            },
        ]
    }

    fn system_prompt(&self) -> String {
        "You are an EHS Compliance Agent specialized in environmental, health, and safety regulatory compliance. \
         Your expertise includes: identifying applicable regulations, checking facility compliance status, \
         analyzing compliance gaps, developing remediation plans, and generating compliance reports. \
         You stay current with regulatory updates, assess compliance risks, recommend corrective actions, \
         and help facilities maintain ongoing compliance. Prioritize gaps by severity and regulatory impact.".to_string()
    }

    fn max_iterations(&self) -> u32 {
        10
    }

    async fn reason(&mut self, query: &UserQuery) -> Result<AgentAction, AgentError> {
        self.state = AgentState::Reasoning {
            query: query.content.clone(),
            iteration: 0,
            thought_chain: vec![
                "Analyzing compliance request...".to_string(),
                "Identifying applicable regulations...".to_string(),
            ],
        };

        // Extract facility information
        if let Some(facility) = Self::parse_facility_from_query(&query.content) {
            self.execution_context.facility_id = Some(facility);
        }

        // Determine applicable regulations
        self.execution_context.applicable_regulations = Self::determine_applicable_regulations(&query.content);

        let content_lower = query.content.to_lowercase();

        if content_lower.contains("check") || content_lower.contains("audit") {
            // Compliance check
            let facility = self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());

            self.compliance_state = ComplianceAgentState::CheckingCompliance {
                audit_id: Uuid::new_v4().to_string(),
                check_count: self.execution_context.applicable_regulations.len(),
            };

            Ok(AgentAction::UseTool(vec![ToolCall {
                id: Uuid::new_v4().to_string(),
                name: "compliance_check".to_string(),
                input: serde_json::json!({
                    "facility_id": facility,
                    "regulation_id": "EPA-40-CFR",
                    "check_scope": "comprehensive"
                }),
            }]))
        } else if content_lower.contains("gap") || content_lower.contains("deficiency") {
            // Gap analysis
            let facility = self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());

            self.compliance_state = ComplianceAgentState::AnalyzingGaps {
                gap_count: 0,
                severity_breakdown: HashMap::new(),
            };

            Ok(AgentAction::UseTool(vec![ToolCall {
                id: Uuid::new_v4().to_string(),
                name: "gap_analysis".to_string(),
                input: serde_json::json!({
                    "facility_id": facility,
                    "regulations": self.execution_context.applicable_regulations,
                    "analysis_type": "comprehensive"
                }),
            }]))
        } else if content_lower.contains("remediat") || content_lower.contains("corrective") {
            // Remediation planning
            let facility = self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());

            self.compliance_state = ComplianceAgentState::DevelopingRemediationPlan {
                plan_id: Uuid::new_v4().to_string(),
                action_items: 0,
            };

            Ok(AgentAction::UseTool(vec![
                ToolCall {
                    id: Uuid::new_v4().to_string(),
                    name: "gap_analysis".to_string(),
                    input: serde_json::json!({
                        "facility_id": facility,
                        "regulations": self.execution_context.applicable_regulations,
                        "analysis_type": "focus"
                    }),
                },
                ToolCall {
                    id: Uuid::new_v4().to_string(),
                    name: "remediation_plan".to_string(),
                    input: serde_json::json!({
                        "facility_id": facility,
                        "gaps": [],
                        "priority": "risk-based"
                    }),
                },
            ]))
        } else if content_lower.contains("report") || content_lower.contains("status") {
            // Compliance report
            let facility = self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string());

            self.compliance_state = ComplianceAgentState::GeneratingComplianceReport {
                report_id: Uuid::new_v4().to_string(),
            };

            Ok(AgentAction::UseTool(vec![ToolCall {
                id: Uuid::new_v4().to_string(),
                name: "compliance_report".to_string(),
                input: serde_json::json!({
                    "facility_id": facility,
                    "report_type": "comprehensive",
                    "include_metrics": true
                }),
            }]))
        } else {
            // Default helpful response
            Ok(AgentAction::Respond(
                "I can assist with EHS compliance management. I support:\n\
                 - Compliance audits and checks\n\
                 - Regulatory requirement lookups\n\
                 - Gap analysis and deficiency identification\n\
                 - Remediation plan development\n\
                 - Compliance status reporting\n\n\
                 What compliance activity would you like?".to_string()
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
                        "compliance_check" => {
                            serde_json::json!({
                                "status": "completed",
                                "facility_id": self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string()),
                                "regulation_checked": call.input.get("regulation_id").unwrap_or(&serde_json::json!("")).to_string(),
                                "compliance_status": "Mostly Compliant",
                                "conformance_percentage": 88,
                                "checks_passed": 18,
                                "checks_failed": 3,
                                "checks_pending": 1,
                                "last_updated": chrono::Utc::now().to_rfc3339()
                            })
                        }
                        "regulation_lookup" => {
                            serde_json::json!({
                                "status": "found",
                                "regulation_code": call.input.get("regulation_code").unwrap_or(&serde_json::json!("")).to_string(),
                                "title": "Environmental Protection Standards",
                                "effective_date": "2024-01-01",
                                "key_requirements": [
                                    "Emission monitoring and reporting",
                                    "Facility operating permits",
                                    "Environmental impact assessments"
                                ],
                                "penalties_for_violation": "Up to $50,000 per day"
                            })
                        }
                        "gap_analysis" => {
                            serde_json::json!({
                                "status": "completed",
                                "facility_id": self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string()),
                                "total_gaps_identified": 7,
                                "critical_gaps": 1,
                                "major_gaps": 3,
                                "minor_gaps": 3,
                                "gap_details": [
                                    {
                                        "gap_id": "GAP-001",
                                        "regulation": "EPA 40 CFR 61",
                                        "requirement": "NESHAP Compliance Documentation",
                                        "severity": "Critical"
                                    },
                                    {
                                        "gap_id": "GAP-002",
                                        "regulation": "OSHA 1910.1450",
                                        "requirement": "Chemical Hygiene Plan Updates",
                                        "severity": "Major"
                                    }
                                ],
                                "analysis_date": chrono::Utc::now().to_rfc3339()
                            })
                        }
                        "remediation_plan" => {
                            serde_json::json!({
                                "status": "created",
                                "plan_id": Uuid::new_v4().to_string(),
                                "facility_id": self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string()),
                                "action_items": 8,
                                "timeline": "90 days for critical items",
                                "estimated_cost": "$150,000",
                                "actions": [
                                    {
                                        "action_id": "ACT-001",
                                        "description": "Complete NESHAP compliance documentation",
                                        "owner": "Environmental Manager",
                                        "due_date": "2026-03-31",
                                        "status": "In Progress"
                                    }
                                ]
                            })
                        }
                        "compliance_report" => {
                            serde_json::json!({
                                "status": "generated",
                                "report_id": Uuid::new_v4().to_string(),
                                "facility_id": self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string()),
                                "report_date": chrono::Utc::now().to_rfc3339(),
                                "overall_compliance_score": 85,
                                "compliant_areas": 9,
                                "non_compliant_areas": 2,
                                "trend": "Improving",
                                "next_audit_date": "2026-06-05"
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
                        duration_ms: 250,
                    });
                }

                // Update compliance state based on tools executed
                if let Some(first_call) = calls.first() {
                    match first_call.name.as_str() {
                        "gap_analysis" => {
                            let mut severity_breakdown = HashMap::new();
                            severity_breakdown.insert("Critical".to_string(), 1);
                            severity_breakdown.insert("Major".to_string(), 3);
                            severity_breakdown.insert("Minor".to_string(), 3);

                            self.compliance_state = ComplianceAgentState::AnalyzingGaps {
                                gap_count: 7,
                                severity_breakdown,
                            };
                        }
                        "remediation_plan" => {
                            self.compliance_state = ComplianceAgentState::DevelopingRemediationPlan {
                                plan_id: Uuid::new_v4().to_string(),
                                action_items: 8,
                            };
                        }
                        "compliance_report" => {
                            self.compliance_state = ComplianceAgentState::MonitoringComplianceStatus {
                                facility_id: self.execution_context.facility_id.clone().unwrap_or_else(|| "FAC-DEFAULT".to_string()),
                                conformance_percentage: 85.0,
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
            "No compliance checks were needed.".to_string()
        } else {
            let mut response = String::from("Compliance assessment completed. Summary:\n\n");

            for obs in observations {
                response.push_str(&format!("**{}**\n", obs.tool_name));

                if let Some(status) = obs.result.get("status").and_then(|s| s.as_str()) {
                    response.push_str(&format!("Status: {}\n", status));
                }

                if let Some(percent) = obs.result.get("conformance_percentage").and_then(|p| p.as_u64()) {
                    response.push_str(&format!("Conformance: {}%\n", percent));
                } else if let Some(score) = obs.result.get("overall_compliance_score").and_then(|s| s.as_u64()) {
                    response.push_str(&format!("Compliance Score: {}\n", score));
                }

                if let Some(total) = obs.result.get("total_gaps_identified").and_then(|t| t.as_u64()) {
                    response.push_str(&format!("Gaps Identified: {}\n", total));

                    if let Some(critical) = obs.result.get("critical_gaps").and_then(|c| c.as_u64()) {
                        if critical > 0 {
                            response.push_str(&format!("  - Critical: {}\n", critical));
                        }
                    }
                    if let Some(major) = obs.result.get("major_gaps").and_then(|m| m.as_u64()) {
                        if major > 0 {
                            response.push_str(&format!("  - Major: {}\n", major));
                        }
                    }
                }

                if let Some(actions) = obs.result.get("action_items").and_then(|a| a.as_u64()) {
                    response.push_str(&format!("Remediation Actions Created: {}\n", actions));
                }

                response.push('\n');
            }

            response.push_str("Recommended next steps: Review identified gaps by severity, assign responsible parties, and track remediation progress.");
            response
        };

        self.state = AgentState::Idle;

        Ok(AgentResponse {
            content,
            citations: vec![
                Citation { source: "EPA Title 40 CFR".to_string(), text: "Environmental Protection".to_string() },
                Citation { source: "OSHA 29 CFR".to_string(), text: "Occupational Safety".to_string() },
                Citation { source: "State Regulations".to_string(), text: "Environmental Compliance Rules".to_string() },
                Citation { source: "ISO 14001".to_string(), text: "Environmental Management".to_string() },
            ],
            suggested_actions: vec![
                "Review critical compliance gaps".to_string(),
                "Assign gap remediation owners".to_string(),
                "Schedule compliance training".to_string(),
                "Establish monitoring schedule".to_string(),
                "Plan next audit cycle".to_string(),
            ],
            token_usage: TokenUsage {
                input_tokens: 350,
                output_tokens: 250,
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
