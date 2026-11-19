use std::sync::Arc;

use dashmap::DashMap;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<Inner>,
}

pub struct Inner {
    pub db: PgPool,
    pub config: Config,
    /// configured external sign-in providers
    pub providers: Vec<crate::sso::Provider>,
    pub secrets: crate::crypto::Secrets,
    /// answers being generated on this replica, by assistant message id
    pub live: DashMap<Uuid, crate::chat::Live>,
    /// desktop apps connected to this replica, by device id
    pub devices: DashMap<Uuid, Arc<crate::desktop::Device>>,
    /// answers Claude Code is writing on this replica, by the token of their MCP endpoint
    pub mcp_runs: DashMap<String, Arc<crate::chat::McpRun>>,
    /// set on SIGTERM
    pub shutting_down: std::sync::atomic::AtomicBool,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.inner
    }
}

impl AppState {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        let db =
            sqlx::postgres::PgPoolOptions::new().max_connections(30).acquire_timeout(std::time::Duration::from_secs(10)).connect(&config.database_url).await?;
        sqlx::migrate!("./migrations").run(&db).await?;
        sqlx::query("INSERT INTO settings (id) VALUES (true) ON CONFLICT DO NOTHING").execute(&db).await?;
        let providers = crate::sso::load(&config)?;
        let secrets = crate::crypto::Secrets::new(&config.secret_key);
        Ok(AppState {
            inner: Arc::new(Inner {
                db,
                config,
                providers,
                secrets,
                live: DashMap::new(),
                devices: DashMap::new(),
                mcp_runs: DashMap::new(),
                shutting_down: Default::default(),
            }),
        })
    }
}

pub fn new_id() -> Uuid {
    Uuid::now_v7()
}
