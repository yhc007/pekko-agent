use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolDefinition, ToolContext, ToolOutput, ToolError};
use serde::{Deserialize, Serialize};


/// Tool for searching environmental and safety permits
pub struct PermitSearchTool;

/// Input parameters for permit search
#[derive(Debug, Deserialize)]
pub struct PermitSearchInput {
    pub query: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub facility_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Result of a permit search
#[derive(Debug, Serialize, Clone)]
pub struct PermitResult {
    pub permit_id: String,
    pub title: String,
    pub status: String,
    pub facility: String,
    pub issued_date: String,
    pub expiry_date: String,
}

#[async_trait]
impl Tool for PermitSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "permit_search".to_string(),
            description: "Search for environmental and safety permits by keyword, status, or facility".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search keyword for permit title or description"
                    },
                    "status": {
                        "type": "string",
                        "enum": ["active", "expired", "pending", "revoked"],
                        "description": "Filter by permit status"
                    },
                    "facility_id": {
                        "type": "string",
                        "description": "Filter by facility identifier"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 10,
                        "description": "Maximum number of results to return"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            required_permissions: vec!["ehs.permit.read".to_string()],
            timeout_ms: 5000,
            idempotent: true,
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let params: PermitSearchInput = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(e.to_string()))?;

        let limit = params.limit.unwrap_or(10).min(100);

        // In production, this would query a real database
        // For now, return mock results based on search parameters
        let mut results = vec![
            PermitResult {
                permit_id: "PRM-2024-001".to_string(),
                title: format!("Environmental Permit - {}", params.query),
                status: params.status.clone().unwrap_or_else(|| "active".to_string()),
                facility: params.facility_id.clone().unwrap_or_else(|| "FAC-001".to_string()),
                issued_date: "2024-01-15".to_string(),
                expiry_date: "2025-01-15".to_string(),
            },
            PermitResult {
                permit_id: "PRM-2024-002".to_string(),
                title: format!("Safety Compliance - {} Review", params.query),
                status: "active".to_string(),
                facility: params.facility_id.clone().unwrap_or_else(|| "FAC-002".to_string()),
                issued_date: "2024-02-20".to_string(),
                expiry_date: "2026-02-20".to_string(),
            },
        ];

        results.truncate(limit);

        Ok(ToolOutput::success(serde_json::to_value(&results).unwrap()))
    }

    fn validate_input(&self, input: &serde_json::Value) -> Result<(), ToolError> {
        let query = input
            .get("query")
            .and_then(|q| q.as_str())
            .map(|s| !s.is_empty());

        if !query.unwrap_or(false) {
            return Err(ToolError::ValidationFailed(
                "query is required and must be non-empty".to_string(),
            ));
        }

        if let Some(limit) = input.get("limit").and_then(|l| l.as_i64()) {
            if limit < 1 || limit > 100 {
                return Err(ToolError::ValidationFailed(
                    "limit must be between 1 and 100".to_string(),
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permit_search_tool_definition() {
        let tool = PermitSearchTool;
        let def = tool.definition();
        assert_eq!(def.name, "permit_search");
        assert_eq!(def.required_permissions, vec!["ehs.permit.read".to_string()]);
        assert!(def.idempotent);
    }

    #[test]
    fn test_validate_input_empty_query() {
        let tool = PermitSearchTool;
        let invalid_input = serde_json::json!({ "query": "" });
        assert!(tool.validate_input(&invalid_input).is_err());
    }

    #[test]
    fn test_validate_input_missing_query() {
        let tool = PermitSearchTool;
        let invalid_input = serde_json::json!({ "status": "active" });
        assert!(tool.validate_input(&invalid_input).is_err());
    }

    #[test]
    fn test_validate_input_valid() {
        let tool = PermitSearchTool;
        let valid_input = serde_json::json!({ "query": "environmental" });
        assert!(tool.validate_input(&valid_input).is_ok());
    }

    #[test]
    fn test_validate_input_limit_too_high() {
        let tool = PermitSearchTool;
        let invalid_input = serde_json::json!({
            "query": "test",
            "limit": 101
        });
        assert!(tool.validate_input(&invalid_input).is_err());
    }

    #[test]
    fn test_validate_input_limit_too_low() {
        let tool = PermitSearchTool;
        let invalid_input = serde_json::json!({
            "query": "test",
            "limit": 0
        });
        assert!(tool.validate_input(&invalid_input).is_err());
    }

    #[test]
    fn test_validate_input_valid_limit() {
        let tool = PermitSearchTool;
        let valid_input = serde_json::json!({
            "query": "test",
            "limit": 50
        });
        assert!(tool.validate_input(&valid_input).is_ok());
    }
}
