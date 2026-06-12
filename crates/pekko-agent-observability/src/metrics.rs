use prometheus::{
    CounterVec, Encoder, Gauge, HistogramOpts, HistogramVec, Opts, Registry, TextEncoder,
};
use std::sync::Arc;

/// All Prometheus metrics for the pekko-agent system.
///
/// Dimensions:
/// - LLM: requests, latency, token usage (by provider + agent_id)
/// - Tools: executions, latency (by tool_name)
/// - HTTP: requests, latency (by method + path pattern + status)
/// - Agents: queries (by agent_id + tenant_id), active WS connections
/// - Workflows: executions, duration (by status)
/// - RAG: searches, documents retrieved
pub struct MetricsRegistry {
    pub registry: Registry,

    // ── LLM ──────────────────────────────────────────────────────────────────
    /// llm_requests_total{provider, agent_id, status}
    pub llm_requests_total:        CounterVec,
    /// llm_request_duration_seconds{provider, agent_id}
    pub llm_request_duration_secs: HistogramVec,
    /// llm_input_tokens_total{provider}
    pub llm_input_tokens_total:    CounterVec,
    /// llm_output_tokens_total{provider}
    pub llm_output_tokens_total:   CounterVec,

    // ── Tools ─────────────────────────────────────────────────────────────────
    /// tool_executions_total{tool_name, status}
    pub tool_executions_total:     CounterVec,
    /// tool_execution_duration_seconds{tool_name}
    pub tool_duration_secs:        HistogramVec,

    // ── HTTP ──────────────────────────────────────────────────────────────────
    /// http_requests_total{method, path, status}
    pub http_requests_total:       CounterVec,
    /// http_request_duration_seconds{method, path}
    pub http_duration_secs:        HistogramVec,

    // ── Agents ────────────────────────────────────────────────────────────────
    /// agent_queries_total{agent_id, tenant_id}
    pub agent_queries_total:       CounterVec,
    /// active_ws_connections
    pub active_ws_connections:     Gauge,

    // ── Workflows ─────────────────────────────────────────────────────────────
    /// workflow_executions_total{status}
    pub workflow_executions_total: CounterVec,
    /// workflow_duration_seconds{status}
    pub workflow_duration_secs:    HistogramVec,

    // ── RAG / Vector store ────────────────────────────────────────────────────
    /// rag_searches_total{hit}  — hit=true if docs were retrieved
    pub rag_searches_total:        CounterVec,

    // ── Circuit Breaker ───────────────────────────────────────────────────────
    /// circuit_breaker_state{provider}  — 0=Closed, 1=Open, 2=HalfOpen
    pub circuit_breaker_state:         prometheus::GaugeVec,
    /// circuit_breaker_rejections_total{provider}
    pub circuit_breaker_rejections:    CounterVec,

    // ── Rate Limiting ─────────────────────────────────────────────────────────
    /// rate_limit_rejections_total{tenant_id}
    pub rate_limit_rejections:         CounterVec,
}

impl MetricsRegistry {
    pub fn new() -> anyhow::Result<Arc<Self>> {
        let registry = Registry::new_custom(Some("pekko_agent".to_string()), None)?;

        // ── LLM ──────────────────────────────────────────────────────────────
        let llm_requests_total = CounterVec::new(
            Opts::new("llm_requests_total", "Total LLM requests"),
            &["provider", "agent_id", "status"],
        )?;

        let llm_request_duration_secs = HistogramVec::new(
            HistogramOpts::new("llm_request_duration_seconds", "LLM request latency")
                .buckets(vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]),
            &["provider", "agent_id"],
        )?;

        let llm_input_tokens_total = CounterVec::new(
            Opts::new("llm_input_tokens_total", "Total input tokens consumed"),
            &["provider"],
        )?;

        let llm_output_tokens_total = CounterVec::new(
            Opts::new("llm_output_tokens_total", "Total output tokens generated"),
            &["provider"],
        )?;

        // ── Tools ─────────────────────────────────────────────────────────────
        let tool_executions_total = CounterVec::new(
            Opts::new("tool_executions_total", "Total tool executions"),
            &["tool_name", "status"],
        )?;

        let tool_duration_secs = HistogramVec::new(
            HistogramOpts::new("tool_execution_duration_seconds", "Tool execution latency")
                .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0]),
            &["tool_name"],
        )?;

        // ── HTTP ──────────────────────────────────────────────────────────────
        let http_requests_total = CounterVec::new(
            Opts::new("http_requests_total", "Total HTTP requests"),
            &["method", "path", "status"],
        )?;

        let http_duration_secs = HistogramVec::new(
            HistogramOpts::new("http_request_duration_seconds", "HTTP request latency")
                .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0]),
            &["method", "path"],
        )?;

        // ── Agents ────────────────────────────────────────────────────────────
        let agent_queries_total = CounterVec::new(
            Opts::new("agent_queries_total", "Total agent queries"),
            &["agent_id", "tenant_id"],
        )?;

        let active_ws_connections = Gauge::with_opts(
            Opts::new("active_ws_connections", "Currently open WebSocket connections"),
        )?;

        // ── Workflows ─────────────────────────────────────────────────────────
        let workflow_executions_total = CounterVec::new(
            Opts::new("workflow_executions_total", "Total workflow executions"),
            &["status"],
        )?;

        let workflow_duration_secs = HistogramVec::new(
            HistogramOpts::new("workflow_duration_seconds", "Workflow execution duration")
                .buckets(vec![1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0]),
            &["status"],
        )?;

        // ── RAG ───────────────────────────────────────────────────────────────
        let rag_searches_total = CounterVec::new(
            Opts::new("rag_searches_total", "Total RAG vector store searches"),
            &["hit"],
        )?;

        // ── Circuit Breaker ───────────────────────────────────────────────────
        let circuit_breaker_state = prometheus::GaugeVec::new(
            Opts::new("circuit_breaker_state",
                "Circuit breaker state: 0=Closed, 1=Open, 2=HalfOpen"),
            &["provider"],
        )?;

        let circuit_breaker_rejections = CounterVec::new(
            Opts::new("circuit_breaker_rejections_total",
                "Requests rejected because the circuit breaker is open"),
            &["provider"],
        )?;

        // ── Rate Limiting ─────────────────────────────────────────────────────
        let rate_limit_rejections = CounterVec::new(
            Opts::new("rate_limit_rejections_total",
                "Requests rejected by the rate limiter"),
            &["tenant_id"],
        )?;

        // Register all metrics
        for m in [
            registry.register(Box::new(llm_requests_total.clone())),
            registry.register(Box::new(llm_request_duration_secs.clone())),
            registry.register(Box::new(llm_input_tokens_total.clone())),
            registry.register(Box::new(llm_output_tokens_total.clone())),
            registry.register(Box::new(tool_executions_total.clone())),
            registry.register(Box::new(tool_duration_secs.clone())),
            registry.register(Box::new(http_requests_total.clone())),
            registry.register(Box::new(http_duration_secs.clone())),
            registry.register(Box::new(agent_queries_total.clone())),
            registry.register(Box::new(active_ws_connections.clone())),
            registry.register(Box::new(workflow_executions_total.clone())),
            registry.register(Box::new(workflow_duration_secs.clone())),
            registry.register(Box::new(rag_searches_total.clone())),
            registry.register(Box::new(circuit_breaker_state.clone())),
            registry.register(Box::new(circuit_breaker_rejections.clone())),
            registry.register(Box::new(rate_limit_rejections.clone())),
        ] {
            m?;
        }

        Ok(Arc::new(Self {
            registry,
            llm_requests_total,
            llm_request_duration_secs,
            llm_input_tokens_total,
            llm_output_tokens_total,
            tool_executions_total,
            tool_duration_secs,
            http_requests_total,
            http_duration_secs,
            agent_queries_total,
            active_ws_connections,
            workflow_executions_total,
            workflow_duration_secs,
            rag_searches_total,
            circuit_breaker_state,
            circuit_breaker_rejections,
            rate_limit_rejections,
        }))
    }

    /// Render all metrics in Prometheus text format (for `/metrics` endpoint).
    pub fn render(&self) -> anyhow::Result<String> {
        let encoder = TextEncoder::new();
        let mut buf = Vec::new();
        encoder.encode(&self.registry.gather(), &mut buf)?;
        Ok(String::from_utf8(buf)?)
    }
}
