use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Permission {
    ToolExecute(String),
    MemoryRead,
    MemoryWrite,
    AgentDelegate,
    WorkflowCreate,
    AuditAccess,
    AdminAll,
}

impl Permission {
    pub fn matches(&self, required: &str) -> bool {
        match self {
            Permission::AdminAll => true,
            Permission::ToolExecute(tool) => required.starts_with("tool.") && required.ends_with(tool),
            Permission::MemoryRead => required == "memory.read",
            Permission::MemoryWrite => required == "memory.write",
            Permission::AgentDelegate => required == "agent.delegate",
            Permission::WorkflowCreate => required == "workflow.create",
            Permission::AuditAccess => required == "audit.access",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RbacManager {
    roles: HashMap<String, Vec<Permission>>,
    agent_roles: HashMap<String, String>,
}

impl RbacManager {
    pub fn new() -> Self {
        let mut rbac = Self {
            roles: HashMap::new(),
            agent_roles: HashMap::new(),
        };
        rbac.setup_default_roles();
        rbac
    }

    fn setup_default_roles(&mut self) {
        self.roles.insert("admin".to_string(), vec![Permission::AdminAll]);
        self.roles.insert("agent".to_string(), vec![
            Permission::MemoryRead,
            Permission::MemoryWrite,
            Permission::AgentDelegate,
        ]);
        self.roles.insert("viewer".to_string(), vec![
            Permission::MemoryRead,
            Permission::AuditAccess,
        ]);
    }

    pub fn assign_role(&mut self, agent_id: impl Into<String>, role: impl Into<String>) {
        self.agent_roles.insert(agent_id.into(), role.into());
    }

    pub fn add_role(&mut self, role_name: impl Into<String>, permissions: Vec<Permission>) {
        self.roles.insert(role_name.into(), permissions);
    }

    pub fn check_permission(&self, agent_id: &str, required: &str) -> bool {
        let role = match self.agent_roles.get(agent_id) {
            Some(r) => r,
            None => {
                warn!(agent_id = %agent_id, "No role assigned");
                return false;
            }
        };

        let permissions = match self.roles.get(role) {
            Some(p) => p,
            None => {
                warn!(role = %role, "Role not found");
                return false;
            }
        };

        permissions.iter().any(|p| p.matches(required))
    }

    pub fn get_agent_permissions(&self, agent_id: &str) -> Vec<&Permission> {
        self.agent_roles.get(agent_id)
            .and_then(|role| self.roles.get(role))
            .map(|perms| perms.iter().collect())
            .unwrap_or_default()
    }
}
