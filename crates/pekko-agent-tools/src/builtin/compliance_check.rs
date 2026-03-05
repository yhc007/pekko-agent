use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolDefinition, ToolContext, ToolOutput, ToolError};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Tool for checking facility compliance against regulations
pub struct ComplianceCheckTool;

/// Input parameters for compliance check
#[derive(Debug, Deserialize)]
pub struct ComplianceInput {
    pub regulation_id: String,
    pub facility_id: String,
    #[serde(default)]
    pub check_items: Vec<String>,
}

/// Result of a compliance check
#[derive(Debug, Serialize, Clone)]
pub struct ComplianceResult {
    pub regulation_id: String,
    pub facility_id: String,
    pub status: String,
    pub score: f64,
    pub findings: Vec<ComplianceFinding>,
}

/// Individual compliance finding
#[derive(Debug, Serialize, Clone)]
pub struct ComplianceFinding {
    pub item: String,
    pub status: String,
    pub severity: String,
    pub recommendation: String,
}

#[async_trait]
impl Tool for ComplianceCheckTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "compliance_check".to_string(),
            description: "Check facility compliance against specific EHS (Environmental, Health & Safety) regulations".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "regulation_id": {
                        "type": "string",
                        "description": "Identifier of the regulation to check against (e.g., ISO-14001, OSHA-1910)"
                    },
                    "facility_id": {
                        "type": "string",
                        "description": "Facility identifier to check"
                    },
                    "check_items": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Specific items to check (optional, if empty all items checked)",
                        "default": []
                    }
                },
                "required": ["regulation_id", "facility_id"],
                "additionalProperties": false
            }),
            required_permissions: vec!["ehs.compliance.read".to_string()],
            timeout_ms: 10000,
            idempotent: true,
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let params: ComplianceInput = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(e.to_string()))?;

        info!(
            regulation_id = %params.regulation_id,
            facility_id = %params.facility_id,
            "Starting compliance check"
        );

        // In production, this would perform actual compliance validation
        // For now, return mock results
        let findings = if params.check_items.is_empty() {
            vec![
                ComplianceFinding {
                    item: "waste_disposal".to_string(),
                    status: "pass".to_string(),
                    severity: "info".to_string(),
                    recommendation: "Continue current waste management practices".to_string(),
                },
                ComplianceFinding {
                    item: "emergency_procedures".to_string(),
                    status: "pass".to_string(),
                    severity: "info".to_string(),
                    recommendation: "Update emergency response plan annually".to_string(),
                },
                ComplianceFinding {
                    item: "hazmat_storage".to_string(),
                    status: "pass".to_string(),
                    severity: "info".to_string(),
                    recommendation: "Inspect storage containers quarterly".to_string(),
                },
            ]
        } else {
            params.check_items.iter().map(|item| {
                ComplianceFinding {
                    item: item.clone(),
                    status: "pass".to_string(),
                    severity: "info".to_string(),
                    recommendation: format!("Continue monitoring {}", item),
                }
            }).collect()
        };

        let result = ComplianceResult {
            regulation_id: params.regulation_id,
            facility_id: params.facility_id,
            status: "compliant".to_string(),
            score: 0.95,
            findings,
        };

        info!("Compliance check completed successfully");
        Ok(ToolOutput::success(serde_json::to_value(&result).unwrap()))
    }

    fn validate_input(&self, input: &serde_json::Value) -> Result<(), ToolError> {
        let regulation_id = input
            .get("regulation_id")
            .and_then(|id| id.as_str())
            .map(|s| !s.is_empty());

        if !regulation_id.unwrap_or(false) {
            return Err(ToolError::ValidationFailed(
                "regulation_id is required and must be non-empty".to_string(),
            ));
        }

        let facility_id = input
            .get("facility_id")
            .and_then(|id| id.as_str())
            .map(|s| !s.is_empty());

        if !facility_id.unwrap_or(false) {
            return Err(ToolError::ValidationFailed(
                "facility_id is required and must be non-empty".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compliance_check_tool_definition() {
        let tool = ComplianceCheckTool;
        let def = tool.definition();
        assert_eq!(def.name, "compliance_check");
        assert_eq!(def.required_permissions, vec!["ehs.compliance.read".to_string()]);
        assert!(def.idempotent);
    }

    #[test]
    fn test_validate_input_missing_regulation_id() {
        let tool = ComplianceCheckTool;
        let invalid_input = serde_json::json!({ "facility_id": "FAC-001" });
        assert!(tool.validate_input(&invalid_input).is_err());
    }

    #[test]
    fn test_validate_input_missing_facility_id() {
        let tool = ComplianceCheckTool;
        let invalid_input = serde_json::json!({ "regulation_id": "ISO-14001" });
        assert!(tool.validate_input(&invalid_input).is_err());
    }

    #[test]
    fn test_validate_input_empty_regulation_id() {
        let tool = ComplianceCheckTool;
        let invalid_input = serde_json::json!({
            "regulation_id": "",
            "facility_id": "FAC-001"
        });
        assert!(tool.validate_input(&invalid_input).is_err());
    }

    #[test]
    fn test_validate_input_empty_facility_id() {
        let tool = ComplianceCheckTool;
        let invalid_input = serde_json::json!({
            "regulation_id": "ISO-14001",
            "facility_id": ""
        });
        assert!(tool.validate_input(&invalid_input).is_err());
    }

    #[test]
    fn test_validate_input_valid() {
        let tool = ComplianceCheckTool;
        let valid_input = serde_json::json!({
            "regulation_id": "ISO-14001",
            "facility_id": "FAC-001"
        });
        assert!(tool.validate_input(&valid_input).is_ok());
    }

    #[test]
    fn test_validate_input_with_check_items() {
        let tool = ComplianceCheckTool;
        let valid_input = serde_json::json!({
            "regulation_id": "ISO-14001",
            "facility_id": "FAC-001",
            "check_items": ["waste_disposal", "emissions"]
        });
        assert!(tool.validate_input(&valid_input).is_ok());
    }
}
