//! Append-only, hash-chained audit log. The chain is computed by the database (trigger
//! `audit_chain`), so every writer — any replica, any code path — links its rows the same way;
//! a second trigger refuses UPDATE, DELETE and TRUNCATE (deletes only through the retention purge).

use serde::Serialize;
use serde_json::Value;

use crate::app::AppState;
use crate::auth::{Client, User};

/// Records an event. Failures are logged, never returned: auditing must not break the action,
/// but a failed write is an error worth alerting on.
pub async fn log(state: &AppState, actor: Option<&User>, client: Option<&Client>, action: &str, target: Option<(&str, String)>, data: Value) {
    let (target_type, target_id) = match target {
        Some((t, id)) => (Some(t), Some(id)),
        None => (None, None),
    };
    let res = sqlx::query(
        "INSERT INTO audit_events (actor_id, actor_label, ip, user_agent, action, target_type, target_id, data, prev_hash, hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '\\x', '\\x')",
    )
    .bind(actor.map(|u| u.id))
    .bind(actor.map(User::label))
    .bind(client.and_then(|c| c.ip.clone()))
    .bind(client.and_then(|c| c.user_agent.clone()))
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(data)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        tracing::error!(action, "audit log write failed: {e}");
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    pub ok: bool,
    pub checked: i64,
    /// first row whose hash doesn't match its content or doesn't link to the row before
    pub first_bad_id: Option<i64>,
    /// hash of the newest row (write it down somewhere else to detect a rewrite of the whole chain)
    pub head: Option<String>,
    pub head_id: Option<i64>,
}

/// Recomputes every row's hash and checks the links. The oldest remaining row is the anchor
/// (rows before it may have been purged by the retention policy).
pub async fn verify(state: &AppState) -> anyhow::Result<Verification> {
    let (checked, first_bad_id): (i64, Option<i64>) = sqlx::query_as(
        "WITH c AS (
            SELECT id,
                   hash = audit_digest(prev_hash, id, at, actor_id, actor_label, ip, user_agent, action, target_type, target_id, data) AS ok,
                   coalesce(prev_hash = lag(hash) OVER (ORDER BY id), true) AS linked
              FROM audit_events)
         SELECT count(*), min(id) FILTER (WHERE NOT ok OR NOT linked) FROM c",
    )
    .fetch_one(&state.db)
    .await?;
    let head: Option<(i64, Vec<u8>)> = sqlx::query_as("SELECT id, hash FROM audit_events ORDER BY id DESC LIMIT 1").fetch_optional(&state.db).await?;
    Ok(Verification { ok: first_bad_id.is_none(), checked, first_bad_id, head: head.as_ref().map(|h| hex::encode(&h.1)), head_id: head.map(|h| h.0) })
}

/// Deletes audit rows older than `days` (the retention policy). The chain stays verifiable from
/// the oldest remaining row on.
pub async fn purge_older_than(state: &AppState, days: u32) -> anyhow::Result<u64> {
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT set_config('agora.audit_purge', 'on', true)").execute(&mut *tx).await?;
    let n = sqlx::query("DELETE FROM audit_events WHERE at < now() - make_interval(days => $1)").bind(days as i32).execute(&mut *tx).await?.rows_affected();
    tx.commit().await?;
    Ok(n)
}
