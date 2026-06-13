use gloo_net::http::Request;
use uuid::Uuid;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::types::*;

const DEFAULT_BASE_URL: &str = "http://localhost:8080";

fn auth_header(token: &str) -> String {
    format!("Bearer {token}")
}

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

/// POST /api/auth/token — exchange API key for JWT
pub async fn issue_token(api_key: &str) -> Result<AuthResponse, String> {
    let url = format!("{}/api/auth/token", base_url());
    let body = AuthRequest { api_key: api_key.to_string() };
    let resp = Request::post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| format!("직렬화 오류: {e}"))?
        .send()
        .await
        .map_err(|e| format!("네트워크 오류: {e}"))?;
    if resp.ok() {
        resp.json::<AuthResponse>().await.map_err(|e| format!("파싱 오류: {e}"))
    } else {
        let err = resp.json::<ErrorResponse>().await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

/// POST /api/agents/collaborate — fan-out to all agents, returns synthesised result
pub async fn collaborate(
    token: &str,
    content: &str,
    agent_ids: Option<Vec<String>>,
    session_id: Option<Uuid>,
) -> Result<CollaborationResult, String> {
    let url = format!("{}/api/agents/collaborate", base_url());
    let body = CollaborateApiRequest { content: content.to_string(), agent_ids, session_id };
    let resp = Request::post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", &auth_header(token))
        .json(&body)
        .map_err(|e| format!("직렬화 오류: {e}"))?
        .send()
        .await
        .map_err(|e| format!("네트워크 오류: {e}"))?;
    if resp.ok() {
        resp.json::<CollaborationResult>().await.map_err(|e| format!("파싱 오류: {e}"))
    } else {
        let err = resp.json::<ErrorResponse>().await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
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

/// POST /api/agents/:agent_id/query/stream  — Fetch-based SSE streaming.
///
/// Calls `on_event` for each SSE event received.
/// Returns Ok(()) when the stream ends cleanly, or Err(message) on network/parse errors.
pub async fn stream_query<F: FnMut(StreamEvent)>(
    agent_id: &str,
    content: &str,
    session_id: Option<Uuid>,
    token: Option<String>,
    mut on_event: F,
) -> Result<(), String> {
    let url = format!("{}/api/agents/{}/query/stream", base_url(), agent_id);
    let body = QueryRequest {
        content: content.to_string(),
        session_id,
        tenant_id: "default".to_string(),
        user_id: "web-user".to_string(),
    };
    let body_json = serde_json::to_string(&body).map_err(|e| format!("직렬화 오류: {e}"))?;

    // Build Fetch request with POST + JSON body
    let headers = web_sys::Headers::new()
        .map_err(|e| format!("Headers 오류: {:?}", e))?;
    headers.append("Content-Type", "application/json")
        .map_err(|e| format!("Header 설정 오류: {:?}", e))?;
    if let Some(t) = token {
        headers.append("Authorization", &auth_header(&t))
            .map_err(|e| format!("Header 설정 오류: {:?}", e))?;
    }

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(web_sys::RequestMode::Cors);
    opts.set_headers(&headers);
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body_json));

    let request = web_sys::Request::new_with_str_and_init(&url, &opts)
        .map_err(|e| format!("Request 오류: {:?}", e))?;

    let window = web_sys::window().ok_or("window 없음")?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Fetch 오류: {:?}", e))?;

    let resp: web_sys::Response = resp_value.dyn_into()
        .map_err(|_| "Response 변환 실패".to_string())?;

    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body_stream = resp.body().ok_or("Response body 없음")?;
    let reader_js = body_stream.get_reader();
    let reader: web_sys::ReadableStreamDefaultReader = reader_js.dyn_into()
        .map_err(|_| "ReadableStreamDefaultReader 변환 실패".to_string())?;

    let mut buffer = String::new();

    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|e| format!("스트림 읽기 오류: {:?}", e))?;

        let done = js_sys::Reflect::get(&chunk, &wasm_bindgen::JsValue::from_str("done"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if done {
            break;
        }

        let value = js_sys::Reflect::get(&chunk, &wasm_bindgen::JsValue::from_str("value"))
            .map_err(|_| "chunk.value 없음".to_string())?;

        let uint8array: js_sys::Uint8Array = value.dyn_into()
            .map_err(|_| "Uint8Array 변환 실패".to_string())?;

        buffer.push_str(&String::from_utf8_lossy(&uint8array.to_vec()));

        // Parse all complete SSE events from the buffer ("data: ...\n\n")
        while let Some(idx) = buffer.find("\n\n") {
            let block = buffer[..idx].to_string();
            buffer = buffer[idx + 2..].to_string();

            for line in block.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(event) = serde_json::from_str::<StreamEvent>(data) {
                        on_event(event);
                    }
                }
            }
        }
    }

    Ok(())
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
