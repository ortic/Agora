pub mod admin;
pub mod auth;
pub mod chats;
pub mod connectors;
pub mod desktop;
pub mod mcp;
pub mod openai_api;
pub mod projects;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post};

use crate::app::AppState;

/// Content-Security-Policy for the app.
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data:; \
                       connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

pub fn api() -> Router<AppState> {
    Router::new()
        // setup & sign-in
        .route("/setup", get(auth::setup_status).post(auth::setup))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/providers", get(crate::sso::list))
        .route("/auth/sso/{id}/login", get(crate::sso::oidc::sso_login))
        .route("/auth/sso/{id}/callback", get(crate::sso::oidc::sso_callback))
        .route("/me", get(auth::me))
        .route("/me/usage", get(auth::my_usage))
        .route("/me/tokens", get(auth::list_tokens).post(auth::create_token))
        .route("/me/tokens/{id}", delete(auth::delete_token))
        // chats
        .route("/models", get(chats::models))
        .route("/chats", get(chats::list).post(chats::create))
        .route("/chats/{id}", get(chats::get).patch(chats::update).delete(chats::delete))
        .route("/chats/{id}/messages", post(chats::send))
        .route("/chats/{id}/regenerate", post(chats::regenerate))
        .route("/messages/{id}/stream", get(chats::follow))
        .route("/messages/{id}/stop", post(chats::stop))
        .route("/attachments/{id}", get(chats::attachment))
        // projects: chats that belong together, with instructions and files
        .route("/projects", get(projects::list).post(projects::create))
        .route("/projects/{id}", get(projects::get).patch(projects::update).delete(projects::delete))
        .route("/projects/{id}/files", post(projects::add_file))
        .route("/projects/{id}/files/{file}", get(projects::file).delete(projects::delete_file))
        // Claude Code's way to Agora's tools while it writes an answer (the token is the credential)
        .route("/mcp/{token}", post(crate::chat::mcp_endpoint))
        // knowledge sources (RAG)
        .route("/connectors", get(connectors::list))
        .route("/connectors/{id}/token", axum::routing::put(connectors::connect).delete(connectors::disconnect))
        .route("/admin/connectors", get(connectors::admin_list).post(connectors::admin_create))
        .route("/admin/connectors/{id}", patch(connectors::admin_update).delete(connectors::admin_delete))
        // MCP servers of the organization: tools for models, calls people confirm
        .route("/mcp-servers", get(mcp::mine))
        .route("/tool-calls/{id}/decision", post(mcp::decide))
        .route("/admin/mcp-servers", get(mcp::admin_list).post(mcp::admin_create))
        .route("/admin/mcp-servers/check", post(mcp::admin_check))
        .route("/admin/mcp-servers/{id}", patch(mcp::admin_update).delete(mcp::admin_delete))
        // Agora Desktop: signing a device in, its connection, administration
        .route("/desktop/login/start", post(desktop::login_start))
        .route("/desktop/login/poll", post(desktop::login_poll))
        .route("/desktop/login/{code}", get(desktop::login_info).post(desktop::login_decide))
        .route("/desktop/policy", get(desktop::my_policy))
        .route("/desktop/ws", get(desktop::ws))
        .route("/me/devices", get(desktop::my_devices))
        .route("/me/devices/{id}", delete(desktop::revoke_my_device))
        .route("/admin/desktop", get(desktop::admin_get))
        .route("/admin/desktop/features/{key}", axum::routing::put(desktop::admin_put_feature))
        .route("/admin/desktop/connectors", post(desktop::admin_create_connector))
        .route("/admin/desktop/connectors/{id}", patch(desktop::admin_update_connector).delete(desktop::admin_delete_connector))
        .route("/admin/desktop/devices/{id}", delete(desktop::admin_revoke_device))
        .route("/admin/desktop/tools", get(desktop::admin_tool_events))
        // OpenAI-compatible API (base URL: /api/v1)
        .route("/v1/models", get(openai_api::models))
        .route("/v1/chat/completions", post(openai_api::completions))
        // administration
        .route("/admin/overview", get(admin::overview))
        .route("/admin/users", get(admin::list_users).post(admin::create_user))
        .route("/admin/users/{id}", patch(admin::update_user))
        .route("/admin/users/{id}/chats", get(admin::user_chats))
        .route("/admin/groups", get(admin::list_groups).post(admin::create_group))
        .route("/admin/groups/{id}", patch(admin::update_group).delete(admin::delete_group))
        .route("/admin/providers", get(admin::list_providers).post(admin::create_provider))
        .route("/admin/providers/{id}", patch(admin::update_provider).delete(admin::delete_provider))
        .route("/admin/providers/{id}/discover", get(admin::discover))
        .route("/admin/models", get(admin::list_models).post(admin::create_model))
        .route("/admin/models/{id}", patch(admin::update_model).delete(admin::delete_model))
        .route("/admin/settings", get(admin::get_settings).put(admin::put_settings))
        .route("/admin/audit", get(admin::audit_log))
        .route("/admin/audit/verify", get(admin::audit_verify))
        .route("/admin/audit/export", get(admin::audit_export))
        .route("/admin/usage", get(admin::usage_summary))
        .route("/admin/usage/export", get(admin::usage_export))
        .route("/admin/web", get(admin::web_events))
        .route("/admin/chats/{id}", get(admin::inspect_chat))
        .route("/health", get(health))
        .route("/ready", get(ready))
        // messages can carry long documents and images
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "status": "ok" }))
}

async fn ready(axum::extract::State(state): axum::extract::State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let ok = !state.shutting_down.load(std::sync::atomic::Ordering::SeqCst) && sqlx::query("SELECT 1").execute(&state.db).await.is_ok();
    let code = if ok { axum::http::StatusCode::OK } else { axum::http::StatusCode::SERVICE_UNAVAILABLE };
    (code, axum::Json(serde_json::json!({ "status": if ok { "ok" } else { "unavailable" } }))).into_response()
}

pub fn router(state: AppState) -> Router {
    let spa = crate::web_assets::service(&state.config.static_dir);
    let hsts = state.config.base_url.starts_with("https://").then(|| {
        tower_http::set_header::SetResponseHeaderLayer::if_not_present(
            axum::http::header::STRICT_TRANSPORT_SECURITY,
            axum::http::HeaderValue::from_static("max-age=31536000"),
        )
    });
    let app = Router::new()
        .nest("/api", saml_routes(api()))
        .fallback_service(spa)
        .layer(tower_http::compression::CompressionLayer::new().compress_when(NotEventStream))
        .layer(tower_http::request_id::PropagateRequestIdLayer::x_request_id())
        .layer(tower_http::trace::TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
            let id = req.headers().get("x-request-id").and_then(|v| v.to_str().ok()).unwrap_or("-");
            tracing::info_span!("http", method = %req.method(), path = %req.uri().path(), request_id = %id)
        }))
        .layer(tower_http::request_id::SetRequestIdLayer::x_request_id(tower_http::request_id::MakeRequestUuid))
        .layer(tower_http::catch_panic::CatchPanicLayer::new())
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        ))
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(
            axum::http::header::REFERRER_POLICY,
            axum::http::HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(
            axum::http::header::X_FRAME_OPTIONS,
            axum::http::HeaderValue::from_static("DENY"),
        ))
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(
            axum::http::header::CONTENT_SECURITY_POLICY,
            axum::http::HeaderValue::from_static(CSP),
        ));
    match hsts {
        Some(h) => app.layer(h),
        None => app,
    }
    .with_state(state)
}

/// Streamed answers must reach the browser chunk by chunk: no compression for event streams.
#[derive(Clone, Copy)]
struct NotEventStream;

impl tower_http::compression::predicate::Predicate for NotEventStream {
    fn should_compress<B: axum::body::HttpBody>(&self, response: &axum::http::Response<B>) -> bool {
        let sse = response.headers().get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/event-stream"));
        !sse && tower_http::compression::predicate::DefaultPredicate::new().should_compress(response)
    }
}

/// SAML endpoints (assertion consumer service + SP metadata) when built with `--features saml`.
#[cfg(feature = "saml")]
fn saml_routes(api: Router<AppState>) -> Router<AppState> {
    api.route("/auth/sso/{id}/acs", post(crate::sso::saml::acs)).route("/auth/sso/{id}/metadata", get(crate::sso::saml::metadata))
}

#[cfg(not(feature = "saml"))]
fn saml_routes(api: Router<AppState>) -> Router<AppState> {
    api
}
