mod permit_agent;

use permit_agent::PermitAgentActor;
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

    info!("Starting EHS Permit Agent Service");

    let mut agent = PermitAgentActor::new("permit-agent-001");

    info!(
        agent_id = agent.agent_id(),
        tools = agent.available_tools().len(),
        "Agent initialized"
    );

    // Demo queries to showcase functionality
    let demo_queries = vec![
        "Search for active environmental permits for facility FAC-001",
        "Check compliance for our manufacturing facility FAC-002",
        "Generate permit documents for the chemical facility FAC-003",
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
                user_id: "user-ehs-001".to_string(),
                tenant_id: "tenant-acme".to_string(),
                roles: vec!["ehs_specialist".to_string(), "agent".to_string()],
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
        "EHS Permit Agent ready (demo mode - no gRPC server)"
    );

    Ok(())
}
