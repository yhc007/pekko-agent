use pekko_agent_core::{Tool, ToolDefinition, ToolContext, ToolOutput, ToolError};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

/// Statistics for tool executions
#[derive(Debug, Clone, Default)]
pub struct ToolStats {
    pub call_count: u64,
    pub total_duration_ms: u64,
    pub success_count: u64,
    pub failure_count: u64,
}

impl ToolStats {
    /// Calculate average duration in milliseconds
    pub fn avg_duration_ms(&self) -> f64 {
        if self.call_count == 0 {
            0.0
        } else {
            self.total_duration_ms as f64 / self.call_count as f64
        }
    }
}

/// Central registry for managing tools
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    stats: HashMap<String, ToolStats>,
}

impl ToolRegistry {
    /// Create a new tool registry
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            stats: HashMap::new(),
        }
    }

    /// Register a tool in the registry
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let def = tool.definition();
        info!(tool_name = %def.name, description = %def.description, "Registering tool");
        self.stats.insert(def.name.clone(), ToolStats::default());
        self.tools.insert(def.name.clone(), tool);
    }

    /// Execute a registered tool
    pub async fn execute(
        &mut self,
        tool_name: &str,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self.tools.get(tool_name)
            .ok_or_else(|| ToolError::NotFound(tool_name.to_string()))?;

        // Validate input before execution
        tool.validate_input(&input)?;

        let start = Instant::now();
        let result = tool.execute(input, ctx).await;
        let duration = start.elapsed().as_millis() as u64;

        // Update statistics
        let stats = self.stats.entry(tool_name.to_string()).or_insert_with(ToolStats::default);
        stats.call_count += 1;
        stats.total_duration_ms += duration;

        match &result {
            Ok(_) => stats.success_count += 1,
            Err(_) => stats.failure_count += 1,
        }

        result
    }

    /// Get all registered tool definitions
    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    /// Get a specific tool definition
    pub fn get_tool_definition(&self, tool_name: &str) -> Option<ToolDefinition> {
        self.tools.get(tool_name).map(|t| t.definition())
    }

    /// Get execution statistics for a tool
    pub fn get_stats(&self, tool_name: &str) -> Option<&ToolStats> {
        self.stats.get(tool_name)
    }

    /// Get all statistics
    pub fn get_all_stats(&self) -> HashMap<String, ToolStats> {
        self.stats.clone()
    }

    /// Check if a tool is registered
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.tools.contains_key(tool_name)
    }

    /// Check if user has required permissions for a tool
    pub fn check_permission(&self, tool_name: &str, user_permissions: &[String]) -> bool {
        if let Some(tool) = self.tools.get(tool_name) {
            let def = tool.definition();
            def.required_permissions.iter().all(|req| user_permissions.contains(req))
        } else {
            false
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = ToolRegistry::new();
        assert_eq!(registry.tools.len(), 0);
    }

    #[test]
    fn test_has_tool() {
        let registry = ToolRegistry::new();
        assert!(!registry.has_tool("test_tool"));
    }

    #[test]
    fn test_stats_avg_duration() {
        let mut stats = ToolStats::default();
        stats.call_count = 10;
        stats.total_duration_ms = 100;
        assert_eq!(stats.avg_duration_ms(), 10.0);
    }

    #[test]
    fn test_stats_avg_duration_zero() {
        let stats = ToolStats::default();
        assert_eq!(stats.avg_duration_ms(), 0.0);
    }
}
