use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolDefinition, ToolContext, ToolOutput, ToolError};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::info;

/// Tool for searching EHS permits from the `permits` table.
pub struct PermitSearchTool {
    pool: Arc<PgPool>,
}

impl PermitSearchTool {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Deserialize)]
pub struct PermitSearchInput {
    pub query: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub facility_id: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PermitResult {
    pub permit_id: String,
    pub title: String,
    pub status: String,
    pub facility_id: String,
    pub industry: Option<String>,
    pub issued_date: Option<String>,
    pub expiry_date: Option<String>,
}

#[async_trait]
impl Tool for PermitSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "permit_search".to_string(),
            description: "EHS 허가(permit) 데이터베이스를 검색합니다. \
                          키워드·상태·시설 ID로 필터링할 수 있습니다.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "허가 제목 키워드 (부분 일치 검색)"
                    },
                    "status": {
                        "type": "string",
                        "enum": ["active", "expired", "pending", "revoked"],
                        "description": "허가 상태 필터"
                    },
                    "facility_id": {
                        "type": "string",
                        "description": "시설 ID 필터"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 20,
                        "description": "최대 반환 건수"
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
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let params: PermitSearchInput = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(e.to_string()))?;

        let limit = params.limit.unwrap_or(20).clamp(1, 100);
        let like_query = format!("%{}%", params.query);

        info!(
            query = %params.query,
            status = ?params.status,
            facility_id = ?params.facility_id,
            tenant_id = %ctx.tenant_id,
            "permit_search 실행"
        );

        // QueryBuilder로 선택적 필터 동적 조합
        let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
            "SELECT permit_id, title, facility_id, status, industry, \
             issued_date::text, expiry_date::text \
             FROM permits WHERE title ILIKE ",
        );
        qb.push_bind(&like_query);

        if let Some(ref status) = params.status {
            qb.push(" AND status = ");
            qb.push_bind(status);
        }
        if let Some(ref fid) = params.facility_id {
            qb.push(" AND facility_id = ");
            qb.push_bind(fid);
        }
        if !ctx.tenant_id.is_empty() && ctx.tenant_id != "default" {
            qb.push(" AND tenant_id = ");
            qb.push_bind(&ctx.tenant_id);
        }

        qb.push(" ORDER BY created_at DESC LIMIT ");
        qb.push_bind(limit);

        let rows = qb
            .build()
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DB 조회 실패: {e}")))?;

        use sqlx::Row;
        let results: Vec<PermitResult> = rows
            .iter()
            .map(|r| PermitResult {
                permit_id:   r.try_get("permit_id").unwrap_or_default(),
                title:       r.try_get("title").unwrap_or_default(),
                facility_id: r.try_get("facility_id").unwrap_or_default(),
                status:      r.try_get("status").unwrap_or_default(),
                industry:    r.try_get("industry").ok(),
                issued_date: r.try_get("issued_date").ok(),
                expiry_date: r.try_get("expiry_date").ok(),
            })
            .collect();

        info!(count = results.len(), "permit_search 완료");

        Ok(ToolOutput::success(serde_json::json!({
            "count": results.len(),
            "permits": results
        })))
    }

    fn validate_input(&self, input: &serde_json::Value) -> Result<(), ToolError> {
        let ok = input
            .get("query")
            .and_then(|q| q.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);

        if !ok {
            return Err(ToolError::ValidationFailed(
                "query는 필수이며 비어 있을 수 없습니다.".to_string(),
            ));
        }
        if let Some(limit) = input.get("limit").and_then(|l| l.as_i64()) {
            if !(1..=100).contains(&limit) {
                return Err(ToolError::ValidationFailed(
                    "limit은 1 이상 100 이하여야 합니다.".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 연결을 열지 않는 풀. `connect_lazy`는 URL만 파싱하고 실제 접속은
    /// 첫 쿼리까지 미루므로, DB를 건드리지 않는 검사에는 그대로 쓸 수 있다.
    /// 다만 풀이 유지 태스크를 spawn하므로 호출부는 Tokio 런타임 안이어야 한다
    /// (그래서 아래 테스트는 `#[tokio::test]`).
    fn offline_pool() -> Arc<PgPool> {
        Arc::new(
            PgPool::connect_lazy("postgres://localhost/unused")
                .expect("lazy pool should not need a server"),
        )
    }

    fn tool() -> PermitSearchTool {
        PermitSearchTool::new(offline_pool())
    }

    #[tokio::test]
    async fn definition_describes_the_tool() {
        let def = tool().definition();
        assert_eq!(def.name, "permit_search");
        assert!(def.idempotent);
        assert_eq!(def.input_schema["required"][0], "query");
    }

    #[tokio::test]
    async fn query_is_required_and_non_empty() {
        let t = tool();
        assert!(t.validate_input(&serde_json::json!({ "query": "" })).is_err());
        assert!(t.validate_input(&serde_json::json!({})).is_err());
        assert!(t.validate_input(&serde_json::json!({ "query": 42 })).is_err());
        assert!(t.validate_input(&serde_json::json!({ "query": "보일러" })).is_ok());
    }

    #[tokio::test]
    async fn limit_must_be_within_bounds() {
        let t = tool();
        for bad in [0, 101, -1] {
            assert!(
                t.validate_input(&serde_json::json!({ "query": "x", "limit": bad }))
                    .is_err(),
                "limit={bad} should be rejected"
            );
        }
        for good in [1, 50, 100] {
            assert!(
                t.validate_input(&serde_json::json!({ "query": "x", "limit": good }))
                    .is_ok(),
                "limit={good} should be accepted"
            );
        }
    }
}
