# Pekko Agent Crates - Usage Examples

## pekko-agent-events

### Publishing Events

```rust
use pekko_agent_events::{EventPublisher, AgentEventEnvelope};
use uuid::Uuid;
use serde_json::json;

#[tokio::main]
async fn main() {
    // Create publisher with 100-event capacity
    let publisher = EventPublisher::new("agent", 100);
    
    // Create and publish an event
    let event = AgentEventEnvelope::new(
        "task-service",
        "task.assigned",
        "tenant-123",
        Uuid::new_v4(),
        json!({ "task_id": "task-456", "priority": "high" }),
    );
    
    publisher.publish(event).await.unwrap();
    
    // Subscribe to events
    let mut consumer = publisher.subscribe();
    // Process events...
}
```

### Consuming Events

```rust
use pekko_agent_events::EventConsumer;

#[tokio::main]
async fn main() {
    let publisher = EventPublisher::new("agent", 100);
    let receiver = publisher.subscribe();
    
    let mut consumer = EventConsumer::new(receiver)
        .with_filter("task.completed");
    
    // Consume one event with filtering
    if let Some(event) = consumer.consume_one().await {
        println!("Task completed: {:?}", event.payload);
    }
}
```

## pekko-agent-orchestrator

### Creating and Running Workflows

```rust
use pekko_agent_orchestrator::{Workflow, WorkflowStep, OrchestratorActor};
use std::collections::HashMap;

fn main() {
    let mut orchestrator = OrchestratorActor::new();
    
    // Create a workflow
    let mut workflow = Workflow::new("EHS Inspection", "Environmental health & safety");
    
    // Add workflow steps
    let step1 = WorkflowStep {
        step_id: "step-1".to_string(),
        agent_type: "inspection_agent".to_string(),
        action: "inspect_site".to_string(),
        input_mapping: HashMap::new(),
        output_key: "inspection_results".to_string(),
        depends_on: vec![],
        timeout_ms: 30000,
    };
    
    workflow.add_step(step1);
    
    // Create the workflow
    let workflow_id = orchestrator.create_workflow(workflow);
    
    // Advance the workflow
    if let Some(wf) = orchestrator.get_workflow(&workflow_id) {
        println!("Workflow: {}", wf.name);
    }
}
```

### Task Assignment

```rust
use pekko_agent_orchestrator::OrchestratorActor;
use pekko_agent_core::{AgentInfo, AgentStatus, AgentTask};

fn main() {
    let mut orchestrator = OrchestratorActor::new();
    
    // Register agents
    let agent = AgentInfo {
        agent_id: "agent-1".to_string(),
        agent_type: "worker".to_string(),
        status: AgentStatus::Available,
        capabilities: vec!["inspection".to_string()],
        max_concurrent_tasks: 5,
    };
    
    orchestrator.register_agent(agent);
    
    // Submit a task
    let task = AgentTask {
        task_id: uuid::Uuid::new_v4(),
        description: "Inspect equipment".to_string(),
        priority: pekko_agent_core::TaskPriority::High,
        parameters: serde_json::json!({}),
        deadline: None,
    };
    
    orchestrator.submit_task(task);
    
    // Assign next task to available agent
    if let Some((agent_id, task)) = orchestrator.assign_next_task() {
        println!("Task {} assigned to {}", task.task_id, agent_id);
    }
}
```

### Saga Pattern for Distributed Transactions

```rust
use pekko_agent_orchestrator::{SagaManager, SagaDefinition, SagaStep};
use uuid::Uuid;

fn main() {
    let mut saga_manager = SagaManager::new();
    
    // Define a saga with compensating transactions
    let saga = SagaDefinition {
        saga_id: Uuid::new_v4(),
        name: "Permit Creation Saga".to_string(),
        steps: vec![
            SagaStep {
                step_name: "create_permit".to_string(),
                agent_type: "permit_agent".to_string(),
                action: "create".to_string(),
                compensation_action: "delete_permit".to_string(),
            },
            SagaStep {
                step_name: "schedule_inspection".to_string(),
                agent_type: "inspection_agent".to_string(),
                action: "schedule".to_string(),
                compensation_action: "cancel_inspection".to_string(),
            },
        ],
    };
    
    saga_manager.register_saga(saga.clone());
    
    // Start execution
    let execution_id = saga_manager.start_execution(&saga.saga_id).unwrap();
    
    // Complete steps
    saga_manager.complete_step(&execution_id, 0);
    saga_manager.complete_step(&execution_id, 1);
    
    println!("Saga completed: {:?}", saga_manager.get_execution(&execution_id));
}
```

## pekko-agent-security

### Role-Based Access Control

```rust
use pekko_agent_security::{RbacManager, Permission};

fn main() {
    let mut rbac = RbacManager::new();
    
    // Assign roles to agents
    rbac.assign_role("agent-1", "admin");
    rbac.assign_role("agent-2", "agent");
    
    // Check permissions
    assert!(rbac.check_permission("agent-1", "workflow.create"));
    assert!(!rbac.check_permission("agent-2", "admin.all"));
    
    // Add custom role
    rbac.add_role("inspector", vec![
        Permission::ToolExecute("inspection".to_string()),
        Permission::MemoryRead,
        Permission::MemoryWrite,
    ]);
    
    rbac.assign_role("agent-3", "inspector");
    assert!(rbac.check_permission("agent-3", "tool.inspection"));
}
```

### Multi-Tenant Management

```rust
use pekko_agent_security::{TenantManager, TenantContext, IsolationLevel, ResourceLimits};

fn main() {
    let mut tenant_mgr = TenantManager::new();
    
    // Create tenant context
    let tenant = TenantContext {
        tenant_id: "tenant-123".to_string(),
        tenant_name: "ACME Corp".to_string(),
        isolation_level: IsolationLevel::Dedicated,
        resource_limits: ResourceLimits {
            max_agents: 50,
            max_tokens_per_day: 500_000,
            max_concurrent_requests: 100,
            max_storage_mb: 5120,
        },
        metadata: Default::default(),
    };
    
    tenant_mgr.register_tenant(tenant);
    
    // Validate requests
    match tenant_mgr.validate_request("tenant-123") {
        Ok(ctx) => println!("Tenant: {}", ctx.tenant_name),
        Err(e) => println!("Error: {}", e),
    }
}
```

### Audit Logging

```rust
use pekko_agent_security::{AuditLogger, AuditEntry, AuditOutcome};
use uuid::Uuid;
use chrono::Utc;

#[tokio::main]
async fn main() {
    let audit_logger = AuditLogger::new(1000); // Keep last 1000 entries
    
    // Log an audit entry
    let entry = AuditEntry {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        tenant_id: "tenant-123".to_string(),
        agent_id: "agent-1".to_string(),
        action: "execute_tool".to_string(),
        resource: "inspection_tool".to_string(),
        outcome: AuditOutcome::Success,
        details: serde_json::json!({"duration_ms": 150}),
    };
    
    audit_logger.log(entry).await;
    
    // Query audit logs
    let logs = audit_logger.query(
        Some("tenant-123"),
        Some("agent-1"),
        10
    ).await;
    
    println!("Audit entries: {}", logs.len());
}
```

## Integration Example

```rust
use pekko_agent_events::{EventPublisher, AgentEventEnvelope, event_types};
use pekko_agent_orchestrator::OrchestratorActor;
use pekko_agent_security::AuditLogger;
use serde_json::json;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    // Initialize components
    let publisher = EventPublisher::new("agent", 100);
    let mut orchestrator = OrchestratorActor::new();
    let audit_logger = AuditLogger::new(1000);
    
    let correlation_id = Uuid::new_v4();
    
    // Publish event
    let event = AgentEventEnvelope::new(
        "coordinator",
        event_types::TASK_ASSIGNED,
        "tenant-1",
        correlation_id,
        json!({"task": "inspect", "agent": "agent-1"}),
    );
    
    publisher.publish(event).await.unwrap();
    
    // Log audit entry
    let audit_entry = pekko_agent_security::AuditEntry {
        id: Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        tenant_id: "tenant-1".to_string(),
        agent_id: "agent-1".to_string(),
        action: "task_assigned".to_string(),
        resource: "task:123".to_string(),
        outcome: pekko_agent_security::AuditOutcome::Success,
        details: json!({"correlation_id": correlation_id}),
    };
    
    audit_logger.log(audit_entry).await;
    
    println!("Integration test completed successfully!");
}
```

## Testing

All components are designed for easy testing:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_event_publishing() {
        let publisher = EventPublisher::new("test", 10);
        let event = AgentEventEnvelope::new(
            "service",
            "test.event",
            "tenant-1",
            Uuid::new_v4(),
            json!({}),
        );
        
        assert!(publisher.publish(event).await.is_ok());
    }
    
    #[test]
    fn test_rbac() {
        let mut rbac = RbacManager::new();
        rbac.assign_role("agent-1", "admin");
        assert!(rbac.check_permission("agent-1", "workflow.create"));
    }
}
```
