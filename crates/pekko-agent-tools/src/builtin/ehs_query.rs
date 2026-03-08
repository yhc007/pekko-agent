use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolDefinition, ToolContext, ToolOutput, ToolError};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{info, warn};

/// EHS 데이터베이스 조회 Tool
/// PostgreSQL의 EHS 테이블에 대해 읽기 전용 쿼리를 실행합니다.
pub struct EhsQueryTool {
    pool: Arc<PgPool>,
}

impl EhsQueryTool {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[derive(Deserialize)]
struct QueryParams {
    /// 실행할 SQL 쿼리 (SELECT만 허용)
    sql: String,
    /// 최대 반환 행 수 (기본: 50)
    limit: Option<i64>,
}

/// 허용된 EHS 테이블 목록 (읽기 전용)
const ALLOWED_TABLES: &[&str] = &[
    "dangerousworkmanagement",
    "tbmmanagement",
    "accidentfreemanagement",
    "riskassessmentmanagement",
    "nearmissmanagement",
    "safetyeducationmanagement",
    "msdsmanagement",
    "safetyhealthcommittee",
    "employeeinfo",
    "worksitemanagement",
    "facilitymanagement",
    "chemicalmanagement",
    "protectiveequipmentmanagement",
    "healthcheckmanagement",
    "environmentalmanagement",
];

/// SQL 안전성 검증
fn validate_sql(sql: &str) -> Result<(), String> {
    let sql_upper = sql.to_uppercase().trim().to_string();

    // SELECT만 허용
    if !sql_upper.starts_with("SELECT") {
        return Err("SELECT 쿼리만 허용됩니다. INSERT, UPDATE, DELETE 등은 사용할 수 없습니다.".into());
    }

    // 위험한 키워드 차단
    let forbidden = ["INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE",
                     "TRUNCATE", "GRANT", "REVOKE", "EXEC", "EXECUTE", "--", ";"];
    // ; 는 쿼리 끝에만 허용
    let sql_no_trailing = sql.trim().trim_end_matches(';');
    if sql_no_trailing.contains(';') {
        return Err("다중 쿼리(;)는 허용되지 않습니다.".into());
    }
    for kw in &forbidden[..forbidden.len() - 1] {
        // Check as a word boundary (not part of column names)
        if sql_upper.contains(&format!(" {kw} ")) || sql_upper.starts_with(&format!("{kw} ")) {
            return Err(format!("{kw} 문은 허용되지 않습니다."));
        }
    }
    if sql.contains("--") {
        return Err("SQL 주석(--)은 허용되지 않습니다.".into());
    }

    Ok(())
}

#[async_trait]
impl Tool for EhsQueryTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ehs_query".to_string(),
            description: "EHS(환경안전보건) 데이터베이스에서 데이터를 조회합니다. \
                          SELECT 쿼리만 허용되며, 다음 테이블에 접근할 수 있습니다: \
                          dangerousworkmanagement(위험작업허가), tbmmanagement(TBM), \
                          accidentfreemanagement(무재해), riskassessmentmanagement(위험성평가), \
                          nearmissmanagement(아차사고), safetyeducationmanagement(안전교육), \
                          msdsmanagement(MSDS), safetyhealthcommittee(안전보건위원회), \
                          employeeinfo(직원정보)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "실행할 SELECT SQL 쿼리. 예: SELECT count(*) FROM dangerousworkmanagement"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "최대 반환 행 수 (기본: 50, 최대: 200)",
                        "default": 50
                    }
                },
                "required": ["sql"]
            }),
            required_permissions: vec![],
            timeout_ms: 10_000,
            idempotent: true,
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let params: QueryParams = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(format!("파라미터 오류: {e}")))?;

        // SQL 안전성 검증
        if let Err(e) = validate_sql(&params.sql) {
            return Ok(ToolOutput::error(e));
        }

        let limit = params.limit.unwrap_or(50).min(200);

        // LIMIT이 없으면 자동으로 추가
        let sql = {
            let upper = params.sql.to_uppercase();
            if !upper.contains("LIMIT") {
                format!("{} LIMIT {}", params.sql.trim().trim_end_matches(';'), limit)
            } else {
                params.sql.trim().trim_end_matches(';').to_string()
            }
        };

        info!(sql = %sql, "EHS DB 쿼리 실행");

        // 쿼리 실행
        match sqlx::query(&sql).fetch_all(self.pool.as_ref()).await {
            Ok(rows) => {
                use sqlx::Row;
                use sqlx::Column;

                // 결과를 JSON 배열로 변환
                let mut result = Vec::new();
                for row in &rows {
                    let mut obj = serde_json::Map::new();
                    for (i, col) in row.columns().iter().enumerate() {
                        let name = col.name().to_string();
                        // Try to get as various types
                        let val: serde_json::Value = if let Ok(v) = row.try_get::<String, _>(i) {
                            serde_json::Value::String(v)
                        } else if let Ok(v) = row.try_get::<i64, _>(i) {
                            serde_json::Value::Number(v.into())
                        } else if let Ok(v) = row.try_get::<i32, _>(i) {
                            serde_json::Value::Number(v.into())
                        } else if let Ok(v) = row.try_get::<f64, _>(i) {
                            serde_json::json!(v)
                        } else if let Ok(v) = row.try_get::<bool, _>(i) {
                            serde_json::Value::Bool(v)
                        } else if let Ok(v) = row.try_get::<chrono::NaiveDateTime, _>(i) {
                            serde_json::Value::String(v.format("%Y-%m-%d %H:%M:%S").to_string())
                        } else if let Ok(v) = row.try_get::<chrono::NaiveDate, _>(i) {
                            serde_json::Value::String(v.format("%Y-%m-%d").to_string())
                        } else {
                            // null or unsupported type
                            serde_json::Value::Null
                        };
                        obj.insert(name, val);
                    }
                    result.push(serde_json::Value::Object(obj));
                }

                info!(row_count = result.len(), "EHS DB 쿼리 완료");

                Ok(ToolOutput::success(serde_json::json!({
                    "row_count": result.len(),
                    "rows": result
                })))
            }
            Err(e) => {
                warn!(error = %e, "EHS DB 쿼리 실패");
                Ok(ToolOutput::error(format!("쿼리 실행 오류: {e}")))
            }
        }
    }
}
