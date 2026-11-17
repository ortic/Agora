//! Hourly housekeeping: retention purges and expired sign-in state.

use std::time::Duration;

use serde_json::json;

use crate::app::AppState;

pub fn spawn(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        // a little after start-up, then every hour
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            if let Err(e) = run(&state).await {
                tracing::error!("maintenance failed: {e:#}");
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

pub async fn run(state: &AppState) -> anyhow::Result<()> {
    let s = crate::settings::load(state).await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at < now()").execute(&state.db).await?;
    sqlx::query("DELETE FROM auth_states WHERE expires_at < now()").execute(&state.db).await?;
    sqlx::query("DELETE FROM login_attempts WHERE window_start < now() - interval '1 day'").execute(&state.db).await?;
    if let Some(days) = s.chat_retention_days {
        let n = sqlx::query("DELETE FROM chats WHERE deleted_at < now() - make_interval(days => $1)").bind(days as i32).execute(&state.db).await?.rows_affected();
        if n > 0 {
            crate::audit::log(state, None, None, "retention.chats_purged", None, json!({ "chats": n, "days": days })).await;
        }
    }
    if let Some(days) = s.audit_retention_days.filter(|d| *d > 0) {
        let n = crate::audit::purge_older_than(state, days).await?;
        if n > 0 {
            crate::audit::log(state, None, None, "retention.audit_purged", None, json!({ "events": n, "days": days })).await;
        }
    }
    Ok(())
}
