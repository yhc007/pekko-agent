//! gRPC service — implements pekko.agent.v1.AgentService (proto/agent.proto).
//!
//! Runs on `GRPC_PORT` (default 50051) alongside the REST API.
//! Authentication: pass the JWT as `authorization: Bearer <token>` metadata.

pub mod proto {
    tonic::include_proto!("pekko.agent.v1");
}

use proto::{
    agent_service_server::{AgentService, AgentServiceServer},
    QueryChunk, QueryRequest, QueryResponse, StatusRequest, StatusResponse,
    TaskRequest, TaskResponse, TokenUsage,
};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use pekko_agent_core::AgentInfo;
use pekko_agent_orchestrator::OrchestratorMessage;
use pekko_agent_security::JwtError;
use tracing::info;

use crate::AppState;

// ── Service struct ─────────────────────────────────────────────────────────────

pub struct AgentGrpcService {
    state: AppState,
}

impl AgentGrpcService {
    fn new(state: AppState) -> Self {
        Self { state }
    }

    fn extract_claims(
        &self,
        metadata: &tonic::metadata::MetadataMap,
    ) -> Result<pekko_agent_security::Claims, Status> {
        let raw = metadata
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Status::unauthenticated("Missing 'authorization' metadata"))?;

        let token = raw
            .strip_prefix("Bearer ")
            .ok_or_else(|| Status::unauthenticated("Authorization must be 'Bearer <token>'"))?;

        self.state
            .jwt_manager
            .validate(token)
            .map_err(|e| match e {
                JwtError::Expired => Status::unauthenticated("Token has expired"),
                _ => Status::unauthenticated("Invalid token"),
            })
    }
}

// ── Service implementation ─────────────────────────────────────────────────────

#[tonic::async_trait]
impl AgentService for AgentGrpcService {
    type StreamQueryStream = ReceiverStream<Result<QueryChunk, Status>>;

    /// Synchronous unary query.
    ///
    /// Routes to `ehs-permit-agent` by default (the proto has no agent_id field;
    /// callers that need to target a specific agent should use `AssignTask`).
    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let claims = self.extract_claims(request.metadata())?;
        let req = request.into_inner();
        let session_id = parse_session_id(&req.session_id)?;

        info!(
            user = %claims.sub, tenant = %claims.tenant_id,
            session = %session_id, "gRPC Query"
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        self.state
            .orchestrator_ref
            .tell(OrchestratorMessage::QueryAgent {
                agent_id:   "ehs-permit-agent".to_string(),
                content:    req.content,
                session_id,
                tenant_id:  claims.tenant_id,
                user_id:    claims.sub,
                reply_to:   reply_tx,
            })
            .map_err(|_| Status::unavailable("Orchestrator unavailable"))?;

        let result = reply_rx
            .await
            .map_err(|_| Status::internal("Orchestrator did not reply"))?
            .map_err(|e| Status::internal(e))?;

        Ok(Response::new(QueryResponse {
            content: result.response,
            citations: vec![],
            usage: Some(TokenUsage {
                input_tokens:  result.input_tokens,
                output_tokens: result.output_tokens,
            }),
        }))
    }

    /// Server-streaming query — streams text deltas as they are generated.
    async fn stream_query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<Self::StreamQueryStream>, Status> {
        let claims = self.extract_claims(request.metadata())?;
        let req = request.into_inner();
        let session_id = parse_session_id(&req.session_id)?;

        info!(
            user = %claims.sub, tenant = %claims.tenant_id,
            session = %session_id, "gRPC StreamQuery"
        );

        let (event_tx, mut event_rx) = mpsc::channel::<String>(256);
        let (chunk_tx, chunk_rx) = mpsc::channel::<Result<QueryChunk, Status>>(256);

        self.state
            .orchestrator_ref
            .tell(OrchestratorMessage::StreamAgent {
                agent_id:  "ehs-permit-agent".to_string(),
                content:   req.content,
                session_id,
                tenant_id: claims.tenant_id,
                user_id:   claims.sub,
                event_tx,
            })
            .map_err(|_| Status::unavailable("Orchestrator unavailable"))?;

        // Bridge orchestrator SSE events → gRPC QueryChunk stream
        tokio::spawn(async move {
            while let Some(json) = event_rx.recv().await {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                    let send = match v["type"].as_str() {
                        Some("text_chunk") => {
                            let delta = v["text"].as_str().unwrap_or("").to_string();
                            chunk_tx.send(Ok(QueryChunk { delta, done: false })).await
                        }
                        Some("done") => {
                            let r = chunk_tx
                                .send(Ok(QueryChunk { delta: String::new(), done: true }))
                                .await;
                            break; // always break after done
                            #[allow(unreachable_code)]
                            r
                        }
                        Some("error") => {
                            let msg = v["message"].as_str().unwrap_or("unknown").to_string();
                            let _ = chunk_tx.send(Err(Status::internal(msg))).await;
                            break;
                        }
                        _ => continue,
                    };
                    if send.is_err() {
                        break; // client disconnected
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(chunk_rx)))
    }

    /// Assign a task to a named agent type and wait for its completion.
    ///
    /// `TaskRequest.agent_type` maps directly to the registered agent_id
    /// (e.g. `"ehs-permit-agent"`, `"ehs-compliance-agent"`).
    async fn assign_task(
        &self,
        request: Request<TaskRequest>,
    ) -> Result<Response<TaskResponse>, Status> {
        let claims = self.extract_claims(request.metadata())?;
        let req = request.into_inner();

        let content = if req.input_json.is_empty() {
            req.description.clone()
        } else {
            format!("{}\n\nInput:\n{}", req.description, req.input_json)
        };

        info!(
            user = %claims.sub, agent_type = %req.agent_type,
            task_id = %req.task_id, "gRPC AssignTask"
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        self.state
            .orchestrator_ref
            .tell(OrchestratorMessage::QueryAgent {
                agent_id:   req.agent_type.clone(),
                content,
                session_id: Uuid::new_v4(),
                tenant_id:  claims.tenant_id,
                user_id:    claims.sub,
                reply_to:   reply_tx,
            })
            .map_err(|_| Status::unavailable("Orchestrator unavailable"))?;

        match reply_rx.await {
            Ok(Ok(_)) => Ok(Response::new(TaskResponse {
                task_id: req.task_id,
                status:  "completed".to_string(),
            })),
            Ok(Err(e)) => Err(Status::internal(format!("Task failed: {e}"))),
            Err(_) => Err(Status::internal("Orchestrator did not reply")),
        }
    }

    /// Return current state information for a registered agent.
    async fn get_status(
        &self,
        request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        let _claims = self.extract_claims(request.metadata())?;
        let req = request.into_inner();

        let (reply_tx, reply_rx) = oneshot::channel::<Vec<AgentInfo>>();
        self.state
            .orchestrator_ref
            .tell(OrchestratorMessage::GetAgents { reply_to: reply_tx })
            .map_err(|_| Status::unavailable("Orchestrator unavailable"))?;

        let agents = reply_rx
            .await
            .map_err(|_| Status::internal("Orchestrator did not reply"))?;

        let agent = agents.iter().find(|a| a.agent_id == req.agent_id).ok_or_else(|| {
            Status::not_found(format!("Agent '{}' not found", req.agent_id))
        })?;

        Ok(Response::new(StatusResponse {
            agent_id:       agent.agent_id.clone(),
            state:          format!("{:?}", agent.status),
            total_requests: 0, // tracked via Prometheus agent_queries_total metric
        }))
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn parse_session_id(s: &str) -> Result<Uuid, Status> {
    if s.is_empty() {
        Ok(Uuid::new_v4())
    } else {
        Uuid::parse_str(s).map_err(|_| Status::invalid_argument("session_id must be a valid UUID"))
    }
}

// ── Public factory ─────────────────────────────────────────────────────────────

/// Build the gRPC service, ready for `tonic::transport::Server::add_service`.
pub fn service(state: AppState) -> AgentServiceServer<AgentGrpcService> {
    AgentServiceServer::new(AgentGrpcService::new(state))
}
