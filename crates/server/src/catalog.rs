//! The models people can use: access rules, provider endpoints, prices.

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::User;
use crate::error::{ApiError, ApiResult};
use crate::llm::{Endpoint, Kind, Usage};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub model: String,
    pub display_name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub input_price: f64,
    pub output_price: f64,
    pub cache_read_price: f64,
    pub cache_write_price: f64,
    pub search_price: f64,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
    pub capabilities: Value,
    pub params: Value,
    pub access: String,
    pub position: i32,
    pub provider_kind: String,
    pub provider_name: String,
    pub provider_enabled: bool,
}

pub const MODEL_SELECT: &str = "SELECT m.id, m.provider_id, m.model, m.display_name, m.description, m.enabled, m.input_price, m.output_price,
        m.cache_read_price, m.cache_write_price, m.search_price, m.context_window, m.max_output_tokens, m.capabilities, m.params, m.access,
        m.position, p.kind AS provider_kind, p.name AS provider_name, p.enabled AS provider_enabled
   FROM models m JOIN providers p ON p.id = m.provider_id";

/// Rule for "may this user use the model" (`$1` = user id): enabled, provider enabled, and open
/// to everyone or to one of the user's groups. Administrators may use every enabled model.
const ACCESS: &str = "m.enabled AND p.enabled AND (m.access = 'all'
    OR EXISTS (SELECT 1 FROM users u WHERE u.id = $1 AND u.role IN ('owner', 'admin'))
    OR EXISTS (SELECT 1 FROM model_groups mg JOIN group_members gm ON gm.group_id = mg.group_id WHERE mg.model_id = m.id AND gm.user_id = $1))";

impl ModelRow {
    pub fn kind(&self) -> Kind {
        Kind::parse(&self.provider_kind).unwrap_or(Kind::Openai)
    }

    pub fn cap(&self, name: &str) -> bool {
        self.capabilities[name].as_bool().unwrap_or(match name {
            // tools work on nearly every current model; reasoning display on Claude
            "tools" => true,
            "reasoning" => self.kind() == Kind::Anthropic && crate::llm::presets::claude_thinks(&self.model),
            _ => false,
        })
    }

    pub fn param_str(&self, name: &str) -> Option<String> {
        self.params[name].as_str().filter(|s| !s.trim().is_empty()).map(String::from)
    }

    /// USD for one model call.
    pub fn cost(&self, u: &Usage) -> f64 {
        (u.input as f64 * self.input_price
            + u.output as f64 * self.output_price
            + u.cache_read as f64 * self.cache_read_price
            + u.cache_write as f64 * self.cache_write_price)
            / 1_000_000.0
            + u.searches as f64 * self.search_price
    }
}

pub async fn accessible(state: &AppState, user: &User) -> ApiResult<Vec<ModelRow>> {
    Ok(sqlx::query_as::<_, ModelRow>(sqlx::AssertSqlSafe(format!("{MODEL_SELECT} WHERE {ACCESS} ORDER BY m.position, m.display_name"))).bind(user.id).fetch_all(&state.db).await?)
}

/// The model if the user may use it (404 otherwise: no hint that it exists).
pub async fn for_user(state: &AppState, user: &User, id: Uuid) -> ApiResult<ModelRow> {
    sqlx::query_as::<_, ModelRow>(sqlx::AssertSqlSafe(format!("{MODEL_SELECT} WHERE {ACCESS} AND m.id = $2")))
        .bind(user.id)
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("model"))
}

/// By the name the OpenAI-compatible API uses (`model` at the provider, or `provider/model`).
pub async fn for_user_by_name(state: &AppState, user: &User, name: &str) -> ApiResult<ModelRow> {
    let all = accessible(state, user).await?;
    if let Ok(id) = name.parse::<Uuid>()
        && let Some(m) = all.iter().find(|m| m.id == id)
    {
        return Ok(m.clone());
    }
    all.iter()
        .find(|m| api_name(m, &all) == name)
        .or_else(|| all.iter().find(|m| m.model == name))
        .cloned()
        .ok_or_else(|| ApiError::NotFound("model"))
}

/// Name of a model in the OpenAI-compatible API: its provider-side name, prefixed with the
/// provider when two providers offer the same name.
pub fn api_name(m: &ModelRow, all: &[ModelRow]) -> String {
    if all.iter().filter(|o| o.model == m.model).count() > 1 { format!("{}/{}", slug(&m.provider_name), m.model) } else { m.model.clone() }
}

fn slug(s: &str) -> String {
    s.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').to_string()
}

pub async fn endpoint(state: &AppState, provider_id: Uuid) -> ApiResult<Endpoint> {
    let (kind, base_url, key_enc, headers): (String, String, Option<Vec<u8>>, Value) =
        sqlx::query_as("SELECT kind, base_url, api_key_enc, headers FROM providers WHERE id = $1").bind(provider_id).fetch_one(&state.db).await?;
    let api_key = match key_enc {
        Some(k) => Some(state.secrets.decrypt(&k).map_err(ApiError::Internal)?),
        None => None,
    };
    let headers = headers.as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect()).unwrap_or_default();
    Ok(Endpoint { kind: Kind::parse(&kind).unwrap_or(Kind::Openai), base_url, api_key, headers, timeout_secs: state.config.model_timeout_secs })
}
