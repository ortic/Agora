//! Usage metering: one row per model call, budgets and rate limits.

use serde::Serialize;
use uuid::Uuid;

use crate::app::{AppState, new_id};
use crate::auth::User;
use crate::catalog::ModelRow;
use crate::error::{ApiError, ApiResult};
use crate::llm::Usage;
use crate::settings::Settings;

pub struct Record<'a> {
    pub user: Option<Uuid>,
    pub chat: Option<Uuid>,
    pub message: Option<Uuid>,
    pub model: &'a ModelRow,
    pub source: &'a str,
    pub usage: Usage,
    pub latency_ms: i64,
    pub status: &'a str,
    pub error: Option<String>,
}

/// Stores a usage event; returns its cost.
pub async fn record(state: &AppState, r: Record<'_>) -> f64 {
    record_reported(state, r, 0.0).await
}

/// Stores a usage event whose provider says what it cost (`reported`): that counts when the
/// model has no prices of its own, so budgets hold for it too.
pub async fn record_reported(state: &AppState, r: Record<'_>, reported: f64) -> f64 {
    let priced = r.model.cost(&r.usage);
    let cost = if priced > 0.0 { priced } else { reported.max(0.0) };
    let res = sqlx::query(
        "INSERT INTO usage_events (id, user_id, chat_id, message_id, model_id, provider_id, model_name, source, input_tokens, output_tokens,
                                   cache_read_tokens, cache_write_tokens, provider_searches, cost_usd, latency_ms, status, error)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
    )
    .bind(new_id())
    .bind(r.user)
    .bind(r.chat)
    .bind(r.message)
    .bind(r.model.id)
    .bind(r.model.provider_id)
    .bind(&r.model.model)
    .bind(r.source)
    .bind(r.usage.input)
    .bind(r.usage.output)
    .bind(r.usage.cache_read)
    .bind(r.usage.cache_write)
    .bind(r.usage.searches as i32)
    .bind(cost)
    .bind(r.latency_ms.min(i32::MAX as i64) as i32)
    .bind(r.status)
    .bind(r.error)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        tracing::error!("usage record failed: {e}");
    }
    cost
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    /// USD per month (`None`: unlimited)
    pub limit: Option<f64>,
    /// where the limit comes from: user | group | default | none
    pub source: &'static str,
    pub spent: f64,
}

pub async fn budget(state: &AppState, user_id: Uuid, settings: &Settings) -> ApiResult<Budget> {
    let (own, group): (Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT u.monthly_budget_usd,
                (SELECT max(g.monthly_budget_usd) FROM groups g JOIN group_members gm ON gm.group_id = g.id WHERE gm.user_id = u.id)
           FROM users u WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    let spent: f64 =
        sqlx::query_scalar("SELECT coalesce(sum(cost_usd), 0)::float8 FROM usage_events WHERE user_id = $1 AND created_at >= date_trunc('month', now())")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;
    let (limit, source) = match (own, group, settings.default_monthly_budget_usd) {
        (Some(l), _, _) => (Some(l), "user"),
        (None, Some(l), _) => (Some(l), "group"),
        (None, None, Some(l)) => (Some(l), "default"),
        _ => (None, "none"),
    };
    Ok(Budget { limit, source, spent })
}

/// Refuses a model request when the person's monthly budget is used up or they send requests
/// faster than the rate limit allows.
pub async fn check(state: &AppState, user: &User, settings: &Settings) -> ApiResult<()> {
    if let Some(per_minute) = settings.rate_limit_per_minute.filter(|n| *n > 0) {
        let recent: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT coalesce(message_id, id)) FROM usage_events WHERE user_id = $1 AND created_at > now() - interval '1 minute' AND source <> 'title'",
        )
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;
        if recent >= per_minute as i64 {
            return Err(ApiError::Limit(format!("you've sent {per_minute} requests in the last minute; wait a moment")));
        }
    }
    let b = budget(state, user.id, settings).await?;
    if let Some(limit) = b.limit
        && b.spent >= limit
    {
        return Err(ApiError::Limit(format!("your monthly budget of ${limit:.2} is used up (${:.2} spent); ask an administrator to raise it", b.spent)));
    }
    Ok(())
}
