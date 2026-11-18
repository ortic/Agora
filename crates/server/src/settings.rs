//! Organisation-wide settings, edited in the admin UI (stored as JSON in the `settings` row).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Organisation name (header, page title).
    pub name: String,
    /// What the audit log keeps of prompts and answers: `full` (the text) or `metadata`
    /// (sizes, model, ids; the conversation itself stays in the chat tables either way).
    pub audit_content: String,
    /// Days after which chats people deleted are purged for good (`None`: never).
    pub chat_retention_days: Option<u32>,
    /// Days after which audit events are purged (`None`: keep forever).
    pub audit_retention_days: Option<u32>,
    /// Monthly budget per person in USD unless a group or the person has their own (`None`: unlimited).
    pub default_monthly_budget_usd: Option<f64>,
    /// Model requests per person per minute (`None`: unlimited).
    pub rate_limit_per_minute: Option<u32>,
    /// Model new chats start with.
    pub default_model_id: Option<Uuid>,
    /// Model that writes chat titles (`None`: the chat's own model; titles cost a little).
    pub title_model_id: Option<Uuid>,
    /// Instructions put in front of every conversation (organisation policy, tone, …).
    pub system_prompt: Option<String>,
    /// Notice shown above the message box (e.g. "Conversations are logged. No customer data.").
    pub notice: Option<String>,
    /// May people set a system prompt of their own per chat?
    pub allow_user_system_prompt: bool,
    pub web_search: WebSearch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebSearch {
    /// `none`, `searxng`, `brave`, `serper` or `tavily`
    pub engine: String,
    /// SearXNG base URL (`https://searx.example.com`)
    pub url: Option<String>,
    /// encrypted API key (Brave, Tavily), base64; never sent to the browser
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_enc: Option<String>,
    pub max_results: u32,
    /// Let models open web pages (`fetch_url`).
    pub fetch_enabled: bool,
    /// Use Claude's own server-side web search for Claude models instead of the engine above.
    pub claude_native: bool,
    /// Domains never searched or fetched (subdomains included).
    pub blocked_domains: Vec<String>,
    /// Tool calls per answer.
    pub max_tool_calls: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: "Agora".into(),
            audit_content: "full".into(),
            chat_retention_days: Some(90),
            audit_retention_days: None,
            default_monthly_budget_usd: None,
            rate_limit_per_minute: Some(30),
            default_model_id: None,
            title_model_id: None,
            system_prompt: None,
            notice: None,
            allow_user_system_prompt: true,
            web_search: WebSearch::default(),
        }
    }
}

impl Default for WebSearch {
    fn default() -> Self {
        WebSearch {
            engine: "none".into(),
            url: None,
            api_key_enc: None,
            max_results: 6,
            fetch_enabled: true,
            claude_native: false,
            blocked_domains: vec![],
            max_tool_calls: 6,
        }
    }
}

impl WebSearch {
    /// Whether any kind of web access is available (an engine, or Claude's own search).
    pub fn available(&self) -> bool {
        self.engine != "none" || self.claude_native
    }

    pub fn blocked(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.blocked_domains
            .iter()
            .map(|d| d.trim().trim_start_matches("*.").to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .any(|d| host == d || host.ends_with(&format!(".{d}")))
    }
}

pub async fn load(state: &AppState) -> anyhow::Result<Settings> {
    let v: serde_json::Value = sqlx::query_scalar("SELECT data FROM settings WHERE id").fetch_one(&state.db).await?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

pub async fn save(state: &AppState, s: &Settings) -> anyhow::Result<()> {
    sqlx::query("UPDATE settings SET data = $1, updated_at = now() WHERE id").bind(serde_json::to_value(s)?).execute(&state.db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn blocked_domains_cover_subdomains() {
        let w = super::WebSearch { blocked_domains: vec!["example.com".into(), "*.bad.org".into()], ..Default::default() };
        assert!(w.blocked("example.com"));
        assert!(w.blocked("www.Example.com."));
        assert!(w.blocked("x.bad.org"));
        assert!(!w.blocked("notexample.com"));
    }
}
