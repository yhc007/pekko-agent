use gloo_net::http::Request;
use uuid::Uuid;

use crate::types::*;

const DEFAULT_BASE_URL: &str = "http://localhost:8080";

fn base_url() -> String {
    // Dynamically determine API URL based on current location
    if let Some(window) = web_sys::window() {
        if let Ok(href) = window.location().href() {
            // If accessing via agenticai.coreon.build, use api subdomain
            if href.contains("agenticai.coreon.build") {
                return "https://agenticai-api.coreon.build".to_string();
            }
        }
    }
    // Fallback to localhost for development
    DEFAULT_BASE_URL.to_string()
}

/// GET /api/health
pub async fn fetch_health() -> Result<HealthResponse, String> {
    let url = format!("{}/api/health", base_url());
    let resp = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("네트워크 오류: {e}"))?;

    if resp.ok() {
        resp.json::<HealthResponse>()
            .await
            .map_err(|e| format!("파싱 오류: {e}"))
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

/// GET /api/agents
pub async fn fetch_agents() -> Result<Vec<AgentInfo>, String> {
    let url = format!("{}/api/agents", base_url());
    let resp = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("네트워크 오류: {e}"))?;

    if resp.ok() {
        resp.json::<Vec<AgentInfo>>()
            .await
            .map_err(|e| format!("파싱 오류: {e}"))
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// POST /api/agents/:agent_id/query
pub async fn send_query(
    agent_id: &str,
    content: &str,
    session_id: Option<Uuid>,
) -> Result<QueryResponse, String> {
    let url = format!("{}/api/agents/{}/query", base_url(), agent_id);
    let body = QueryRequest {
        content: content.to_string(),
        session_id,
        tenant_id: "default".to_string(),
        user_id: "web-user".to_string(),
    };

    let resp = Request::post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| format!("직렬화 오류: {e}"))?
        .send()
        .await
        .map_err(|e| format!("네트워크 오류: {e}"))?;

    if resp.ok() {
        resp.json::<QueryResponse>()
            .await
            .map_err(|e| format!("파싱 오류: {e}"))
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

/// GET /api/sessions/:session_id/history
pub async fn fetch_history(session_id: Uuid) -> Result<Vec<HistoryMessage>, String> {
    let url = format!("{}/api/sessions/{}/history", base_url(), session_id);
    let resp = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("네트워크 오류: {e}"))?;

    if resp.ok() {
        resp.json::<Vec<HistoryMessage>>()
            .await
            .map_err(|e| format!("파싱 오류: {e}"))
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// GET /api/tools
pub async fn fetch_tools() -> Result<Vec<ToolInfo>, String> {
    let url = format!("{}/api/tools", base_url());
    let resp = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("네트워크 오류: {e}"))?;

    if resp.ok() {
        resp.json::<Vec<ToolInfo>>()
            .await
            .map_err(|e| format!("파싱 오류: {e}"))
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}
