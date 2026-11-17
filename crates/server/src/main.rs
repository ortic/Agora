use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use agora::app::AppState;
use agora::config::Config;

#[derive(Parser)]
#[command(name = "agora", version, about = "Agora — an internal AI platform")]
struct Cli {
    #[command(flatten)]
    config: Config,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web server (default).
    Serve,
    /// Apply database migrations and exit.
    Migrate,
    /// Check the audit log's hash chain; exits non-zero when it is broken.
    VerifyAudit,
    /// Probe the running server's `/api/health` (for container HEALTHCHECKs).
    Healthcheck,
}

/// Plain-TCP GET of `/api/health` on the configured bind address (no curl needed in the image).
fn healthcheck(bind: std::net::SocketAddr) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use std::time::Duration;
    let mut addr = bind;
    if addr.ip().is_unspecified() {
        addr.set_ip(if addr.is_ipv4() { std::net::Ipv4Addr::LOCALHOST.into() } else { std::net::Ipv6Addr::LOCALHOST.into() });
    }
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    s.write_all(format!("GET /api/health HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes())?;
    let mut body = String::new();
    s.take(64 * 1024).read_to_string(&mut body)?;
    anyhow::ensure!(body.starts_with("HTTP/1.") && body.contains(" 200 ") && body.contains("\"ok\""), "unhealthy: {}", body.lines().next().unwrap_or(""));
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(Command::Healthcheck) = cli.command {
        return healthcheck(cli.config.bind);
    }
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn"));
    if cli.config.log_format.eq_ignore_ascii_case("json") {
        tracing_subscriber::fmt().json().flatten_event(true).with_current_span(true).with_span_list(false).with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    match cli.command.unwrap_or(Command::Serve) {
        Command::Healthcheck => unreachable!(),
        Command::Migrate => {
            AppState::new(cli.config).await?;
            println!("migrations applied");
        }
        Command::VerifyAudit => {
            let state = AppState::new(cli.config).await?;
            let v = agora::audit::verify(&state).await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
            anyhow::ensure!(v.ok, "the audit log's hash chain is broken at event {:?}", v.first_bad_id);
        }
        Command::Serve => {
            let bind = cli.config.bind;
            let state = AppState::new(cli.config).await?;
            agora::chat::recover(&state).await?;
            agora::maintenance::spawn(&state);
            let app = agora::routes::router(state.clone());
            let listener = tokio::net::TcpListener::bind(bind).await?;
            tracing::info!("Agora listening on http://{bind}; web app: {}", agora::web_assets::source(&state.config.static_dir));
            axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(shutdown(state)).await?;
        }
    }
    Ok(())
}

/// Waits for Ctrl-C or SIGTERM, then stops the answers being generated (they are stored as stopped).
async fn shutdown(state: AppState) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("shutting down");
    state.shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
    for live in state.live.iter() {
        live.cancel.cancel();
    }
    // let the answers be stored
    for _ in 0..50 {
        if state.live.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
