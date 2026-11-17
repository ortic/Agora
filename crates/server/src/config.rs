use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Args;

/// Server configuration (CLI flags or `AGORA_*` environment variables). Everything an
/// administrator can change at runtime (models, budgets, web search, retention) lives in the
/// database instead and is edited in the admin UI.
#[derive(Debug, Clone, Args)]
pub struct Config {
    /// PostgreSQL connection URL.
    #[arg(long, env = "AGORA_DATABASE_URL", default_value = "postgres://agora:agora@127.0.0.1:55433/agora")]
    pub database_url: String,

    /// Address to listen on.
    #[arg(long, env = "AGORA_BIND", default_value = "127.0.0.1:8790")]
    pub bind: SocketAddr,

    /// Public base URL (SSO redirect URIs, absolute links).
    #[arg(long, env = "AGORA_BASE_URL", default_value = "http://127.0.0.1:8790")]
    pub base_url: String,

    /// Directory with the built frontend (`web/dist`). Served at `/`; the copy embedded in the
    /// binary is used when the directory has no build.
    #[arg(long, env = "AGORA_STATIC_DIR", default_value = "./web/dist")]
    pub static_dir: PathBuf,

    /// Key that encrypts provider API keys in the database (any string; 32+ random characters
    /// recommended, e.g. `openssl rand -hex 32`). Changing it makes stored keys unreadable.
    #[arg(long, env = "AGORA_SECRET_KEY", hide_env_values = true, default_value = "")]
    pub secret_key: String,

    /// Mark session cookies `Secure` (enable behind HTTPS).
    #[arg(long, env = "AGORA_SECURE_COOKIES", default_value_t = false)]
    pub secure_cookies: bool,

    /// Session lifetime in days.
    #[arg(long, env = "AGORA_SESSION_DAYS", default_value_t = 14)]
    pub session_days: i64,

    /// Sign out sessions inactive for this many minutes (0 = only the absolute lifetime).
    #[arg(long, env = "AGORA_SESSION_IDLE_MINUTES", default_value_t = 0)]
    pub session_idle_minutes: u64,

    /// External sign-in providers as a JSON list (OIDC, GitHub, LDAP, SAML; see README).
    #[arg(long, env = "AGORA_AUTH_PROVIDERS", hide_env_values = true)]
    pub auth_providers: Option<String>,

    /// File with the provider list (JSON), e.g. a mounted Kubernetes secret.
    #[arg(long, env = "AGORA_AUTH_PROVIDERS_FILE")]
    pub auth_providers_file: Option<PathBuf>,

    /// Turn off local passwords (SSO only; LDAP still signs in with username and password).
    #[arg(long, env = "AGORA_DISABLE_PASSWORD_LOGIN", default_value_t = false)]
    pub disable_password_login: bool,

    /// Let `fetch_url` and model providers reach private / loopback addresses for page fetches
    /// (off by default: SSRF protection). Providers themselves may always be private.
    #[arg(long, env = "AGORA_FETCH_ALLOW_PRIVATE", default_value_t = false)]
    pub fetch_allow_private: bool,

    /// Seconds a model may take to start answering (and between streamed chunks).
    #[arg(long, env = "AGORA_MODEL_TIMEOUT_SECS", default_value_t = 300)]
    pub model_timeout_secs: u64,

    /// Log format: `text` or `json`.
    #[arg(long, env = "AGORA_LOG_FORMAT", default_value = "text")]
    pub log_format: String,

    /// Trust `X-Forwarded-For` / `X-Real-IP` for client addresses (behind a reverse proxy).
    #[arg(long, env = "AGORA_TRUST_PROXY", default_value_t = false)]
    pub trust_proxy: bool,
}
