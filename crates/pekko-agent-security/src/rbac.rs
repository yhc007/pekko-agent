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
            Permission::AdminAll => true, // admin grants every permission string
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
        // "viewer" role can only read, "agent" role can also delegate
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

    /// Check whether any of the provided `roles` (from a JWT token) grants `required`.
    /// Used by HTTP handlers after the JWT extractor has already validated the token.
    pub fn check_user_permission(&self, roles: &[String], required: &str) -> bool {
        roles.iter().any(|role| {
            self.roles.get(role.as_str())
                .map(|perms| perms.iter().any(|p| p.matches(required)))
                .unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_role_grants_all_permissions() {
        let mut rbac = RbacManager::new();
        rbac.assign_role("admin-user", "admin");

        assert!(rbac.check_permission("admin-user", "memory.read"));
        assert!(rbac.check_permission("admin-user", "agent.delegate"));
        assert!(rbac.check_permission("admin-user", "admin.all"));
        assert!(rbac.check_permission("admin-user", "any.unknown.permission"));
    }

    #[test]
    fn agent_role_grants_correct_permissions() {
        let mut rbac = RbacManager::new();
        rbac.assign_role("my-agent", "agent");

        assert!(rbac.check_permission("my-agent", "memory.read"));
        assert!(rbac.check_permission("my-agent", "memory.write"));
        assert!(rbac.check_permission("my-agent", "agent.delegate"));
        assert!(!rbac.check_permission("my-agent", "admin.all"));
        assert!(!rbac.check_permission("my-agent", "audit.access"));
    }

    #[test]
    fn viewer_role_grants_read_and_audit_only() {
        let mut rbac = RbacManager::new();
        rbac.assign_role("reader", "viewer");

        assert!(rbac.check_permission("reader", "memory.read"));
        assert!(rbac.check_permission("reader", "audit.access"));
        assert!(!rbac.check_permission("reader", "memory.write"));
        assert!(!rbac.check_permission("reader", "agent.delegate"));
    }

    #[test]
    fn unknown_agent_returns_false() {
        let rbac = RbacManager::new();
        assert!(!rbac.check_permission("no-such-agent", "memory.read"));
    }

    #[test]
    fn check_user_permission_with_jwt_roles_vec() {
        let rbac = RbacManager::new();

        let admin_roles = vec!["admin".to_string()];
        assert!(rbac.check_user_permission(&admin_roles, "memory.read"));
        assert!(rbac.check_user_permission(&admin_roles, "any.permission"));

        let agent_roles = vec!["agent".to_string()];
        assert!(rbac.check_user_permission(&agent_roles, "agent.delegate"));
        assert!(!rbac.check_user_permission(&agent_roles, "admin.all"));

        let empty: Vec<String> = vec![];
        assert!(!rbac.check_user_permission(&empty, "memory.read"));
    }

    #[test]
    fn custom_ehs_operator_role() {
        let mut rbac = RbacManager::new();
        rbac.add_role("ehs-operator", vec![
            Permission::ToolExecute("permit_search".into()),
            Permission::MemoryRead,
        ]);
        rbac.assign_role("op-1", "ehs-operator");

        assert!(rbac.check_permission("op-1", "memory.read"));
        assert!(rbac.check_permission("op-1", "tool.permit_search"));
        assert!(!rbac.check_permission("op-1", "agent.delegate"));
        assert!(!rbac.check_permission("op-1", "tool.other_tool"));
    }

    #[test]
    fn permission_tool_execute_matches_tool_string() {
        let p = Permission::ToolExecute("permit_search".into());
        assert!(p.matches("tool.permit_search"));
        assert!(!p.matches("tool.compliance_check"));
        assert!(!p.matches("memory.read"));
    }

    #[test]
    fn get_agent_permissions_returns_correct_list() {
        let mut rbac = RbacManager::new();
        rbac.assign_role("agent-x", "agent");
        let perms = rbac.get_agent_permissions("agent-x");
        // agent role: MemoryRead, MemoryWrite, AgentDelegate
        assert_eq!(perms.len(), 3);
    }
}
