use std::sync::Arc;

use sqlx::PgPool;
use tracing::{error, info, warn};
use uuid::Uuid;

use pekko_agent_core::{AgentInfo, AgentProfile};

use crate::workflow::{Workflow, WorkflowStatus};

/// PostgreSQL-backed persistence for orchestrator state.
///
/// Persists:
///   - Agent registry (AgentInfo + AgentProfile)
///   - Workflow run history and status
///
/// All writes are best-effort: if the DB is unavailable the actor keeps
/// running with in-memory state and emits a warning.
pub struct OrchestratorPersistence {
    pool: Arc<PgPool>,
}

impl OrchestratorPersistence {
    pub fn new(pool: Arc<PgPool>) -> Arc<Self> {
        Arc::new(Self { pool })
    }

    /// Create tables if they don't exist.
    pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS pekko_agent_registry (
                agent_id   TEXT        PRIMARY KEY,
                info       JSONB       NOT NULL,
                profile    JSONB       NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );

            CREATE TABLE IF NOT EXISTS pekko_workflow_runs (
                workflow_id UUID        PRIMARY KEY,
                data        JSONB       NOT NULL,
                status      TEXT        NOT NULL,
                updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            "#,
        )
        .execute(pool)
        .await?;

        info!("OrchestratorPersistence tables ready");
        Ok(())
    }

    // ── Agent Registry ────────────────────────────────────────────────────────

    /// Upsert a registered agent (info + profile).
    pub async fn save_agent(
        &self,
        info: &AgentInfo,
        profile: &AgentProfile,
    ) -> Result<(), anyhow::Error> {
        let info_json    = serde_json::to_value(info)?;
        let profile_json = serde_json::to_value(profile)?;

        sqlx::query(
            r#"
            INSERT INTO pekko_agent_registry (agent_id, info, profile, updated_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (agent_id) DO UPDATE
                SET info       = EXCLUDED.info,
                    profile    = EXCLUDED.profile,
                    updated_at = NOW()
            "#,
        )
        .bind(&info.agent_id)
        .bind(info_json)
        .bind(profile_json)
        .execute(&*self.pool)
        .await?;

        Ok(())
    }

    /// Delete an agent from the registry.
    pub async fn delete_agent(&self, agent_id: &str) -> Result<(), anyhow::Error> {
        sqlx::query("DELETE FROM pekko_agent_registry WHERE agent_id = $1")
            .bind(agent_id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    /// Load all persisted agents — returns `(AgentInfo, AgentProfile)` pairs.
    pub async fn load_agents(&self) -> Result<Vec<(AgentInfo, AgentProfile)>, anyhow::Error> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT info, profile FROM pekko_agent_registry ORDER BY updated_at"
        )
        .fetch_all(&*self.pool)
        .await?;

        let mut agents = Vec::with_capacity(rows.len());
        for row in rows {
            let info_val:    serde_json::Value = row.try_get("info")?;
            let profile_val: serde_json::Value = row.try_get("profile")?;
            let info: AgentInfo       = serde_json::from_value(info_val)?;
            let profile: AgentProfile = serde_json::from_value(profile_val)?;
            agents.push((info, profile));
        }
        Ok(agents)
    }

    // ── Workflow Runs ─────────────────────────────────────────────────────────

    /// Upsert a workflow (full snapshot as JSONB).
    pub async fn save_workflow(&self, workflow: &Workflow) -> Result<(), anyhow::Error> {
        let data   = serde_json::to_value(workflow)?;
        let status = workflow_status_label(&workflow.status);

        sqlx::query(
            r#"
            INSERT INTO pekko_workflow_runs (workflow_id, data, status, updated_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (workflow_id) DO UPDATE
                SET data       = EXCLUDED.data,
                    status     = EXCLUDED.status,
                    updated_at = NOW()
            "#,
        )
        .bind(workflow.id)
        .bind(data)
        .bind(status)
        .execute(&*self.pool)
        .await?;

        Ok(())
    }

    /// Update only the status label of an existing workflow run (lightweight).
    pub async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        new_status: &WorkflowStatus,
        updated_data: Option<&Workflow>,
    ) -> Result<(), anyhow::Error> {
        let label = workflow_status_label(new_status);

        if let Some(wf) = updated_data {
            let data = serde_json::to_value(wf)?;
            sqlx::query(
                r#"UPDATE pekko_workflow_runs
                   SET status = $2, data = $3, updated_at = NOW()
                   WHERE workflow_id = $1"#,
            )
            .bind(workflow_id)
            .bind(label)
            .bind(data)
            .execute(&*self.pool)
            .await?;
        } else {
            sqlx::query(
                r#"UPDATE pekko_workflow_runs
                   SET status = $2, updated_at = NOW()
                   WHERE workflow_id = $1"#,
            )
            .bind(workflow_id)
            .bind(label)
            .execute(&*self.pool)
            .await?;
        }

        Ok(())
    }

    /// Load all persisted workflows.
    ///
    /// Caller is responsible for marking any `Running` workflows as `Failed`
    /// since their execution context was lost on restart.
    pub async fn load_workflows(&self) -> Result<Vec<Workflow>, anyhow::Error> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT data FROM pekko_workflow_runs ORDER BY updated_at"
        )
        .fetch_all(&*self.pool)
        .await?;

        let mut workflows = Vec::with_capacity(rows.len());
        for row in rows {
            let data: serde_json::Value = row.try_get("data")?;
            match serde_json::from_value::<Workflow>(data) {
                Ok(wf)  => workflows.push(wf),
                Err(e)  => warn!(error = %e, "Skipping corrupt workflow row"),
            }
        }
        Ok(workflows)
    }

    /// Delete a specific workflow run record.
    pub async fn delete_workflow(&self, workflow_id: Uuid) -> Result<(), anyhow::Error> {
        sqlx::query("DELETE FROM pekko_workflow_runs WHERE workflow_id = $1")
            .bind(workflow_id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn workflow_status_label(status: &WorkflowStatus) -> &'static str {
    match status {
        WorkflowStatus::Created                    => "created",
        WorkflowStatus::Running { .. }             => "running",
        WorkflowStatus::Paused  { .. }             => "paused",
        WorkflowStatus::Completed                  => "completed",
        WorkflowStatus::Failed  { .. }             => "failed",
        WorkflowStatus::Cancelled                  => "cancelled",
        WorkflowStatus::Compensating { .. }        => "compensating",
        WorkflowStatus::Compensated                => "compensated",
        WorkflowStatus::CompensationFailed { .. }  => "compensation_failed",
    }
}

/// Fire-and-forget helper: persists `workflow` in the background.
/// Errors are logged as warnings so the caller is never blocked.
pub fn spawn_save_workflow(store: Arc<OrchestratorPersistence>, workflow: Workflow) {
    tokio::spawn(async move {
        if let Err(e) = store.save_workflow(&workflow).await {
            warn!(workflow_id = %workflow.id, error = %e, "Failed to persist workflow");
        }
    });
}

/// Fire-and-forget helper: persists agent registration in the background.
pub fn spawn_save_agent(
    store: Arc<OrchestratorPersistence>,
    info: AgentInfo,
    profile: AgentProfile,
) {
    tokio::spawn(async move {
        if let Err(e) = store.save_agent(&info, &profile).await {
            warn!(agent_id = %info.agent_id, error = %e, "Failed to persist agent");
        }
    });
}
