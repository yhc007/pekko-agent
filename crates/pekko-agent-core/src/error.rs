use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("Max iterations exceeded: {0}")]
    MaxIterationsExceeded(u32),
    #[error("Agent not found: {0}")]
    AgentNotFound(String),
    #[error("Tool error: {0}")]
    ToolError(#[from] ToolError),
    #[error("LLM error: {0}")]
    LlmError(String),
    #[error("Memory error: {0}")]
    MemoryError(#[from] MemoryError),
    #[error("Security error: {0}")]
    SecurityError(String),
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

#[derive(Error, Debug)]
pub enum ToolError {
    #[error("Tool not found: {0}")]
    NotFound(String),
    #[error("Validation failed: {0}")]
    ValidationFailed(String),
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Timeout")]
    Timeout,
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

#[derive(Error, Debug)]
pub enum MemoryError {
    #[error("Storage error: {0}")]
    StorageError(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
}
