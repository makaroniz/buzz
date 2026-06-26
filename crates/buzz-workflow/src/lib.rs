#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-workflow` — Workflow engine for Buzz.
//!
//! Channel-scoped automations with sequential execution, variable substitution,
//! conditional logic, and execution traces.
//!
//! ## Architecture
//!
//! - [`WorkflowEngine`] — top-level handle; lives in `AppState`
//! - [`schema`] — YAML/JSON definition types (`WorkflowDef`, `TriggerDef`, `ActionDef`, `Step`)
//! - [`executor`] — sequential execution, template resolution, condition evaluation
//! - [`error`] — [`WorkflowError`] enum
//!
//! ## Usage
//!
//! ```rust,ignore
//! let engine = Arc::new(WorkflowEngine::new(db, WorkflowConfig::default()));
//!
//! // Parse and validate a YAML definition.
//! let (def, json) = WorkflowEngine::parse_yaml(yaml_str)?;
//!
//! // React to an incoming event (called from event handler post-store hook).
//! engine.on_event(&stored_event).await?;
//!
//! // Run the background scheduler (cron triggers).
//! tokio::spawn(async move { engine.run().await });
//! ```

pub mod action_sink;
pub mod error;
pub mod executor;
pub mod schema;

pub use action_sink::{ActionSink, ActionSinkError};
pub use error::{PartialProgress, WorkflowError};
pub use executor::ExecutionResult;
pub use schema::{ActionDef, Step, TriggerDef, WorkflowDef};

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use buzz_core::kind::{event_kind_u32, is_workflow_execution_kind, KIND_REACTION};
use buzz_db::workflow::{RunStatus, WorkflowRecord};
use buzz_db::Db;
use chrono::{DateTime, Utc};
use tokio::sync::Semaphore;
use uuid::Uuid;

/// Runtime configuration for the workflow engine.
#[derive(Clone, Debug)]
pub struct WorkflowConfig {
    /// Maximum number of concurrently executing workflow runs. Default: 100.
    pub max_concurrent: usize,
    /// Default per-step timeout in seconds. Default: 300 (5 minutes).
    pub default_timeout_secs: u64,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 100,
            default_timeout_secs: 300,
        }
    }
}

/// The workflow engine. Clone is cheap (Arc-backed DB pool + semaphore).
pub struct WorkflowEngine {
    pub(crate) db: Db,
    pub(crate) config: WorkflowConfig,
    /// Semaphore enforcing `config.max_concurrent` simultaneous workflow runs.
    pub(crate) run_semaphore: Arc<Semaphore>,
    /// Action sink for executing side-effects (SendMessage, etc.).
    /// Late-initialized via [`set_action_sink`] after `AppState` construction.
    pub(crate) action_sink: OnceLock<Arc<dyn ActionSink>>,
}

impl WorkflowEngine {
    /// Create a new `WorkflowEngine`.
    pub fn new(db: Db, config: WorkflowConfig) -> Self {
        let permits = config.max_concurrent.max(1);
        let run_semaphore = Arc::new(Semaphore::new(permits));
        Self {
            db,
            config,
            run_semaphore,
            action_sink: OnceLock::new(),
        }
    }

    /// Set the action sink. Called once after `AppState` construction.
    ///
    /// # Panics
    /// Panics if called more than once.
    pub fn set_action_sink(&self, sink: Arc<dyn ActionSink>) {
        if self.action_sink.set(sink).is_err() {
            panic!("action_sink already initialized");
        }
    }

    /// Get the action sink reference.
    ///
    /// Returns `Err(WorkflowError)` if the sink has not been initialized via
    /// [`set_action_sink`]. This avoids a panic if the engine is used before
    /// wiring is complete.
    pub(crate) fn action_sink(&self) -> Result<&dyn ActionSink, WorkflowError> {
        self.action_sink.get().map(|s| s.as_ref()).ok_or_else(|| {
            WorkflowError::InvalidDefinition(
                "action_sink not initialized — call set_action_sink() before executing workflows"
                    .into(),
            )
        })
    }

    /// Parse and validate a YAML workflow definition.
    ///
    /// Returns `(WorkflowDef, canonical_json)` on success. The canonical JSON
    /// is suitable for storage in the `definition` column.
    pub fn parse_yaml(yaml: &str) -> Result<(WorkflowDef, String), WorkflowError> {
        schema::parse_yaml(yaml)
    }

    /// Finalize a workflow run after execution completes or fails.
    ///
    /// This is the **single** place that maps an executor result to a DB status
    /// update. All execution paths (event-triggered, manual trigger/webhook,
    /// approval resume) call this instead of duplicating the 3-way match.
    ///
    /// `existing_trace` is prepended to the executor's trace — used by the
    /// approval-resume path where pre-approval steps already have trace entries.
    pub async fn finalize_run(
        &self,
        run_id: uuid::Uuid,
        result: Result<ExecutionResult, (WorkflowError, PartialProgress)>,
        existing_trace: Option<Vec<serde_json::Value>>,
    ) {
        let prefix = existing_trace.unwrap_or_default();

        match result {
            Ok(result) => {
                let mut full_trace = prefix;
                full_trace.extend(result.trace);
                let trace_json = serde_json::Value::Array(full_trace);
                let step_count = result.step_index as i32;

                if result.approval_token.is_some() {
                    // Approval gates are not yet implemented (WF-08).
                    // Fail explicitly rather than creating unreachable WaitingApproval rows.
                    tracing::warn!(
                        run_id = %run_id,
                        step_index = result.step_index,
                        "Workflow hit approval gate — not yet implemented, marking as failed"
                    );
                    if let Err(e) = self
                        .db
                        .update_workflow_run(
                            run_id,
                            RunStatus::Failed,
                            step_count,
                            &trace_json,
                            Some("approval gates not yet implemented — see WF-08"),
                        )
                        .await
                    {
                        tracing::error!(
                            run_id = %run_id,
                            "Failed to update run to Failed (approval gate): {e}"
                        );
                    }
                } else {
                    tracing::info!(run_id = %run_id, "Workflow run completed");
                    if let Err(e) = self
                        .db
                        .update_workflow_run(
                            run_id,
                            RunStatus::Completed,
                            step_count,
                            &trace_json,
                            None,
                        )
                        .await
                    {
                        tracing::error!(
                            run_id = %run_id,
                            "Failed to update run to Completed: {e}"
                        );
                    }
                }
            }
            Err((e, progress)) => {
                tracing::error!(run_id = %run_id, "Workflow run failed: {e}");
                let mut full_trace = prefix;
                full_trace.extend(progress.trace);
                let trace_json = serde_json::Value::Array(full_trace);
                if let Err(db_err) = self
                    .db
                    .update_workflow_run(
                        run_id,
                        RunStatus::Failed,
                        progress.step_index as i32,
                        &trace_json,
                        Some(&e.to_string()),
                    )
                    .await
                {
                    tracing::error!(
                        run_id = %run_id,
                        "Failed to update run to Failed: {db_err}"
                    );
                }
            }
        }
    }

    /// Called from the event handler post-store hook for every stored event.
    ///
    /// Checks whether any workflow in the event's channel has a matching trigger.
    /// Workflow execution events (kinds 46001–46012) are excluded to prevent loops.
    ///
    /// The method takes `self: &Arc<Self>` so that the spawned task can hold a
    /// clone of the `Arc` without requiring `'static` on `&self`.
    pub async fn on_event(
        self: &Arc<Self>,
        event: &buzz_core::StoredEvent,
    ) -> Result<(), WorkflowError> {
        let Some(channel_id) = event.channel_id else {
            tracing::debug!(
                event_id = %event.event.id.to_hex(),
                kind = event_kind_u32(&event.event),
                "Skipping workflow trigger — event has no channel_id"
            );
            return Ok(());
        };

        let kind_u32 = event_kind_u32(&event.event);

        // Exclude workflow execution events to prevent infinite loops.
        if is_workflow_execution_kind(kind_u32) {
            return Ok(());
        }

        let workflows = self
            .db
            .list_enabled_channel_workflows(channel_id)
            .await
            .map_err(WorkflowError::from)?;

        if workflows.is_empty() {
            return Ok(());
        }

        let trigger_ctx = build_trigger_context(event);

        let trigger_ctx_json: serde_json::Value = match serde_json::to_value(&trigger_ctx) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Failed to serialize trigger context: {e}");
                return Ok(());
            }
        };

        for workflow in &workflows {
            let def: WorkflowDef = match serde_json::from_value(workflow.definition.clone()) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(workflow_id = %workflow.id, "Failed to parse definition: {e}");
                    continue;
                }
            };

            if !def.enabled || !trigger_matches_event(&def.trigger, kind_u32) {
                continue;
            }

            if !should_fire_workflow(&def, &trigger_ctx, workflow.id).await {
                continue;
            }

            let trigger_event_id_bytes = event.event.id.as_bytes().to_vec();
            let run_id = match self
                .db
                .create_workflow_run(
                    workflow.id,
                    Some(&trigger_event_id_bytes),
                    Some(&trigger_ctx_json),
                )
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!(workflow_id = %workflow.id, "Failed to create run: {e}");
                    continue;
                }
            };

            tracing::debug!(
                workflow_id = %workflow.id,
                run_id = %run_id,
                "Workflow triggered — spawning execution"
            );

            let engine = Arc::clone(self);
            let def_clone = def.clone();
            let ctx_clone = trigger_ctx.clone();

            tokio::spawn(async move {
                let result = executor::execute_run(&engine, run_id, &def_clone, &ctx_clone).await;
                engine.finalize_run(run_id, result, None).await;
            });
        }

        Ok(())
    }

    /// Background loop for scheduled (cron/interval) triggers.
    ///
    /// Ticks every 60 seconds. For each active workflow with a `Schedule`
    /// trigger, checks whether the cron expression or interval has elapsed
    /// and spawns execution if so.
    ///
    /// Uses window-based matching for cron expressions to handle tick drift:
    /// `schedule.after(&(now - 60s)).next() <= now` instead of `includes(now)`.
    ///
    /// Cron and interval triggers claim a DB-authored `(workflow_id, scheduled_for)`
    /// row before creating a run. The claim resolves the workflow community inside
    /// Postgres, so the caller never supplies tenant identity for scheduled fires.
    pub async fn run(self: &Arc<Self>) {
        tracing::info!("WorkflowEngine cron loop started (60s tick)");

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;

            let now = Utc::now();

            let workflows = match self.db.list_all_enabled_workflows().await {
                Ok(wf) => wf,
                Err(e) => {
                    tracing::error!("Cron tick: failed to load workflows: {e}");
                    continue;
                }
            };

            for workflow in &workflows {
                self.process_scheduled_workflow_at(workflow, now).await;
            }
        }
    }

    async fn process_scheduled_workflow_at(
        self: &Arc<Self>,
        workflow: &WorkflowRecord,
        now: DateTime<Utc>,
    ) -> Option<Uuid> {
        let def: schema::WorkflowDef = match serde_json::from_value(workflow.definition.clone()) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    workflow_id = %workflow.id,
                    "Cron tick: failed to parse workflow definition: {e}"
                );
                return None;
            }
        };

        if !def.enabled {
            return None;
        }

        // Skip workflows with no channel_id — an empty channel_id causes silent
        // downstream failures when the run tries to act on a channel.
        let Some(channel_id) = workflow.channel_id else {
            tracing::warn!(
                workflow_id = %workflow.id,
                "Cron tick: skipping schedule workflow with no channel_id"
            );
            return None;
        };

        let (scheduled_for, trigger_type) = match &def.trigger {
            schema::TriggerDef::Schedule {
                cron: Some(expr),
                interval: None,
            } => (cron_should_fire(expr, now, 60, workflow.id), "cron"),
            schema::TriggerDef::Schedule {
                cron: None,
                interval: Some(dur),
            } => {
                let latest = match self.db.latest_scheduled_workflow_fire(workflow.id).await {
                    Ok(latest) => latest,
                    Err(e) => {
                        tracing::error!(
                            workflow_id = %workflow.id,
                            "Cron tick: failed to load latest scheduled workflow fire: {e}"
                        );
                        return None;
                    }
                };
                (
                    interval_should_fire(dur, latest, workflow.created_at, now, workflow.id),
                    "interval",
                )
            }
            _ => (None, ""), // Non-schedule triggers handled by on_event()
        };

        let scheduled_for = scheduled_for?;

        let trigger_ctx = executor::TriggerContext {
            channel_id: channel_id.to_string(),
            timestamp: scheduled_for.timestamp().to_string(),
            ..Default::default()
        };
        let trigger_ctx_json = match serde_json::to_value(&trigger_ctx) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::error!(
                    workflow_id = %workflow.id,
                    "Cron tick: failed to serialize trigger context: {e}"
                );
                return None;
            }
        };

        let claim = match self
            .db
            .claim_scheduled_workflow_fire(workflow.id, scheduled_for)
            .await
        {
            Ok(Some(claim)) => claim,
            Ok(None) => {
                tracing::debug!(
                    workflow_id = %workflow.id,
                    scheduled_for = %scheduled_for,
                    trigger = trigger_type,
                    "Cron tick: scheduled workflow fire already claimed"
                );
                return None;
            }
            Err(e) => {
                tracing::error!(
                    workflow_id = %workflow.id,
                    scheduled_for = %scheduled_for,
                    trigger = trigger_type,
                    "Cron tick: failed to claim scheduled workflow fire: {e}"
                );
                return None;
            }
        };

        let run_id = match self
            .db
            .create_workflow_run(
                workflow.id,
                None, // no trigger event for cron
                trigger_ctx_json.as_ref(),
            )
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(
                    workflow_id = %workflow.id,
                    "Cron tick: failed to create workflow run: {e}"
                );
                return None;
            }
        };

        match self
            .db
            .attach_scheduled_workflow_run(workflow.id, scheduled_for, run_id)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    workflow_id = %workflow.id,
                    run_id = %run_id,
                    scheduled_for = %scheduled_for,
                    "Cron tick: scheduled workflow claim was not attached to run"
                );
            }
            Err(e) => {
                tracing::warn!(
                    workflow_id = %workflow.id,
                    run_id = %run_id,
                    scheduled_for = %scheduled_for,
                    "Cron tick: failed to attach workflow run to scheduled claim: {e}"
                );
            }
        }

        tracing::info!(
            workflow_id = %workflow.id,
            community_id = %claim.community_id.as_uuid(),
            run_id = %run_id,
            trigger = trigger_type,
            scheduled_for = %scheduled_for,
            claimed_at = %claim.claimed_at,
            "Cron trigger fired"
        );

        let engine = Arc::clone(self);
        let def_clone = def.clone();
        let ctx_clone = trigger_ctx.clone();
        tokio::spawn(async move {
            let result = executor::execute_run(&engine, run_id, &def_clone, &ctx_clone).await;
            engine.finalize_run(run_id, result, None).await;
        });

        Some(run_id)
    }
}

/// Check whether a cron expression should fire within the `window_secs`-wide
/// window ending at `now`.
///
/// Uses window-based matching: finds the next scheduled time after
/// `(now - window_secs)` and checks whether it falls at or before `now`.
/// This tolerates tick drift gracefully — a 61s tick won't miss a
/// minute-granularity cron expression.
///
/// Returns `None` (and logs a warning) if the expression is invalid.
fn cron_should_fire(
    expr: &str,
    now: DateTime<Utc>,
    window_secs: i64,
    workflow_id: Uuid,
) -> Option<DateTime<Utc>> {
    let normalized = schema::normalize_cron(expr);
    match normalized.parse::<cron::Schedule>() {
        Ok(sched) => {
            let window_start = now - chrono::Duration::seconds(window_secs);
            sched.after(&window_start).next().filter(|t| *t <= now)
        }
        Err(e) => {
            tracing::warn!(
                workflow_id = %workflow_id,
                "Cron tick: invalid cron expression '{expr}': {e}"
            );
            None
        }
    }
}

/// Return the most recent interval schedule instant due at or before `now`.
///
/// The anchor is DB-authoritative: the last claimed schedule instant when one
/// exists, otherwise the workflow row's `created_at`. Returning a canonical
/// interval boundary makes all pods compute the same `(workflow_id,
/// scheduled_for)` claim key even when their local clocks are in different parts
/// of the same interval window. If multiple intervals elapsed while the engine
/// was down, old intervals are skipped instead of replayed one per tick.
///
/// Returns `None` (and logs a warning) if the duration string is invalid.
fn interval_should_fire(
    dur: &str,
    latest_scheduled_for: Option<DateTime<Utc>>,
    workflow_created_at: DateTime<Utc>,
    now: DateTime<Utc>,
    workflow_id: Uuid,
) -> Option<DateTime<Utc>> {
    let anchor = latest_scheduled_for.unwrap_or(workflow_created_at);

    match executor::parse_duration_secs(dur) {
        Ok(interval_secs) => {
            let Ok(interval_secs) = i64::try_from(interval_secs) else {
                tracing::warn!(
                    workflow_id = %workflow_id,
                    "Cron tick: interval '{dur}' is too large"
                );
                return None;
            };
            if interval_secs <= 0 {
                tracing::warn!(
                    workflow_id = %workflow_id,
                    "Cron tick: interval '{dur}' must be positive"
                );
                return None;
            }

            let elapsed_secs = (now - anchor).num_seconds();
            if elapsed_secs < interval_secs {
                return None;
            }

            let elapsed_intervals = elapsed_secs / interval_secs;
            Some(anchor + chrono::Duration::seconds(interval_secs * elapsed_intervals))
        }
        Err(e) => {
            tracing::warn!(
                workflow_id = %workflow_id,
                "Cron tick: invalid interval '{dur}': {e}"
            );
            None
        }
    }
}

/// Check emoji and filter-expression conditions that determine whether a
/// matched workflow should actually fire. Extracted from `on_event` to keep
/// the per-workflow loop body small.
///
/// Returns `true` if the workflow should fire, `false` to skip.
async fn should_fire_workflow(
    def: &WorkflowDef,
    trigger_ctx: &executor::TriggerContext,
    workflow_id: uuid::Uuid,
) -> bool {
    if let TriggerDef::ReactionAdded {
        emoji: Some(ref expected),
    } = def.trigger
    {
        if &trigger_ctx.emoji != expected {
            tracing::debug!(
                workflow_id = %workflow_id,
                expected_emoji = %expected,
                actual_emoji = %trigger_ctx.emoji,
                "Reaction emoji mismatch — skipping workflow"
            );
            return false;
        }
    }

    if let TriggerDef::MessagePosted {
        filter: Some(ref expr),
    } = def.trigger
    {
        match executor::evaluate_condition(expr, trigger_ctx, &HashMap::new()).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(
                    workflow_id = %workflow_id,
                    "Trigger filter evaluated false — skipping workflow"
                );
                return false;
            }
            Err(e) => {
                tracing::warn!(
                    workflow_id = %workflow_id,
                    "Trigger filter error: {e} — skipping workflow"
                );
                return false;
            }
        }
    }

    if let TriggerDef::DiffPosted {
        filter: Some(ref expr),
    } = def.trigger
    {
        match executor::evaluate_condition(expr, trigger_ctx, &HashMap::new()).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(
                    workflow_id = %workflow_id,
                    "Trigger filter evaluated false — skipping workflow"
                );
                return false;
            }
            Err(e) => {
                tracing::warn!(
                    workflow_id = %workflow_id,
                    "Trigger filter error: {e} — skipping workflow"
                );
                return false;
            }
        }
    }

    true
}

/// Build a [`executor::TriggerContext`] from a [`buzz_core::StoredEvent`].
///
/// - `text` — event content (message body or reaction emoji character)
/// - `author` — pubkey hex string
/// - `channel_id` — channel UUID as string (empty if no channel scope)
/// - `timestamp` — Unix timestamp as string
/// - `emoji` — for `KIND_REACTION` events, the content is the emoji; otherwise empty
/// - `message_id` — for reactions, the target message's event ID (from `e` tag);
///   for all other events, the event's own ID
pub fn build_trigger_context(event: &buzz_core::StoredEvent) -> executor::TriggerContext {
    let kind_u32 = event_kind_u32(&event.event);
    let content = event.event.content.clone();

    let author = event
        .event
        .tags
        .iter()
        .find_map(|tag| {
            if tag.kind().to_string() == "actor" {
                tag.content().map(|value| value.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| event.event.pubkey.to_hex());

    // For reaction events (NIP-25), the content field holds the emoji character
    // or shortcode (e.g. "👍", "+", "-"). Expose it as `emoji`.
    let emoji = if kind_u32 == KIND_REACTION {
        content.clone()
    } else {
        String::new()
    };

    // For reactions (NIP-25), `message_id` should be the target message, not
    // the reaction event itself. NIP-25 stores the target in an `e` tag whose
    // value is a 64-char hex event ID (not a UUID channel reference).
    // Per NIP-25, the last `e` tag is the direct target (earlier ones may be thread roots).
    let message_id = if kind_u32 == KIND_REACTION {
        event
            .event
            .tags
            .iter()
            .rev()
            .find_map(|tag| {
                let key = tag.kind().to_string();
                if key == "e" {
                    tag.content().and_then(|v| {
                        // Distinguish hex event IDs (64 chars) from UUID channel refs.
                        if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                            Some(v.to_string())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            // Fallback to the reaction event's own ID if no valid `e` tag found.
            .unwrap_or_else(|| event.event.id.to_hex())
    } else {
        event.event.id.to_hex()
    };

    executor::TriggerContext {
        text: content,
        author,
        channel_id: event
            .channel_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        timestamp: event.event.created_at.as_secs().to_string(),
        emoji,
        message_id,
        webhook_fields: HashMap::new(),
    }
}

/// Returns `true` if the trigger type matches the given event kind.
fn trigger_matches_event(trigger: &TriggerDef, kind_u32: u32) -> bool {
    use buzz_core::kind::{KIND_REACTION, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_DIFF};
    match trigger {
        TriggerDef::MessagePosted { .. } => kind_u32 == KIND_STREAM_MESSAGE,
        TriggerDef::ReactionAdded { .. } => kind_u32 == KIND_REACTION,
        TriggerDef::DiffPosted { .. } => kind_u32 == KIND_STREAM_MESSAGE_DIFF,
        // Schedule and Webhook triggers are not fired by channel events.
        TriggerDef::Schedule { .. } | TriggerDef::Webhook => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_should_fire_matches_within_window() {
        // "every minute" cron — should always fire within a 60s window.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:30Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert!(
            cron_should_fire("* * * * *", now, 60, wf_id).is_some(),
            "every-minute cron should fire within 60s window"
        );
    }

    #[test]
    fn cron_should_fire_returns_false_for_invalid_expr() {
        let now = Utc::now();
        let wf_id = Uuid::new_v4();
        assert!(
            cron_should_fire("not-a-cron", now, 60, wf_id).is_none(),
            "invalid cron should return false"
        );
    }

    #[test]
    fn cron_should_fire_returns_false_outside_window() {
        // Fixed time: 2026-06-15 14:30:00 UTC (a Sunday in June)
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        // "0 0 1 1 *" = midnight on Jan 1 only — June 15 is definitely outside.
        assert!(
            cron_should_fire("0 0 1 1 *", now, 60, wf_id).is_none(),
            "Jan-1-only cron should not fire on June 15"
        );
    }

    #[test]
    fn cron_should_fire_at_exact_minute_boundary() {
        // Fixed time: exactly 09:00:00 UTC. Cron "0 9 * * *" fires at 09:00.
        // Window [08:59:00, 09:00:00] should contain the fire time.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert_eq!(
            cron_should_fire("0 9 * * *", now, 60, wf_id),
            Some(now),
            "cron should fire at exact minute boundary"
        );
    }

    #[test]
    fn cron_should_fire_within_drift_window() {
        // Fixed time: 09:00:45 UTC (45s drift). Cron "0 9 * * *" fires at 09:00.
        // Window [08:59:45, 09:00:45] should still contain 09:00:00.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:45Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        let scheduled = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            cron_should_fire("0 9 * * *", now, 60, wf_id),
            Some(scheduled),
            "cron should return the canonical scheduled instant even with drift"
        );
    }

    #[test]
    fn cron_should_fire_returns_false_just_outside_window() {
        // Fixed time: 09:01:01 UTC. Cron "0 9 * * *" fires at 09:00:00.
        // Window [09:00:01, 09:01:01] does NOT contain 09:00:00.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:01:01Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert!(
            cron_should_fire("0 9 * * *", now, 60, wf_id).is_none(),
            "cron should not fire 61s after the scheduled time"
        );
    }

    #[test]
    fn interval_should_fire_returns_false_before_first_interval_from_created_at() {
        let wf_id = Uuid::new_v4();
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let now = created_at + chrono::Duration::minutes(30);
        assert!(
            interval_should_fire("1h", None, created_at, now, wf_id).is_none(),
            "first interval should not fire before the workflow-created anchor reaches the interval"
        );
    }

    #[test]
    fn interval_should_fire_anchors_first_fire_to_workflow_created_at() {
        let wf_id = Uuid::new_v4();
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let now = created_at + chrono::Duration::hours(1) + chrono::Duration::seconds(30);
        assert_eq!(
            interval_should_fire("1h", None, created_at, now, wf_id),
            Some(created_at + chrono::Duration::hours(1)),
            "first interval fire should use the DB row-created anchor, not per-pod startup time"
        );
    }

    #[test]
    fn interval_should_fire_skips_missed_intervals_from_created_at() {
        let wf_id = Uuid::new_v4();
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let now = created_at + chrono::Duration::hours(3) + chrono::Duration::minutes(5);
        assert_eq!(
            interval_should_fire("1h", None, created_at, now, wf_id),
            Some(created_at + chrono::Duration::hours(3)),
            "should claim the most recent due boundary, not replay old missed intervals"
        );
    }

    #[test]
    fn interval_should_fire_skips_missed_intervals_after_latest_claim() {
        let wf_id = Uuid::new_v4();
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let latest = created_at + chrono::Duration::hours(1);
        let now = latest + chrono::Duration::hours(2) + chrono::Duration::minutes(5);
        assert_eq!(
            interval_should_fire("1h", Some(latest), created_at, now, wf_id),
            Some(latest + chrono::Duration::hours(2)),
            "should return the most recent due boundary, not catch up one interval per tick"
        );
    }

    #[test]
    fn interval_should_fire_returns_false_before_interval_elapsed() {
        let wf_id = Uuid::new_v4();
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let latest = created_at + chrono::Duration::hours(1);
        let now = latest + chrono::Duration::minutes(30);
        assert!(
            interval_should_fire("1h", Some(latest), created_at, now, wf_id).is_none(),
            "should not fire before interval elapsed"
        );
    }

    #[test]
    fn interval_should_fire_returns_false_for_invalid_duration() {
        let created_at = Utc::now();
        let now = created_at + chrono::Duration::hours(1);
        let wf_id = Uuid::new_v4();
        assert!(
            interval_should_fire("not-a-duration", None, created_at, now, wf_id).is_none(),
            "invalid duration should return false"
        );
    }

    #[test]
    fn interval_should_fire_at_exact_boundary() {
        let wf_id = Uuid::new_v4();
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let latest = created_at + chrono::Duration::hours(1);
        let now = latest + chrono::Duration::hours(1);
        assert_eq!(
            interval_should_fire("1h", Some(latest), created_at, now, wf_id),
            Some(now),
            "should fire at exact interval boundary"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn scheduled_cron_claim_is_exactly_once_across_two_engines() {
        let (db, pool) = connect_test_db().await;
        let community = db
            .ensure_configured_community(&format!("workflow-cron-claim-{}.test", Uuid::new_v4()))
            .await
            .expect("create community");
        let owner_pubkey = [42_u8; 32];
        db.ensure_user(&owner_pubkey).await.expect("create owner");

        let channel_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, created_by)
            VALUES ($1, $2, $3, 'stream', 'open', $4)
            "#,
        )
        .bind(channel_id)
        .bind(community.id.as_uuid())
        .bind("workflow cron claim test")
        .bind(owner_pubkey.as_slice())
        .execute(&pool)
        .await
        .expect("create channel");

        let (_def, definition_json) = WorkflowEngine::parse_yaml(
            r#"
name: Cron claim test
trigger:
  on: schedule
  cron: '* * * * *'
steps:
  - id: wait
    action: delay
    duration: 0s
"#,
        )
        .expect("valid workflow yaml");

        let workflow_id = db
            .create_workflow(
                community.id,
                Some(channel_id),
                &owner_pubkey,
                "Cron claim test",
                &definition_json,
                b"test-definition-hash",
            )
            .await
            .expect("create workflow");
        let workflow = db.get_workflow(workflow_id).await.expect("load workflow");

        let engine_a = Arc::new(WorkflowEngine::new(db.clone(), WorkflowConfig::default()));
        let engine_b = Arc::new(WorkflowEngine::new(db.clone(), WorkflowConfig::default()));
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let (run_a, run_b) = tokio::join!(
            engine_a.process_scheduled_workflow_at(&workflow, now),
            engine_b.process_scheduled_workflow_at(&workflow, now)
        );

        assert_eq!(
            [run_a, run_b].into_iter().flatten().count(),
            1,
            "only one engine should create a run for the same cron instant"
        );

        assert_eq!(
            engine_a.process_scheduled_workflow_at(&workflow, now).await,
            None,
            "re-processing the same cron instant should not create another run"
        );

        let fire_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM scheduled_workflow_fires WHERE workflow_id = $1",
        )
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .expect("count scheduled fires");
        assert_eq!(fire_count, 1);

        let run_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM workflow_runs WHERE workflow_id = $1",
        )
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .expect("count workflow runs");
        assert_eq!(run_count, 1);
    }

    /// Engine-layer crash-mid-run audit (b): proves the `scheduled_workflow_fires`
    /// claim row alone — *with no attached run* — is the dedupe boundary the
    /// scheduler relies on. Simulates: a prior pod won the claim, then died
    /// before `create_workflow_run` (or before `attach_scheduled_workflow_run`),
    /// leaving an orphan claim row. A subsequent tick for the same canonical
    /// `scheduled_for` must no-op rather than create a duplicate run.
    ///
    /// A future refactor that ever made `attach_scheduled_workflow_run` (or the
    /// `workflow_runs` row itself) the dedupe gate instead of the claim row
    /// would still pass `scheduled_cron_claim_is_exactly_once_across_two_engines`
    /// (which only exercises the success path) but must fail this one.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn orphan_claim_blocks_refire_at_same_canonical_instant() {
        let (db, pool) = connect_test_db().await;
        let community = db
            .ensure_configured_community(&format!("workflow-cron-orphan-{}.test", Uuid::new_v4()))
            .await
            .expect("create community");
        let owner_pubkey = [43_u8; 32];
        db.ensure_user(&owner_pubkey).await.expect("create owner");

        let channel_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, created_by)
            VALUES ($1, $2, $3, 'stream', 'open', $4)
            "#,
        )
        .bind(channel_id)
        .bind(community.id.as_uuid())
        .bind("workflow cron orphan test")
        .bind(owner_pubkey.as_slice())
        .execute(&pool)
        .await
        .expect("create channel");

        // Fixed daily cron so the canonical instant is trivially known.
        let (_def, definition_json) = WorkflowEngine::parse_yaml(
            r#"
name: Cron orphan test
trigger:
  on: schedule
  cron: '0 12 * * *'
steps:
  - id: wait
    action: delay
    duration: 0s
"#,
        )
        .expect("valid workflow yaml");

        let workflow_id = db
            .create_workflow(
                community.id,
                Some(channel_id),
                &owner_pubkey,
                "Cron orphan test",
                &definition_json,
                b"test-definition-hash",
            )
            .await
            .expect("create workflow");
        let workflow = db.get_workflow(workflow_id).await.expect("load workflow");

        // Cron `0 12 * * *` + `now = 12:00:30Z` → canonical scheduled_for = 12:00:00Z.
        let canonical_scheduled_for = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:30Z")
            .unwrap()
            .with_timezone(&Utc);

        // Pre-insert an orphan claim row: claim exists, workflow_run_id is NULL
        // (simulating prior pod crashed after claim, before create_workflow_run).
        // Source community_id from the workflow row to keep the FK pair sound —
        // the same invariant F4 made schema-impossible to forge.
        sqlx::query(
            r#"
            INSERT INTO scheduled_workflow_fires
                (community_id, workflow_id, scheduled_for, workflow_run_id)
            SELECT w.community_id, w.id, $2, NULL
            FROM workflows w
            WHERE w.id = $1
            "#,
        )
        .bind(workflow_id)
        .bind(canonical_scheduled_for)
        .execute(&pool)
        .await
        .expect("seed orphan claim row");

        let engine = Arc::new(WorkflowEngine::new(db.clone(), WorkflowConfig::default()));
        let result = engine.process_scheduled_workflow_at(&workflow, now).await;

        assert_eq!(
            result, None,
            "orphan claim row must block refire — the claim row, not attach/run, is the dedupe boundary"
        );

        let run_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM workflow_runs WHERE workflow_id = $1",
        )
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .expect("count workflow runs");
        assert_eq!(
            run_count, 0,
            "no workflow_runs row should exist for this workflow — the orphan claim must short-circuit before create_workflow_run"
        );

        let fire_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM scheduled_workflow_fires WHERE workflow_id = $1",
        )
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .expect("count scheduled fires");
        assert_eq!(
            fire_count, 1,
            "the single orphan claim row must remain — no duplicate claim was inserted"
        );
    }

    /// Engine-layer canonical-boundary audit (c): adjacent interval windows
    /// fed through the seam produce distinct run ids.
    ///
    /// Sami's `clock_skewed_adjacent_windows_each_claim_independently` proves
    /// the DB layer keeps adjacent `scheduled_for` keys independent under the
    /// composite PK. This test proves the engine-side canonicalization
    /// (`interval_should_fire`) actually *produces* distinct keys for adjacent
    /// `now` values when the anchor shifts between ticks — i.e. the property
    /// `71da65e51` was designed to provide is observable end-to-end through
    /// the seam.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn adjacent_interval_boundaries_produce_distinct_runs() {
        let (db, pool) = connect_test_db().await;
        let community = db
            .ensure_configured_community(&format!("workflow-cron-adjacent-{}.test", Uuid::new_v4()))
            .await
            .expect("create community");
        let owner_pubkey = [44_u8; 32];
        db.ensure_user(&owner_pubkey).await.expect("create owner");

        let channel_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, created_by)
            VALUES ($1, $2, $3, 'stream', 'open', $4)
            "#,
        )
        .bind(channel_id)
        .bind(community.id.as_uuid())
        .bind("workflow cron adjacent test")
        .bind(owner_pubkey.as_slice())
        .execute(&pool)
        .await
        .expect("create channel");

        // Interval workflow exercises the seam's own canonical-boundary math
        // through `interval_should_fire`. The DB-authoritative anchor
        // (latest_scheduled_workflow_fire) shifts after the first call, which is
        // the exact property under test.
        let (_def, definition_json) = WorkflowEngine::parse_yaml(
            r#"
name: Interval adjacent test
trigger:
  on: schedule
  interval: 60s
steps:
  - id: wait
    action: delay
    duration: 0s
"#,
        )
        .expect("valid workflow yaml");

        let workflow_id = db
            .create_workflow(
                community.id,
                Some(channel_id),
                &owner_pubkey,
                "Interval adjacent test",
                &definition_json,
                b"test-definition-hash",
            )
            .await
            .expect("create workflow");
        let workflow = db.get_workflow(workflow_id).await.expect("load workflow");

        let engine = Arc::new(WorkflowEngine::new(db.clone(), WorkflowConfig::default()));

        // Anchor is workflow.created_at. With interval = 60s:
        //   now_1 = anchor + 90s → canonical scheduled_for_1 = anchor + 60s
        //   (after call 1, latest_scheduled_workflow_fire == anchor + 60s)
        //   now_2 = anchor + 150s → canonical scheduled_for_2 = anchor + 120s
        //                          (= new_anchor + 60s)
        let anchor = workflow.created_at;
        let now_1 = anchor + chrono::Duration::seconds(90);
        let now_2 = anchor + chrono::Duration::seconds(150);

        let run_1 = engine
            .process_scheduled_workflow_at(&workflow, now_1)
            .await
            .expect("first canonical boundary must fire");
        let run_2 = engine
            .process_scheduled_workflow_at(&workflow, now_2)
            .await
            .expect("second canonical boundary must fire");

        assert_ne!(
            run_1, run_2,
            "adjacent canonical boundaries must produce distinct run ids"
        );

        let fire_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM scheduled_workflow_fires WHERE workflow_id = $1",
        )
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .expect("count scheduled fires");
        assert_eq!(
            fire_count, 2,
            "two distinct canonical instants must produce two claim rows"
        );

        let run_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM workflow_runs WHERE workflow_id = $1",
        )
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .expect("count workflow runs");
        assert_eq!(run_count, 2);
    }

    async fn connect_test_db() -> (Db, sqlx::PgPool) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned());
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("connect to test DB");
        let db = Db::from_pool(pool.clone());
        db.migrate().await.expect("run migrations");
        (db, pool)
    }

    #[test]
    fn workflow_config_defaults() {
        let cfg = WorkflowConfig::default();
        assert_eq!(cfg.max_concurrent, 100);
        assert_eq!(cfg.default_timeout_secs, 300);
    }

    #[test]
    fn parse_yaml_roundtrip() {
        let yaml = r#"
name: "Test Workflow"
trigger:
  on: message_posted
steps:
  - id: s1
    action: send_message
    text: "Hello {{trigger.author}}"
"#;
        let (def, json) = WorkflowEngine::parse_yaml(yaml).expect("parse failed");
        assert_eq!(def.name, "Test Workflow");

        let reparsed: WorkflowDef = serde_json::from_str(&json).expect("json round-trip");
        assert_eq!(reparsed.name, def.name);
        assert_eq!(reparsed.steps.len(), 1);
    }

    #[test]
    fn trigger_matches_stream_message() {
        let trigger = TriggerDef::MessagePosted { filter: None };
        assert!(trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_STREAM_MESSAGE
        ));
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_REACTION
        ));
    }

    #[test]
    fn trigger_matches_reaction() {
        let trigger = TriggerDef::ReactionAdded { emoji: None };
        assert!(trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_REACTION
        ));
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_STREAM_MESSAGE
        ));
    }

    #[test]
    fn schedule_trigger_never_matches_events() {
        let trigger = TriggerDef::Schedule {
            cron: Some("0 9 * * 1-5".to_owned()),
            interval: None,
        };
        // Schedule triggers are fired by the cron loop, not by events.
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_STREAM_MESSAGE
        ));
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_REACTION
        ));
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_WORKFLOW_TRIGGERED
        ));
    }

    #[test]
    fn webhook_trigger_never_matches_events() {
        let trigger = TriggerDef::Webhook;
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_STREAM_MESSAGE
        ));
        assert!(!trigger_matches_event(&trigger, 0));
    }

    #[test]
    fn message_posted_matches_kind_9_only() {
        let trigger = TriggerDef::MessagePosted { filter: None };
        // Must match KIND_STREAM_MESSAGE = 9.
        assert!(trigger_matches_event(&trigger, 9));
        // Must NOT match reaction (kind 7).
        assert!(!trigger_matches_event(&trigger, 7));
        // Must NOT match forum post (kind 45001).
        assert!(!trigger_matches_event(&trigger, 45001));
        // Must NOT match stream message v2 (kind 40002).
        assert!(!trigger_matches_event(&trigger, 40002));
    }

    #[test]
    fn reaction_added_matches_kind_7_only() {
        let trigger = TriggerDef::ReactionAdded { emoji: None };
        // Must match KIND_REACTION = 7.
        assert!(trigger_matches_event(&trigger, 7));
        // Must NOT match stream message (kind 9).
        assert!(!trigger_matches_event(&trigger, 9));
        // Must NOT match forum post (kind 45001).
        assert!(!trigger_matches_event(&trigger, 45001));
    }

    #[test]
    fn reaction_added_with_emoji_filter_still_matches_kind_7() {
        // The emoji filter is evaluated at execution time, not trigger-matching time.
        // trigger_matches_event only checks the kind number.
        let trigger = TriggerDef::ReactionAdded {
            emoji: Some("thumbsup".to_owned()),
        };
        assert!(trigger_matches_event(&trigger, 7));
        assert!(!trigger_matches_event(&trigger, 9));
    }

    #[test]
    fn message_posted_with_filter_still_matches_kind_9() {
        // The filter expression is evaluated at execution time, not trigger-matching time.
        let trigger = TriggerDef::MessagePosted {
            filter: Some("str_contains(trigger_text, \"P1\")".to_owned()),
        };
        assert!(trigger_matches_event(&trigger, 9));
        assert!(!trigger_matches_event(&trigger, 7));
    }

    #[test]
    fn workflow_execution_kinds_do_not_match_any_trigger() {
        // Workflow execution events (46001–46012) must never match triggers
        // to prevent infinite loops. The on_event() method filters these out
        // before calling trigger_matches_event, but verify the function itself
        // also returns false for these kinds.
        let msg_trigger = TriggerDef::MessagePosted { filter: None };
        let react_trigger = TriggerDef::ReactionAdded { emoji: None };

        for kind in buzz_core::kind::KIND_WORKFLOW_TRIGGERED
            ..=buzz_core::kind::KIND_WORKFLOW_APPROVAL_DENIED
        {
            assert!(
                !trigger_matches_event(&msg_trigger, kind),
                "message_posted should not match workflow execution kind {kind}"
            );
            assert!(
                !trigger_matches_event(&react_trigger, kind),
                "reaction_added should not match workflow execution kind {kind}"
            );
        }
    }

    #[test]
    fn trigger_matches_event_kind_zero_matches_nothing() {
        // Kind 0 is a profile event — no trigger should match it.
        let msg_trigger = TriggerDef::MessagePosted { filter: None };
        let react_trigger = TriggerDef::ReactionAdded { emoji: None };
        let sched_trigger = TriggerDef::Schedule {
            cron: None,
            interval: Some("1h".to_owned()),
        };
        let webhook_trigger = TriggerDef::Webhook;

        assert!(!trigger_matches_event(&msg_trigger, 0));
        assert!(!trigger_matches_event(&react_trigger, 0));
        assert!(!trigger_matches_event(&sched_trigger, 0));
        assert!(!trigger_matches_event(&webhook_trigger, 0));
    }

    #[test]
    fn diff_posted_matches_kind_40008_only() {
        let trigger = TriggerDef::DiffPosted { filter: None };
        assert!(trigger_matches_event(&trigger, 40008));
        assert!(!trigger_matches_event(&trigger, 9));
        assert!(!trigger_matches_event(&trigger, 7));
    }

    #[test]
    fn message_posted_does_not_match_kind_40008() {
        let trigger = TriggerDef::MessagePosted { filter: None };
        assert!(!trigger_matches_event(&trigger, 40008));
        assert!(trigger_matches_event(&trigger, 9));
    }

    #[test]
    fn workflow_config_custom_values() {
        let cfg = WorkflowConfig {
            max_concurrent: 50,
            default_timeout_secs: 600,
        };
        assert_eq!(cfg.max_concurrent, 50);
        assert_eq!(cfg.default_timeout_secs, 600);
    }

    fn make_message_event() -> buzz_core::StoredEvent {
        use nostr::{EventBuilder, Keys, Kind};
        use uuid::Uuid;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "hello world")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        buzz_core::StoredEvent::new(event, Some(Uuid::new_v4()))
    }

    /// Create a reaction event with an `e` tag pointing to a target message.
    fn make_reaction_event() -> (buzz_core::StoredEvent, String) {
        use nostr::{EventBuilder, Keys, Kind, Tag};
        use uuid::Uuid;
        let keys = Keys::generate();
        // Create a dummy target message ID (64-char hex).
        let target_keys = Keys::generate();
        let target_event = EventBuilder::new(Kind::Custom(9), "target msg")
            .tags([])
            .sign_with_keys(&target_keys)
            .expect("sign target");
        let target_id_hex = target_event.id.to_hex();
        // NIP-25: reaction references the target via an `e` tag.
        let e_tag = Tag::parse(["e", &target_id_hex]).expect("tag parse");
        let event = EventBuilder::new(Kind::Reaction, "👍")
            .tags([e_tag])
            .sign_with_keys(&keys)
            .expect("sign");
        (
            buzz_core::StoredEvent::new(event, Some(Uuid::new_v4())),
            target_id_hex,
        )
    }

    #[test]
    fn build_trigger_context_message_event() {
        let stored = make_message_event();
        let ctx = build_trigger_context(&stored);

        assert_eq!(ctx.text, "hello world");
        assert_eq!(ctx.author, stored.event.pubkey.to_hex());
        assert_eq!(ctx.channel_id, stored.channel_id.unwrap().to_string());
        assert_eq!(ctx.timestamp, stored.event.created_at.as_secs().to_string());
        assert_eq!(ctx.message_id, stored.event.id.to_hex());
        // Non-reaction events have empty emoji.
        assert_eq!(ctx.emoji, "");
        assert!(ctx.webhook_fields.is_empty());
    }

    #[test]
    fn build_trigger_context_reaction_event() {
        let (stored, target_id_hex) = make_reaction_event();
        let ctx = build_trigger_context(&stored);

        // For reactions, content IS the emoji.
        assert_eq!(ctx.text, "👍");
        assert_eq!(ctx.emoji, "👍");
        assert_eq!(ctx.author, stored.event.pubkey.to_hex());
        // message_id should be the TARGET message, not the reaction event itself.
        assert_eq!(ctx.message_id, target_id_hex);
        assert_ne!(ctx.message_id, stored.event.id.to_hex());
        assert!(ctx.webhook_fields.is_empty());
    }

    #[test]
    fn build_trigger_context_no_channel_id() {
        use nostr::{EventBuilder, Keys, Kind};
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "msg")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        // channel_id = None (global/DM event)
        let stored = buzz_core::StoredEvent::new(event, None);
        let ctx = build_trigger_context(&stored);

        assert_eq!(ctx.channel_id, "");
        assert_eq!(ctx.text, "msg");
    }

    #[test]
    fn build_trigger_context_author_is_hex_pubkey() {
        let stored = make_message_event();
        let ctx = build_trigger_context(&stored);
        // Pubkey hex is 64 lowercase hex characters.
        assert_eq!(ctx.author.len(), 64);
        assert!(ctx.author.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn build_trigger_context_message_id_is_hex() {
        let stored = make_message_event();
        let ctx = build_trigger_context(&stored);
        // Event ID hex is 64 lowercase hex characters.
        assert_eq!(ctx.message_id.len(), 64);
        assert!(ctx.message_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn build_trigger_context_timestamp_is_numeric_string() {
        let stored = make_message_event();
        let ctx = build_trigger_context(&stored);
        // Timestamp must parse as a u64.
        ctx.timestamp
            .parse::<u64>()
            .expect("timestamp should be a u64 string");
    }

    #[test]
    fn test_build_trigger_context_reaction_multiple_e_tags() {
        // NIP-25: last e tag is the direct target, first may be thread root
        use nostr::{EventBuilder, EventId, Keys, Kind, Tag};
        use uuid::Uuid;

        let keys = Keys::generate();
        let thread_root_id = EventId::all_zeros();
        let direct_target_id = EventId::from_byte_array([0x42; 32]);

        let event = EventBuilder::new(Kind::Reaction, "👍")
            .tags([
                Tag::parse(["e", &thread_root_id.to_hex()]).unwrap(),
                Tag::parse(["e", &direct_target_id.to_hex()]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign");

        let stored = buzz_core::StoredEvent::new(event, Some(Uuid::new_v4()));
        let ctx = build_trigger_context(&stored);

        // Should pick the LAST e tag (direct target), not the first (thread root)
        assert_eq!(ctx.message_id, direct_target_id.to_hex());
    }
}
