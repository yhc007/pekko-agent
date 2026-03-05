mod compliance_agent;

use compliance_agent::ComplianceAgentActor;
use pekko_agent_core::*;
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .with_level(true)
        .init();

    info!("Starting EHS Compliance Agent Service");

    let mut agent = ComplianceAgentActor::new("compliance-agent-001");

    info!(
        agent_id = agent.agent_id(),
        tools = agent.available_tools().len(),
        "Agent initialized"
    );

    // Demo queries showcasing compliance capabilities
    let demo_queries = vec![
        "Check regulatory compliance for manufacturing facility FAC-001",
        "Perform gap analysis on our chemical handling at FAC-002",
        "Develop a remediation plan for identified compliance gaps at FAC-003",
    ];

    for (idx, query_text) in demo_queries.iter().enumerate() {
        info!(query_num = idx + 1, "Processing query: {}", query_text);

        let query = UserQuery {
            session_id: Uuid::new_v4(),
            content: query_text.to_string(),
            context: ConversationContext {
                messages: vec![],
                metadata: HashMap::new(),
            },
            auth: AuthContext {
                user_id: "user-ehs-003".to_string(),
                tenant_id: "tenant-acme".to_string(),
                roles: vec!["compliance_officer".to_string(), "agent".to_string()],
            },
        };

        // Execute the agent reasoning cycle
        match agent.reason(&query).await {
            Ok(action) => {
                info!(iteration = idx + 1, "Agent reasoning completed");

                // Execute tools
                match agent.act(&action).await {
                    Ok(observations) => {
                        info!(
                            observation_count = observations.len(),
                            "Tool execution completed"
                        );

                        // Generate response
                        match agent.respond(&observations).await {
                            Ok(response) => {
                                info!(
                                    content_length = response.content.len(),
                                    suggested_actions = response.suggested_actions.len(),
                                    citations = response.citations.len(),
                                    "Agent response generated"
                                );
                            }
                            Err(e) => {
                                tracing::error!("Response generation failed: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Tool execution failed: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Agent reasoning failed: {}", e);
            }
        }

        info!("Query {} completed", idx + 1);
    }

    info!(
        system_prompt_length = agent.system_prompt().len(),
        max_iterations = agent.max_iterations(),
        "EHS Compliance Agent ready (demo mode - no gRPC server)"
    );

    Ok(())
}
