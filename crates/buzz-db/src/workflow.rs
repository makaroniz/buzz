//! Workflow CRUD -- workflows, workflow_runs, and workflow_approvals tables.
//!
//! All IDs are native Postgres UUID columns. Never uses string interpolation
//! for query values -- all user data goes through bind parameters.
//!
//! Security notes:
//! - Approval tokens are stored as SHA-256 hashes (never plaintext).
//! - All list queries have a bounded LIMIT to prevent unbounded scans.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use buzz_core::CommunityId;

use crate::error::{DbError, Result};

// -- Token hashing ------------------------------------------------------------

/// Default maximum rows returned by list queries. Callers may request fewer.
pub const LIST_DEFAULT_LIMIT: i64 = 100;
/// Hard cap on rows returned by list queries.
pub const LIST_MAX_LIMIT: i64 = 1000;

/// SHA-256 hash of a raw approval token. Returns the 32-byte digest.
///
/// Approval tokens are stored hashed so that a DB read does not expose
/// the raw token (same pattern as API tokens in buzz-auth).
fn hash_approval_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

// -- Status enums -------------------------------------------------------------

/// Status of a workflow definition. Stored as ENUM('active','disabled','archived').
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowStatus {
    /// Workflow is live and will fire on matching events.
    Active,
    /// Workflow is paused and will not fire.
    Disabled,
    /// Workflow has been retired.
    Archived,
}

impl fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkflowStatus::Active => write!(f, "active"),
            WorkflowStatus::Disabled => write!(f, "disabled"),
            WorkflowStatus::Archived => write!(f, "archived"),
        }
    }
}

impl FromStr for WorkflowStatus {
    type Err = DbError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "active" => Ok(WorkflowStatus::Active),
            "disabled" => Ok(WorkflowStatus::Disabled),
            "archived" => Ok(WorkflowStatus::Archived),
            other => Err(DbError::InvalidData(format!(
                "unknown workflow status: {other}"
            ))),
        }
    }
}

/// Status of a workflow run. Stored as ENUM in workflow_runs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Run is queued but not yet started.
    Pending,
    /// Run is actively executing steps.
    Running,
    /// Run is suspended waiting for an approval gate.
    WaitingApproval,
    /// Run finished successfully.
    Completed,
    /// Run terminated with an error.
    Failed,
    /// Run was cancelled before completion.
    Cancelled,
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunStatus::Pending => write!(f, "pending"),
            RunStatus::Running => write!(f, "running"),
            RunStatus::WaitingApproval => write!(f, "waiting_approval"),
            RunStatus::Completed => write!(f, "completed"),
            RunStatus::Failed => write!(f, "failed"),
            RunStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl FromStr for RunStatus {
    type Err = DbError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(RunStatus::Pending),
            "running" => Ok(RunStatus::Running),
            "waiting_approval" => Ok(RunStatus::WaitingApproval),
            "completed" => Ok(RunStatus::Completed),
            "failed" => Ok(RunStatus::Failed),
            "cancelled" => Ok(RunStatus::Cancelled),
            other => Err(DbError::InvalidData(format!("unknown run status: {other}"))),
        }
    }
}

/// Status of an approval request. Stored as ENUM in workflow_approvals.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalStatus {
    /// Approval has been requested but not yet acted on.
    Pending,
    /// Approval was granted; the run may proceed.
    Granted,
    /// Approval was denied; the run should fail.
    Denied,
    /// The approval window elapsed without a decision.
    Expired,
}

impl fmt::Display for ApprovalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApprovalStatus::Pending => write!(f, "pending"),
            ApprovalStatus::Granted => write!(f, "granted"),
            ApprovalStatus::Denied => write!(f, "denied"),
            ApprovalStatus::Expired => write!(f, "expired"),
        }
    }
}

impl FromStr for ApprovalStatus {
    type Err = DbError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(ApprovalStatus::Pending),
            "granted" => Ok(ApprovalStatus::Granted),
            "denied" => Ok(ApprovalStatus::Denied),
            "expired" => Ok(ApprovalStatus::Expired),
            other => Err(DbError::InvalidData(format!(
                "unknown approval status: {other}"
            ))),
        }
    }
}

// -- Record types -------------------------------------------------------------

/// A workflow definition record. Run-state columns live in `workflow_runs`.
#[derive(Debug, Clone)]
pub struct WorkflowRecord {
    /// Unique workflow identifier.
    pub id: Uuid,
    /// Server-resolved community that owns this workflow.
    pub community_id: CommunityId,
    /// Human-readable workflow name.
    pub name: String,
    /// Compressed public key bytes of the workflow owner.
    pub owner_pubkey: Vec<u8>,
    /// Channel this workflow is scoped to, if any.
    pub channel_id: Option<Uuid>,
    /// Canonical JSON of the workflow definition.
    pub definition: serde_json::Value,
    /// SHA-256 hash of the canonical definition JSON.
    pub definition_hash: Vec<u8>,
    /// Current lifecycle status of the workflow definition.
    pub status: WorkflowStatus,
    /// Whether the workflow will fire on matching events.
    pub enabled: bool,
    /// When the workflow was created.
    pub created_at: DateTime<Utc>,
    /// When the workflow was last updated.
    pub updated_at: DateTime<Utc>,
}

/// A single execution of a workflow.
#[derive(Debug, Clone)]
pub struct WorkflowRunRecord {
    /// Unique run identifier.
    pub id: Uuid,
    /// The workflow definition that was executed.
    pub workflow_id: Uuid,
    /// Current execution status of this run.
    pub status: RunStatus,
    /// Raw event ID bytes that triggered this run, if any.
    pub trigger_event_id: Option<Vec<u8>>,
    /// Index of the step currently executing (0-based).
    pub current_step: i32,
    /// JSON execution trace -- one entry per completed step.
    pub execution_trace: serde_json::Value,
    /// Serialized `TriggerContext` captured at workflow start.
    /// NULL for runs created before this column was added (backwards-compatible).
    pub trigger_context: Option<serde_json::Value>,
    /// When execution began.
    pub started_at: Option<DateTime<Utc>>,
    /// When execution finished (success or failure).
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message if the run failed.
    pub error_message: Option<String>,
    /// When the run record was created.
    pub created_at: DateTime<Utc>,
}

/// A winning scheduled workflow fire claim.
///
/// The primary identity is `(workflow_id, scheduled_for)`. `community_id` is
/// resolved from the workflow row inside the claim SQL and returned for scoped
/// audit/logging; callers never supply it as a claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledWorkflowFireClaim {
    /// Community that owns this scheduled fire.
    pub community_id: CommunityId,
    /// Workflow definition that should run.
    pub workflow_id: Uuid,
    /// Authoritative schedule instant this claim represents.
    pub scheduled_for: DateTime<Utc>,
    /// Database timestamp for when this pod won the claim.
    pub claimed_at: DateTime<Utc>,
}

/// A pending or resolved approval gate for a workflow step.
#[derive(Debug, Clone)]
pub struct ApprovalRecord {
    /// Token hash as stored in the DB (BYTEA).
    pub token: Vec<u8>,
    /// The workflow this approval belongs to.
    pub workflow_id: Uuid,
    /// The run waiting on this approval.
    pub run_id: Uuid,
    /// The step ID that requested approval.
    pub step_id: String,
    /// Zero-based index of the step in the workflow.
    pub step_index: i32,
    /// Who may approve (user mention or role spec).
    pub approver_spec: String,
    /// Current status of this approval request.
    pub status: ApprovalStatus,
    /// Compressed public key bytes of the user who acted on this approval.
    pub approver_pubkey: Option<Vec<u8>>,
    /// Optional note left by the approver.
    pub note: Option<String>,
    /// When this approval request expires.
    pub expires_at: DateTime<Utc>,
    /// When the approval record was created.
    pub created_at: DateTime<Utc>,
}

// -- Workflow CRUD ------------------------------------------------------------

/// Insert a new workflow record. Returns the new workflow's UUID.
/// New workflows start as `active` and `enabled = TRUE`.
pub async fn create_workflow(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Option<Uuid>,
    owner_pubkey: &[u8],
    name: &str,
    definition_json: &str,
    definition_hash: &[u8],
) -> Result<Uuid> {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO workflows
            (id, community_id, name, owner_pubkey, channel_id, definition, definition_hash, status, enabled)
        VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, 'active', TRUE)
        "#,
    )
    .bind(id)
    .bind(community_id.as_uuid())
    .bind(name)
    .bind(owner_pubkey)
    .bind(channel_id)
    .bind(definition_json)
    .bind(definition_hash)
    .execute(pool)
    .await?;

    Ok(id)
}

/// Fetch a single workflow by ID. Returns `DbError::InvalidData` if missing.
pub async fn get_workflow(pool: &PgPool, id: Uuid) -> Result<WorkflowRecord> {
    let row = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("workflow {id}")))?;

    row_to_workflow_record(row)
}

/// List workflows for a channel, ordered newest first.
///
/// `limit` is capped at [`LIST_MAX_LIMIT`]. Pass `None` to use [`LIST_DEFAULT_LIMIT`].
/// `offset` enables pagination (0-based row offset).
pub async fn list_channel_workflows(
    pool: &PgPool,
    channel_id: Uuid,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<WorkflowRecord>> {
    let limit = limit.unwrap_or(LIST_DEFAULT_LIMIT).clamp(1, LIST_MAX_LIMIT);
    let offset = offset.unwrap_or(0).max(0);

    let rows = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE channel_id = $1
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(channel_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_workflow_record).collect()
}

/// List active, enabled workflows for a channel.
/// Used by the trigger-matching path to find workflows that should fire.
/// Only returns workflows with status = 'active' AND enabled = TRUE.
///
/// Bounded to [`LIST_MAX_LIMIT`] rows -- the trigger path should not process
/// an unbounded number of workflows per event.
pub async fn list_enabled_channel_workflows(
    pool: &PgPool,
    channel_id: Uuid,
) -> Result<Vec<WorkflowRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE channel_id = $1
          AND status = 'active'
          AND enabled = TRUE
        ORDER BY created_at DESC
        LIMIT $2
        "#,
    )
    .bind(channel_id)
    .bind(LIST_MAX_LIMIT)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_workflow_record).collect()
}

/// List all active, enabled workflows with a `schedule` trigger across all channels.
///
/// Used by the cron scheduler. Filters by trigger type in SQL to avoid loading
/// event-triggered workflows that the cron loop would immediately discard.
/// Results are bounded to [`LIST_MAX_LIMIT`] rows.
pub async fn list_all_enabled_workflows(pool: &PgPool) -> Result<Vec<WorkflowRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE status = 'active'
          AND enabled = TRUE
          AND definition->'trigger'->>'on' = 'schedule'
        ORDER BY created_at ASC
        LIMIT $1
        "#,
    )
    .bind(LIST_MAX_LIMIT)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_workflow_record).collect()
}

/// Claim a scheduled workflow fire for an authoritative schedule instant.
///
/// Returns `Some` only for the first pod that claims `(workflow_id,
/// scheduled_for)`. All other pods receive `None` and must skip creating a
/// workflow run. The `scheduled_for` value must come from an external
/// schedule anchor (cron expression) or DB-authoritative interval anchor; a
/// per-pod in-memory timestamp is not safe because different pods can compute
/// different claim keys.
pub async fn claim_scheduled_workflow_fire(
    pool: &PgPool,
    workflow_id: Uuid,
    scheduled_for: DateTime<Utc>,
) -> Result<Option<ScheduledWorkflowFireClaim>> {
    let row = sqlx::query(
        r#"
        INSERT INTO scheduled_workflow_fires (community_id, workflow_id, scheduled_for)
        SELECT w.community_id, w.id, $2
        FROM workflows w
        WHERE w.id = $1
        ON CONFLICT (community_id, workflow_id, scheduled_for) DO NOTHING
        RETURNING community_id, workflow_id, scheduled_for, claimed_at
        "#,
    )
    .bind(workflow_id)
    .bind(scheduled_for)
    .fetch_optional(pool)
    .await?;

    row.map(|row| {
        let community_id: Uuid = row.try_get("community_id")?;
        Ok(ScheduledWorkflowFireClaim {
            community_id: CommunityId::from_uuid(community_id),
            workflow_id: row.try_get("workflow_id")?,
            scheduled_for: row.try_get("scheduled_for")?,
            claimed_at: row.try_get("claimed_at")?,
        })
    })
    .transpose()
}

/// Fetch the greatest claimed schedule instant for a workflow.
///
/// Interval schedulers use this as their DB-authoritative `last_fired` anchor.
/// It makes all pods compute the same next interval instant after a successful
/// claim, and preserves the interval clock across pod restarts. This intentionally
/// reads from `scheduled_workflow_fires`, not `workflow_runs`, because the claim
/// row is the source of truth for schedule deduplication.
pub async fn latest_scheduled_workflow_fire(
    pool: &PgPool,
    workflow_id: Uuid,
) -> Result<Option<DateTime<Utc>>> {
    let row = sqlx::query(
        r#"
        SELECT MAX(scheduled_for) AS scheduled_for
        FROM scheduled_workflow_fires
        WHERE workflow_id = $1
        "#,
    )
    .bind(workflow_id)
    .fetch_one(pool)
    .await?;

    row.try_get("scheduled_for").map_err(Into::into)
}

/// Link a won scheduled-fire claim to the workflow run it created.
///
/// This is for ops/audit forensics only; the claim row remains the dedupe
/// boundary. If run creation succeeds, callers should attach the run id before
/// spawning execution. If run creation fails, leaving `workflow_run_id` NULL is
/// intentional: the schedule instant was claimed and must not duplicate later.
pub async fn attach_scheduled_workflow_run(
    pool: &PgPool,
    workflow_id: Uuid,
    scheduled_for: DateTime<Utc>,
    workflow_run_id: Uuid,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE scheduled_workflow_fires
        SET workflow_run_id = $3
        WHERE workflow_id = $1
          AND scheduled_for = $2
          AND workflow_run_id IS NULL
        "#,
    )
    .bind(workflow_id)
    .bind(scheduled_for)
    .bind(workflow_run_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

/// Delete old scheduled workflow fire claims for retention.
///
/// Schedule claim rows are correctness metadata, but they grow with every fire.
/// The relay/ops janitor should retain enough history for audits and interval
/// anchoring: the cutoff must be older than the largest interval schedule the
/// deployment supports, or interval workflows can lose their DB-authoritative
/// anchor after pruning.
pub async fn prune_scheduled_workflow_fires_before(
    pool: &PgPool,
    older_than: DateTime<Utc>,
) -> Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM scheduled_workflow_fires
        WHERE claimed_at < $1
        "#,
    )
    .bind(older_than)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Update a workflow's name, definition, and definition_hash.
pub async fn update_workflow(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    definition_json: &str,
    definition_hash: &[u8],
) -> Result<()> {
    let affected = sqlx::query(
        r#"
        UPDATE workflows
        SET name = $1, definition = $2::jsonb, definition_hash = $3
        WHERE id = $4
        "#,
    )
    .bind(name)
    .bind(definition_json)
    .bind(definition_hash)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow {id}")));
    }
    Ok(())
}

/// Update a workflow's status (active -> disabled -> archived).
pub async fn update_workflow_status(pool: &PgPool, id: Uuid, status: WorkflowStatus) -> Result<()> {
    let affected = sqlx::query(
        r#"
        UPDATE workflows
        SET status = $1::workflow_status
        WHERE id = $2
        "#,
    )
    .bind(status.to_string())
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow {id}")));
    }
    Ok(())
}

/// Enable or disable a workflow without changing its status.
pub async fn set_workflow_enabled(pool: &PgPool, id: Uuid, enabled: bool) -> Result<()> {
    let affected = sqlx::query(
        r#"
        UPDATE workflows
        SET enabled = $1
        WHERE id = $2
        "#,
    )
    .bind(enabled)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow {id}")));
    }
    Ok(())
}

/// Delete a workflow and all its runs/approvals (CASCADE).
pub async fn delete_workflow(pool: &PgPool, id: Uuid) -> Result<()> {
    let affected = sqlx::query("DELETE FROM workflows WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow {id}")));
    }
    Ok(())
}

// -- Workflow Run CRUD --------------------------------------------------------

/// Insert a new workflow run. Returns the new run's UUID.
///
/// `trigger_context` is the serialized `TriggerContext` for this run. It is stored
/// so that post-approval resume steps can restore the original trigger data and
/// correctly resolve `{{trigger.*}}` template variables.
pub async fn create_workflow_run(
    pool: &PgPool,
    workflow_id: Uuid,
    trigger_event_id: Option<&[u8]>,
    trigger_context: Option<&serde_json::Value>,
) -> Result<Uuid> {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO workflow_runs
            (id, workflow_id, status, trigger_event_id, current_step, execution_trace, trigger_context)
        VALUES ($1, $2, 'pending', $3, 0, '[]', $4)
        "#,
    )
    .bind(id)
    .bind(workflow_id)
    .bind(trigger_event_id)
    .bind(trigger_context)
    .execute(pool)
    .await?;

    Ok(id)
}

/// Fetch a single workflow run by ID.
pub async fn get_workflow_run(pool: &PgPool, id: Uuid) -> Result<WorkflowRunRecord> {
    let row = sqlx::query(
        r#"
        SELECT id, workflow_id, status::text AS status, trigger_event_id, current_step,
               execution_trace, trigger_context, started_at, completed_at, error_message, created_at
        FROM workflow_runs
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("workflow_run {id}")))?;

    row_to_run_record(row)
}

/// List runs for a workflow, newest first, up to `limit` rows.
pub async fn list_workflow_runs(
    pool: &PgPool,
    workflow_id: Uuid,
    limit: i64,
) -> Result<Vec<WorkflowRunRecord>> {
    let limit = limit.min(1000);
    let rows = sqlx::query(
        r#"
        SELECT id, workflow_id, status::text AS status, trigger_event_id, current_step,
               execution_trace, trigger_context, started_at, completed_at, error_message, created_at
        FROM workflow_runs
        WHERE workflow_id = $1
        ORDER BY created_at DESC
        LIMIT $2
        "#,
    )
    .bind(workflow_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_run_record).collect()
}

/// Update run status, current step, execution trace, and optional error message.
///
/// Fix C3: `started_at` is set when the NEW status is 'running' and `started_at`
/// has not yet been stamped (IS NULL). The original code read `status` from the
/// column AFTER `SET status = ?` had already changed it, so the condition was
/// always false. We now check the bind parameter directly.
pub async fn update_workflow_run(
    pool: &PgPool,
    id: Uuid,
    status: RunStatus,
    current_step: i32,
    trace: &serde_json::Value,
    error: Option<&str>,
) -> Result<()> {
    let status_str = status.to_string();
    let affected = sqlx::query(
        r#"
        UPDATE workflow_runs
        SET status        = $1::run_status,
            current_step  = $2,
            execution_trace = $3,
            error_message = $4,
            started_at    = CASE WHEN $5 = 'running' AND started_at IS NULL
                                 THEN NOW() ELSE started_at END,
            completed_at  = CASE WHEN $6 IN ('completed','failed','cancelled')
                                 THEN NOW() ELSE completed_at END
        WHERE id = $7
        "#,
    )
    .bind(&status_str)
    .bind(current_step)
    .bind(trace)
    .bind(error)
    .bind(&status_str) // for started_at CASE
    .bind(&status_str) // for completed_at CASE
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow_run {id}")));
    }
    Ok(())
}

// -- Approval CRUD ------------------------------------------------------------

/// Parameters for creating a new approval request.
pub struct CreateApprovalParams<'a> {
    /// Raw approval token (will be hashed before storage).
    pub token: &'a str,
    /// The workflow this approval belongs to.
    pub workflow_id: Uuid,
    /// The run waiting on this approval.
    pub run_id: Uuid,
    /// The step ID that requested approval.
    pub step_id: &'a str,
    /// Zero-based index of the step in the workflow.
    pub step_index: i32,
    /// Who may approve (user mention or role spec).
    pub approver_spec: &'a str,
    /// When this approval request expires.
    pub expires_at: DateTime<Utc>,
}

/// Insert a new approval request.
///
/// The `token` parameter is the raw (plaintext) token. It is hashed with
/// SHA-256 before storage so the DB never holds the raw value.
pub async fn create_approval(pool: &PgPool, params: CreateApprovalParams<'_>) -> Result<()> {
    let CreateApprovalParams {
        token,
        workflow_id,
        run_id,
        step_id,
        step_index,
        approver_spec,
        expires_at,
    } = params;
    let token_hash = hash_approval_token(token);

    sqlx::query(
        r#"
        INSERT INTO workflow_approvals
            (token, workflow_id, run_id, step_id, step_index, approver_spec, status, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7)
        "#,
    )
    .bind(token_hash)
    .bind(workflow_id)
    .bind(run_id)
    .bind(step_id)
    .bind(step_index)
    .bind(approver_spec)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// Fetch an approval record by raw token.
///
/// The token is hashed before the DB lookup so plaintext tokens are never
/// sent to the database layer.
pub async fn get_approval(pool: &PgPool, token: &str) -> Result<ApprovalRecord> {
    let token_hash = hash_approval_token(token);
    get_approval_by_stored_hash(pool, &token_hash).await
}

/// Fetch an approval record by its already-hashed token value.
///
/// Use this when you already have the hash stored in the DB (e.g., from
/// `get_run_approvals`). The `token_hash` is used directly without re-hashing.
pub async fn get_approval_by_stored_hash(
    pool: &PgPool,
    token_hash: &[u8],
) -> Result<ApprovalRecord> {
    let row = sqlx::query(
        r#"
        SELECT token, workflow_id, run_id, step_id, step_index, approver_spec,
               status::text AS status, approver_pubkey, note, expires_at, created_at
        FROM workflow_approvals
        WHERE token = $1
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DbError::NotFound("approval token (hashed)".to_string()))?;

    row_to_approval_record(row)
}

/// Fetch all approval records for a given workflow run.
pub async fn get_run_approvals(
    pool: &PgPool,
    workflow_id: Uuid,
    run_id: Uuid,
) -> Result<Vec<ApprovalRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT token, workflow_id, run_id, step_id, step_index, approver_spec,
               status::text AS status, approver_pubkey, note, expires_at, created_at
        FROM workflow_approvals
        WHERE run_id = $1 AND workflow_id = $2
        ORDER BY step_index, created_at
        "#,
    )
    .bind(run_id)
    .bind(workflow_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_approval_record).collect()
}

/// Update an approval's status, approver pubkey, and optional note.
/// Also stamps `granted_at` or `denied_at` based on the new status.
///
/// The `token` parameter is the raw (plaintext) token; it is hashed before
/// the WHERE lookup.
///
/// # TOCTOU safety (N5)
/// The WHERE clause includes `AND status = 'pending'` so that two concurrent
/// grant/deny requests cannot both succeed. If the approval was already acted
/// on (status != 'pending'), the UPDATE touches 0 rows and this function
/// returns `Ok(false)`. Callers should treat `false` as a conflict (HTTP 409).
pub async fn update_approval(
    pool: &PgPool,
    token: &str,
    status: ApprovalStatus,
    approver_pubkey: Option<&[u8]>,
    note: Option<&str>,
) -> Result<bool> {
    let token_hash = hash_approval_token(token);
    update_approval_by_stored_hash(pool, &token_hash, status, approver_pubkey, note).await
}

/// Update an approval by its already-hashed token value.
///
/// Use this when you already have the hash stored in the DB (e.g., from
/// `get_run_approvals`). The `token_hash` is used directly without re-hashing.
///
/// See [`update_approval`] for TOCTOU safety notes.
pub async fn update_approval_by_stored_hash(
    pool: &PgPool,
    token_hash: &[u8],
    status: ApprovalStatus,
    approver_pubkey: Option<&[u8]>,
    note: Option<&str>,
) -> Result<bool> {
    let status_str = status.to_string();
    let affected = sqlx::query(
        r#"
        UPDATE workflow_approvals
        SET status          = $1::approval_status,
            approver_pubkey = $2,
            note            = $3,
            granted_at      = CASE WHEN $4 = 'granted' THEN NOW() ELSE granted_at END,
            denied_at       = CASE WHEN $5 = 'denied'  THEN NOW() ELSE denied_at  END
        WHERE token = $6 AND status = 'pending'
        "#,
    )
    .bind(&status_str)
    .bind(approver_pubkey)
    .bind(note)
    .bind(&status_str) // for granted_at CASE
    .bind(&status_str) // for denied_at CASE
    .bind(token_hash)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(affected > 0)
}

// -- Row mappers --------------------------------------------------------------

fn row_to_workflow_record(row: sqlx::postgres::PgRow) -> Result<WorkflowRecord> {
    let id: Uuid = row.try_get("id")?;
    let channel_id: Option<Uuid> = row.try_get("channel_id")?;

    let status_str: String = row.try_get("status")?;
    let status = status_str.parse::<WorkflowStatus>()?;

    let enabled: bool = row.try_get("enabled")?;

    let community_id: Uuid = row.try_get("community_id")?;

    Ok(WorkflowRecord {
        id,
        community_id: CommunityId::from_uuid(community_id),
        name: row.try_get("name")?,
        owner_pubkey: row.try_get("owner_pubkey")?,
        channel_id,
        definition: row.try_get("definition")?,
        definition_hash: row.try_get("definition_hash")?,
        status,
        enabled,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn row_to_run_record(row: sqlx::postgres::PgRow) -> Result<WorkflowRunRecord> {
    let id: Uuid = row.try_get("id")?;
    let workflow_id: Uuid = row.try_get("workflow_id")?;

    let status_str: String = row.try_get("status")?;
    let status = status_str.parse::<RunStatus>()?;

    Ok(WorkflowRunRecord {
        id,
        workflow_id,
        status,
        trigger_event_id: row.try_get("trigger_event_id")?,
        current_step: row.try_get("current_step")?,
        execution_trace: row.try_get("execution_trace")?,
        trigger_context: row.try_get("trigger_context")?,
        started_at: row.try_get("started_at")?,
        completed_at: row.try_get("completed_at")?,
        error_message: row.try_get("error_message")?,
        created_at: row.try_get("created_at")?,
    })
}

fn row_to_approval_record(row: sqlx::postgres::PgRow) -> Result<ApprovalRecord> {
    let workflow_id: Uuid = row.try_get("workflow_id")?;
    let run_id: Uuid = row.try_get("run_id")?;

    let status_str: String = row.try_get("status")?;
    let status = status_str.parse::<ApprovalStatus>()?;

    Ok(ApprovalRecord {
        token: row.try_get("token")?,
        workflow_id,
        run_id,
        step_id: row.try_get("step_id")?,
        step_index: row.try_get("step_index")?,
        approver_spec: row.try_get("approver_spec")?,
        status,
        approver_pubkey: row.try_get("approver_pubkey")?,
        note: row.try_get("note")?,
        expires_at: row.try_get("expires_at")?,
        created_at: row.try_get("created_at")?,
    })
}

/// Find a workflow by owner pubkey and name. Returns the first match (active or not).
pub async fn find_by_owner_and_name(
    pool: &PgPool,
    owner_pubkey: &[u8],
    name: &str,
) -> Result<Option<WorkflowRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE owner_pubkey = $1 AND name = $2
        LIMIT 1
        "#,
    )
    .bind(owner_pubkey)
    .bind(name)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(row_to_workflow_record(r)?)),
        None => Ok(None),
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // -- WorkflowStatus enum --------------------------------------------------

    #[test]
    fn workflow_status_display_is_lowercase() {
        assert_eq!(WorkflowStatus::Active.to_string(), "active");
        assert_eq!(WorkflowStatus::Disabled.to_string(), "disabled");
        assert_eq!(WorkflowStatus::Archived.to_string(), "archived");
    }

    #[test]
    fn workflow_status_from_str_round_trips() {
        for s in &["active", "disabled", "archived"] {
            let status: WorkflowStatus = s.parse().expect("parse");
            assert_eq!(status.to_string(), *s);
        }
    }

    #[test]
    fn workflow_status_from_str_rejects_unknown() {
        let err = "pending".parse::<WorkflowStatus>().unwrap_err();
        assert!(matches!(err, DbError::InvalidData(_)));
    }

    #[test]
    fn workflow_status_equality() {
        assert_eq!(WorkflowStatus::Active, WorkflowStatus::Active);
        assert_ne!(WorkflowStatus::Active, WorkflowStatus::Disabled);
    }

    // -- RunStatus enum -------------------------------------------------------

    #[test]
    fn run_status_display_is_lowercase() {
        assert_eq!(RunStatus::Pending.to_string(), "pending");
        assert_eq!(RunStatus::Running.to_string(), "running");
        assert_eq!(RunStatus::WaitingApproval.to_string(), "waiting_approval");
        assert_eq!(RunStatus::Completed.to_string(), "completed");
        assert_eq!(RunStatus::Failed.to_string(), "failed");
        assert_eq!(RunStatus::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn run_status_from_str_round_trips() {
        for s in &[
            "pending",
            "running",
            "waiting_approval",
            "completed",
            "failed",
            "cancelled",
        ] {
            let status: RunStatus = s.parse().expect("parse");
            assert_eq!(status.to_string(), *s);
        }
    }

    #[test]
    fn run_status_from_str_rejects_unknown() {
        let err = "active".parse::<RunStatus>().unwrap_err();
        assert!(matches!(err, DbError::InvalidData(_)));
    }

    // -- ApprovalStatus enum --------------------------------------------------

    #[test]
    fn approval_status_display_is_lowercase() {
        assert_eq!(ApprovalStatus::Pending.to_string(), "pending");
        assert_eq!(ApprovalStatus::Granted.to_string(), "granted");
        assert_eq!(ApprovalStatus::Denied.to_string(), "denied");
        assert_eq!(ApprovalStatus::Expired.to_string(), "expired");
    }

    #[test]
    fn approval_status_from_str_round_trips() {
        for s in &["pending", "granted", "denied", "expired"] {
            let status: ApprovalStatus = s.parse().expect("parse");
            assert_eq!(status.to_string(), *s);
        }
    }

    #[test]
    fn approval_status_from_str_rejects_unknown() {
        let err = "approved".parse::<ApprovalStatus>().unwrap_err();
        assert!(matches!(err, DbError::InvalidData(_)));
    }

    // -- WorkflowRecord -------------------------------------------------------

    #[test]
    fn workflow_record_fields_are_accessible() {
        let id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let now = Utc::now();
        let def = serde_json::json!({
            "name": "My Workflow",
            "trigger": { "on": "message_posted" },
            "steps": [{ "id": "s1", "action": "send_message", "text": "hi" }]
        });

        let community_id = CommunityId::from_uuid(Uuid::new_v4());

        let record = WorkflowRecord {
            id,
            community_id,
            name: "My Workflow".to_owned(),
            owner_pubkey: vec![0xab; 32],
            channel_id: Some(channel_id),
            definition: def.clone(),
            definition_hash: vec![0x01, 0x02, 0x03, 0x04],
            status: WorkflowStatus::Active,
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        assert_eq!(record.id, id);
        assert_eq!(record.community_id, community_id);
        assert_eq!(record.name, "My Workflow");
        assert_eq!(record.owner_pubkey, vec![0xab; 32]);
        assert_eq!(record.channel_id, Some(channel_id));
        assert_eq!(record.definition, def);
        assert_eq!(record.definition_hash, vec![0x01, 0x02, 0x03, 0x04]);
        assert_eq!(record.status, WorkflowStatus::Active);
        assert!(record.enabled);
    }

    #[test]
    fn workflow_record_channel_id_can_be_none() {
        let id = Uuid::new_v4();
        let now = Utc::now();

        let record = WorkflowRecord {
            id,
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            name: "Global Workflow".to_owned(),
            owner_pubkey: vec![0x00; 32],
            channel_id: None,
            definition: serde_json::json!({}),
            definition_hash: vec![],
            status: WorkflowStatus::Active,
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        assert!(record.channel_id.is_none());
    }

    #[test]
    fn workflow_record_clone_is_independent() {
        let id = Uuid::new_v4();
        let now = Utc::now();

        let record = WorkflowRecord {
            id,
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            name: "Original".to_owned(),
            owner_pubkey: vec![0x01; 32],
            channel_id: None,
            definition: serde_json::json!({}),
            definition_hash: vec![0xAA],
            status: WorkflowStatus::Active,
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        let mut cloned = record.clone();
        cloned.name = "Cloned".to_owned();

        assert_eq!(record.name, "Original");
        assert_eq!(cloned.name, "Cloned");
    }

    #[test]
    fn workflow_record_status_variants() {
        let now = Utc::now();
        for status in &[
            WorkflowStatus::Active,
            WorkflowStatus::Disabled,
            WorkflowStatus::Archived,
        ] {
            let record = WorkflowRecord {
                id: Uuid::new_v4(),
                community_id: CommunityId::from_uuid(Uuid::new_v4()),
                name: "Test".to_owned(),
                owner_pubkey: vec![],
                channel_id: None,
                definition: serde_json::json!({}),
                definition_hash: vec![],
                status: status.clone(),
                enabled: true,
                created_at: now,
                updated_at: now,
            };
            assert_eq!(&record.status, status);
        }
    }

    #[test]
    fn workflow_record_disabled_has_enabled_false() {
        let now = Utc::now();
        let record = WorkflowRecord {
            id: Uuid::new_v4(),
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            name: "Paused".to_owned(),
            owner_pubkey: vec![],
            channel_id: None,
            definition: serde_json::json!({}),
            definition_hash: vec![],
            status: WorkflowStatus::Active,
            enabled: false,
            created_at: now,
            updated_at: now,
        };
        assert!(!record.enabled);
        assert_eq!(record.status, WorkflowStatus::Active);
    }

    // -- WorkflowRunRecord ----------------------------------------------------

    #[test]
    fn workflow_run_record_fields_are_accessible() {
        let id = Uuid::new_v4();
        let workflow_id = Uuid::new_v4();
        let now = Utc::now();
        let trigger_event_id = vec![0xde, 0xad, 0xbe, 0xef];

        let record = WorkflowRunRecord {
            id,
            workflow_id,
            status: RunStatus::Running,
            trigger_event_id: Some(trigger_event_id.clone()),
            current_step: 2,
            execution_trace: serde_json::json!([
                { "step": "s1", "status": "completed" }
            ]),
            trigger_context: None,
            started_at: Some(now),
            completed_at: None,
            error_message: None,
            created_at: now,
        };

        assert_eq!(record.id, id);
        assert_eq!(record.workflow_id, workflow_id);
        assert_eq!(record.status, RunStatus::Running);
        assert_eq!(record.trigger_event_id, Some(trigger_event_id));
        assert_eq!(record.current_step, 2);
        assert!(record.started_at.is_some());
        assert!(record.completed_at.is_none());
        assert!(record.error_message.is_none());
    }

    #[test]
    fn workflow_run_record_no_trigger_event() {
        let now = Utc::now();
        let record = WorkflowRunRecord {
            id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            status: RunStatus::Pending,
            trigger_event_id: None,
            current_step: 0,
            execution_trace: serde_json::json!([]),
            trigger_context: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            created_at: now,
        };

        assert!(record.trigger_event_id.is_none());
        assert_eq!(record.current_step, 0);
        assert!(record.started_at.is_none());
    }

    #[test]
    fn workflow_run_record_failed_with_error_message() {
        let now = Utc::now();
        let record = WorkflowRunRecord {
            id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            status: RunStatus::Failed,
            trigger_event_id: None,
            current_step: 1,
            execution_trace: serde_json::json!([]),
            trigger_context: None,
            started_at: Some(now),
            completed_at: Some(now),
            error_message: Some("step timeout exceeded".to_owned()),
            created_at: now,
        };

        assert_eq!(record.status, RunStatus::Failed);
        assert!(record.completed_at.is_some());
        assert_eq!(
            record.error_message.as_deref(),
            Some("step timeout exceeded")
        );
    }

    #[test]
    fn workflow_run_record_execution_trace_is_json_array() {
        let now = Utc::now();
        let trace = serde_json::json!([
            { "step_id": "notify", "status": "completed", "output": { "sent": true } },
            { "step_id": "log", "status": "skipped" }
        ]);

        let record = WorkflowRunRecord {
            id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            status: RunStatus::Completed,
            trigger_event_id: None,
            current_step: 2,
            execution_trace: trace.clone(),
            trigger_context: None,
            started_at: Some(now),
            completed_at: Some(now),
            error_message: None,
            created_at: now,
        };

        assert!(record.execution_trace.is_array());
        assert_eq!(record.execution_trace.as_array().unwrap().len(), 2);
    }

    #[test]
    fn workflow_run_record_clone_is_independent() {
        let now = Utc::now();
        let record = WorkflowRunRecord {
            id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            status: RunStatus::Pending,
            trigger_event_id: None,
            current_step: 0,
            execution_trace: serde_json::json!([]),
            trigger_context: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            created_at: now,
        };

        let mut cloned = record.clone();
        cloned.status = RunStatus::Running;

        assert_eq!(record.status, RunStatus::Pending);
        assert_eq!(cloned.status, RunStatus::Running);
    }

    // -- ApprovalRecord -------------------------------------------------------

    #[test]
    fn approval_record_fields_are_accessible() {
        let workflow_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let expires_at = Utc.with_ymd_and_hms(2026, 12, 31, 23, 59, 59).unwrap();
        let now = Utc::now();

        let record = ApprovalRecord {
            token: b"abc123def456abc123def456abc123de".to_vec(),
            workflow_id,
            run_id,
            step_id: "request_approval".to_owned(),
            step_index: 1,
            approver_spec: "@engineering-lead".to_owned(),
            status: ApprovalStatus::Pending,
            approver_pubkey: None,
            note: None,
            expires_at,
            created_at: now,
        };

        assert_eq!(record.token, b"abc123def456abc123def456abc123de");
        assert_eq!(record.workflow_id, workflow_id);
        assert_eq!(record.run_id, run_id);
        assert_eq!(record.step_id, "request_approval");
        assert_eq!(record.step_index, 1);
        assert_eq!(record.approver_spec, "@engineering-lead");
        assert_eq!(record.status, ApprovalStatus::Pending);
        assert!(record.approver_pubkey.is_none());
        assert!(record.note.is_none());
    }

    #[test]
    fn approval_record_granted_with_pubkey_and_note() {
        let now = Utc::now();
        let approver_pubkey = vec![0xca; 32];

        let record = ApprovalRecord {
            token: b"token-granted".to_vec(),
            workflow_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            step_id: "gate".to_owned(),
            step_index: 0,
            approver_spec: "@manager".to_owned(),
            status: ApprovalStatus::Granted,
            approver_pubkey: Some(approver_pubkey.clone()),
            note: Some("Looks good, approved.".to_owned()),
            expires_at: now,
            created_at: now,
        };

        assert_eq!(record.status, ApprovalStatus::Granted);
        assert_eq!(record.approver_pubkey, Some(approver_pubkey));
        assert_eq!(record.note.as_deref(), Some("Looks good, approved."));
    }

    #[test]
    fn approval_record_denied_with_note() {
        let now = Utc::now();

        let record = ApprovalRecord {
            token: b"token-denied".to_vec(),
            workflow_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            step_id: "gate".to_owned(),
            step_index: 0,
            approver_spec: "@manager".to_owned(),
            status: ApprovalStatus::Denied,
            approver_pubkey: Some(vec![0xbb; 32]),
            note: Some("Not ready for production.".to_owned()),
            expires_at: now,
            created_at: now,
        };

        assert_eq!(record.status, ApprovalStatus::Denied);
        assert!(record.note.is_some());
    }

    #[test]
    fn approval_record_clone_is_independent() {
        let now = Utc::now();
        let record = ApprovalRecord {
            token: b"original-token".to_vec(),
            workflow_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            step_id: "gate".to_owned(),
            step_index: 0,
            approver_spec: "@lead".to_owned(),
            status: ApprovalStatus::Pending,
            approver_pubkey: None,
            note: None,
            expires_at: now,
            created_at: now,
        };

        let mut cloned = record.clone();
        cloned.status = ApprovalStatus::Granted;

        assert_eq!(record.status, ApprovalStatus::Pending);
        assert_eq!(cloned.status, ApprovalStatus::Granted);
    }

    // -- F1 / S1 attack surface for scheduled workflow claims ------------------
    //
    // These tests pin the locked spec from Eva [13] / Mari [12]:
    //
    //   1. `workflows.community_id` is row-owned, NOT NULL, immutable.
    //   2. Claim resolves `community_id` server-side via `workflow_id`.
    //   3. Claim uniqueness is `(workflow_id, scheduled_for)` — `workflow_id`
    //      is globally unique, so adding `community_id` to the key weakens it.
    //   4. `latest_scheduled_workflow_fire` drops caller-supplied community.
    //
    // Until that lands, `claim_for_workflow_in_other_community_no_ops` is the
    // S1 regression lock: today the schema permits a caller in community A to
    // claim a workflow owned by community B and have `claimed.community_id`
    // come back as A. That is a cross-tenant write surface (Theorem S1,
    // `docs/multi-tenant-relay.md:248`: "the claimed community never appears
    // in this function — only the resolved one").
    //
    // The other two tests are characterization guards: same-window race must
    // yield exactly one claim winner, and the retention primitive's docstring
    // caveat (pruning below the largest interval breaks `latest_*`) is
    // load-bearing for the §5c deployment-config rule Sami flagged.

    use crate::user::ensure_user;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());

        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    /// Insert a community with a unique host. Returns its `CommunityId`.
    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&host)
            .execute(pool)
            .await
            .expect("insert community");
        CommunityId::from_uuid(id)
    }

    /// Insert a channel under a community. Returns the channel id.
    async fn make_channel(pool: &PgPool, community: CommunityId, owner: &[u8]) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO channels (id, community_id, name, created_by)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(id)
        .bind(community.as_uuid())
        .bind(format!("ch-{}", id.simple()))
        .bind(owner)
        .execute(pool)
        .await
        .expect("insert channel");
        id
    }

    /// Insert a workflow whose tenant is `community`'s channel. Returns the
    /// workflow id and the owning community for callers that want to assert
    /// the resolved tenant.
    async fn make_workflow_in(pool: &PgPool, community: CommunityId) -> (Uuid, CommunityId) {
        let owner = vec![0xa1; 32];
        ensure_user(pool, community, &owner)
            .await
            .expect("ensure owner");
        let channel_id = make_channel(pool, community, &owner).await;
        let workflow_id = create_workflow(
            pool,
            community,
            Some(channel_id),
            &owner,
            "f1-attack-workflow",
            r#"{"trigger":{"on":"schedule"},"steps":[]}"#,
            &[0u8; 32],
        )
        .await
        .expect("create workflow");
        (workflow_id, community)
    }

    /// F1 attack: a caller in community A must NOT be able to claim a fire
    /// for a workflow owned by community B and have the claim resolve under
    /// A's tenant. The resolved community on the returned claim row MUST
    /// equal the workflow's actual tenant (B).
    ///
    /// Post-fix (`1fa3d837f`) the claim signature no longer accepts a caller
    /// tenant — the SQL resolves `community_id` from the `workflows` row via
    /// `INSERT ... SELECT w.community_id, w.id, $2 FROM workflows w WHERE
    /// w.id = $1`. This test still creates an "attacker_community" that the
    /// caller is *not* permitted to name on the wire; the assertion is that
    /// the resolved tenant equals the workflow owner's community, never the
    /// attacker's. With the pre-fix signature this test was RED (the row's
    /// `community_id` came back as the attacker's); under the locked spec it
    /// must be GREEN.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_for_workflow_in_other_community_no_ops() {
        let pool = setup_pool().await;

        // The attacker community exists in the schema but the caller has no
        // way to pass it to the claim API anymore — that *is* the S1 fix.
        // Keeping the row in this test makes the no-influence invariant
        // explicit: even with two real tenants in play, the resolved
        // community is the workflow's owner.
        let _attacker_community = make_community(&pool).await;
        let owner_community = make_community(&pool).await;
        let (workflow_id, expected_community) = make_workflow_in(&pool, owner_community).await;

        let scheduled_for = Utc.with_ymd_and_hms(2026, 6, 27, 0, 0, 0).unwrap();

        let claim = claim_scheduled_workflow_fire(&pool, workflow_id, scheduled_for)
            .await
            .expect("claim should not error")
            .expect("claim should succeed exactly once");

        assert_eq!(
            claim.community_id,
            expected_community,
            "claim must resolve community from workflow_id (server-side); \
             resolved={resolved:?} expected={expected_community:?}",
            resolved = claim.community_id,
        );
        assert_eq!(claim.workflow_id, workflow_id);
        assert_eq!(claim.scheduled_for, scheduled_for);
    }

    /// Same `(workflow_id, scheduled_for)` claimed concurrently by N tasks
    /// must yield exactly one `Some` winner. Post-fix the PK is
    /// `(workflow_id, scheduled_for)` (`workflow_id` is globally unique,
    /// `community_id` is a scoped/audit label only) — exactly the locked
    /// spec. Characterization guard: protects the dedup boundary against
    /// regressions in the claim SQL.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_same_window_claims_exactly_one_wins() {
        let pool = setup_pool().await;

        let community = make_community(&pool).await;
        let (workflow_id, _) = make_workflow_in(&pool, community).await;
        let scheduled_for = Utc.with_ymd_and_hms(2026, 6, 27, 0, 1, 0).unwrap();

        const N: usize = 8;
        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                claim_scheduled_workflow_fire(&pool, workflow_id, scheduled_for).await
            }));
        }

        let mut winners = 0usize;
        for h in handles {
            let result = h.await.expect("task did not panic").expect("claim ok");
            if result.is_some() {
                winners += 1;
            }
        }
        assert_eq!(
            winners, 1,
            "exactly one task must win the claim race for (workflow_id, scheduled_for)"
        );
    }

    /// Documents the retention-vs-interval coupling Sami flagged for §5c:
    /// pruning every claim below the workflow's interval makes
    /// `latest_scheduled_workflow_fire` return `None`, which re-introduces the
    /// per-pod-clock anchor bug F5 was meant to fix. Test is GREEN today and
    /// MUST stay green — it pins the deployment-config rule that the janitor
    /// cutoff must exceed `MAX(interval_secs) + safety margin`. If a future
    /// change makes `latest_*` resilient to pruning (e.g. by reading the most
    /// recent workflow_run instead, or by retaining a sentinel row), this
    /// test's assertion encodes the contract that must be updated alongside.
    ///
    /// Test isolation: the prune primitive is global (filters only on
    /// `claimed_at`), so to avoid colliding with parallel claim tests we
    /// back-date this workflow's `claimed_at` into the deep past and use a
    /// past cutoff that cannot match any other test's `claimed_at = NOW()`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn latest_after_prune_below_interval_breaks_anchor() {
        let pool = setup_pool().await;

        let community = make_community(&pool).await;
        let (workflow_id, _) = make_workflow_in(&pool, community).await;
        let scheduled_for = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();

        claim_scheduled_workflow_fire(&pool, workflow_id, scheduled_for)
            .await
            .expect("claim ok")
            .expect("first claim wins");

        // Backdate this row's `claimed_at` so the global prune below targets
        // only this workflow's row and cannot race-delete other tests' rows.
        let backdated_claimed_at = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        sqlx::query(
            "UPDATE scheduled_workflow_fires SET claimed_at = $1 \
             WHERE community_id = $2 AND workflow_id = $3 AND scheduled_for = $4",
        )
        .bind(backdated_claimed_at)
        .bind(community.as_uuid())
        .bind(workflow_id)
        .bind(scheduled_for)
        .execute(&pool)
        .await
        .expect("backdate ok");

        let latest_before = latest_scheduled_workflow_fire(&pool, workflow_id)
            .await
            .expect("latest ok");
        assert_eq!(
            latest_before,
            Some(scheduled_for),
            "latest must reflect the claim before pruning",
        );

        // Janitor cutoff above only the back-dated row: prunes the anchor row
        // without touching anything claimed at wall-clock NOW.
        let cutoff = backdated_claimed_at + chrono::Duration::seconds(1);
        let pruned = prune_scheduled_workflow_fires_before(&pool, cutoff)
            .await
            .expect("prune ok");
        assert!(
            pruned >= 1,
            "expected at least one row pruned, got {pruned}"
        );

        let latest_after = latest_scheduled_workflow_fire(&pool, workflow_id)
            .await
            .expect("latest ok");
        assert_eq!(
            latest_after, None,
            "pruning below the largest interval breaks the DB anchor; \
             retention cutoff MUST exceed MAX(interval_secs) + safety margin (§5c)",
        );
    }
}
