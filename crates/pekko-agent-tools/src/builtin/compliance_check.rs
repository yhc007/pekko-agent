use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolDefinition, ToolContext, ToolOutput, ToolError};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::info;

/// Tool for checking facility compliance records from the `compliance_records` table.
pub struct ComplianceCheckTool {
    pool: Arc<PgPool>,
}

impl ComplianceCheckTool {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Deserialize)]
pub struct ComplianceInput {
    pub regulation_id: String,
    pub facility_id: String,
    #[serde(default)]
    pub check_items: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ComplianceResult {
    pub regulation_id: String,
    pub facility_id: String,
    pub status: String,
    pub score: Option<f64>,
    pub findings: serde_json::Value,
    pub remediation: serde_json::Value,
    pub checked_at: Option<String>,
    pub record_count: i64,
}

#[async_trait]
impl Tool for ComplianceCheckTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "compliance_check".to_string(),
            description: "시설의 EHS 규정 준수 이력을 조회합니다. \
                          regulation_id와 facility_id로 최신 준수 기록을 반환합니다.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "regulation_id": {
                        "type": "string",
                        "description": "규정/법규 식별자 (예: ISO-14001, OSHA-1910, 산안법). 부분 일치 검색 지원."
                    },
                    "facility_id": {
                        "type": "string",
                        "description": "시설 ID"
                    },
                    "check_items": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "확인할 특정 항목 목록 (비어 있으면 전체 조회)",
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
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let params: ComplianceInput = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(e.to_string()))?;

        info!(
            regulation_id = %params.regulation_id,
            facility_id = %params.facility_id,
            tenant_id = %ctx.tenant_id,
            "compliance_check 실행"
        );

        let like_reg = format!("%{}%", params.regulation_id);

        // 총 건수 조회
        let count_row = sqlx::query_as::<_, (i64,)>(
            "SELECT COUNT(*) FROM compliance_records \
             WHERE regulation_id ILIKE $1 AND facility_id = $2",
        )
        .bind(&like_reg)
        .bind(&params.facility_id)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("DB 조회 실패: {e}")))?;

        let record_count = count_row.0;

        if record_count == 0 {
            return Ok(ToolOutput::success(serde_json::json!({
                "regulation_id": params.regulation_id,
                "facility_id": params.facility_id,
                "status": "no_record",
                "message": "해당 시설/규정에 대한 준수 이력이 없습니다.",
                "record_count": 0
            })));
        }

        // 최신 기록 조회
        use sqlx::Row;
        let row = sqlx::query(
            "SELECT regulation_id, facility_id, status, score, \
                    findings, remediation, checked_at::text \
             FROM compliance_records \
             WHERE regulation_id ILIKE $1 AND facility_id = $2 \
             ORDER BY checked_at DESC LIMIT 1",
        )
        .bind(&like_reg)
        .bind(&params.facility_id)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("DB 조회 실패: {e}")))?;

        let mut findings: serde_json::Value = row
            .try_get::<serde_json::Value, _>("findings")
            .unwrap_or(serde_json::Value::Null);

        // check_items가 지정된 경우 해당 항목만 필터링
        if !params.check_items.is_empty() {
            if let serde_json::Value::Array(arr) = &findings {
                let filtered: Vec<serde_json::Value> = arr
                    .iter()
                    .filter(|f| {
                        let item = f.get("item").and_then(|v| v.as_str()).unwrap_or("");
                        params.check_items.iter().any(|ci| {
                            item.to_lowercase().contains(&ci.to_lowercase())
                        })
                    })
                    .cloned()
                    .collect();
                findings = serde_json::Value::Array(filtered);
            }
        }

        let result = ComplianceResult {
            regulation_id: row.try_get("regulation_id").unwrap_or_default(),
            facility_id:   row.try_get("facility_id").unwrap_or_default(),
            status:        row.try_get("status").unwrap_or_default(),
            score:         row.try_get::<Option<f32>, _>("score").ok().flatten().map(|s| s as f64),
            findings,
            remediation:   row.try_get::<serde_json::Value, _>("remediation")
                               .unwrap_or(serde_json::Value::Null),
            checked_at:    row.try_get("checked_at").ok(),
            record_count,
        };

        info!(
            status = %result.status,
            score = ?result.score,
            record_count = record_count,
            "compliance_check 완료"
        );

        Ok(ToolOutput::success(serde_json::to_value(&result).unwrap()))
    }

    fn validate_input(&self, input: &serde_json::Value) -> Result<(), ToolError> {
        for field in &["regulation_id", "facility_id"] {
            let ok = input
                .get(field)
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if !ok {
                return Err(ToolError::ValidationFailed(
                    format!("{field}는 필수이며 비어 있을 수 없습니다."),
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
    fn test_validate_missing_regulation_id() {
        let input = serde_json::json!({ "facility_id": "FAC-001" });
        let ok = input.get("regulation_id").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        assert!(!ok);
    }

    #[test]
    fn test_validate_missing_facility_id() {
        let input = serde_json::json!({ "regulation_id": "ISO-14001" });
        let ok = input.get("facility_id").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        assert!(!ok);
    }

    #[test]
    fn test_validate_valid() {
        let input = serde_json::json!({
            "regulation_id": "ISO-14001",
            "facility_id": "FAC-001"
        });
        let reg_ok = input.get("regulation_id").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        let fac_ok = input.get("facility_id").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        assert!(reg_ok && fac_ok);
    }
}
