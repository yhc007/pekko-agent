mod inspection_agent;

use pekko_agent_core::{AgentInfo, AgentProfile, AgentStatus};
use axum::{routing::get, Router, Json};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{error, info, warn};

fn agent_info() -> AgentInfo {
    AgentInfo {
        agent_id:     "ehs-inspection-agent".to_string(),
        agent_type:   "ehs".to_string(),
        description:  "안전점검 및 위험성 평가 에이전트".to_string(),
        capabilities: vec!["ehs_query".to_string()],
        status:       AgentStatus::Available,
    }
}

fn agent_profile() -> AgentProfile {
    AgentProfile {
        // Inspection agent uses general EHS query only (no permit or compliance tools)
        tool_whitelist: Some(vec!["ehs_query".to_string()]),
        max_tokens_override: None,
    }
}

#[derive(Serialize)]
struct AuthRequest<'a> { api_key: &'a str }

#[derive(Deserialize)]
struct AuthResponse { token: String }

#[derive(Serialize)]
struct RegisterRequest { info: AgentInfo, profile: AgentProfile }

async fn register(gateway_url: &str, api_key: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
    let auth: AuthResponse = client
        .post(format!("{gateway_url}/api/auth/token"))
        .json(&AuthRequest { api_key })
        .send().await?.error_for_status()?.json().await?;
    client
        .post(format!("{gateway_url}/api/agents/register"))
        .bearer_auth(&auth.token)
        .json(&RegisterRequest { info: agent_info(), profile: agent_profile() })
        .send().await?.error_for_status()?;
    Ok(())
}

async fn register_with_retry(gateway_url: String, api_key: String) {
    let mut delay = Duration::from_secs(2);
    for attempt in 1..=10 {
        match register(&gateway_url, &api_key).await {
            Ok(()) => {
                info!(
                    agent_id = "ehs-inspection-agent",
                    tools = ?agent_profile().tool_whitelist,
                    "Registered with API Gateway"
                );
                return;
            }
            Err(e) => {
                warn!(attempt, error = %e, "Registration failed, retry in {delay:?}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(60));
            }
        }
    }
    error!("Registration failed after 10 attempts — running in degraded mode");
}

#[derive(Serialize)]
struct HealthResponse { status: &'static str, agent_id: &'static str, version: &'static str }

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "healthy", agent_id: "ehs-inspection-agent", version: env!("CARGO_PKG_VERSION") })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let _otel = pekko_agent_observability::tracing::init("ehs-inspection-agent");

    let gateway_url = std::env::var("GATEWAY_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let api_key     = std::env::var("AGENT_API_KEY").unwrap_or_else(|_| { warn!("AGENT_API_KEY not set"); String::new() });
    let port: u16   = std::env::var("EHS_INSPECTION_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8082);

    info!(agent_id = "ehs-inspection-agent", %gateway_url, "Starting EHS Inspection Agent");

    tokio::spawn(register_with_retry(gateway_url, api_key));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!(addr = %format!("0.0.0.0:{port}"), "EHS Inspection Agent listening");

    axum::serve(listener, Router::new().route("/health", get(health)))
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            info!("EHS Inspection Agent shutting down");
        })
        .await?;

    pekko_agent_observability::tracing::shutdown(_otel);
    Ok(())
}
