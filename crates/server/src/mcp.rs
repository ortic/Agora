//! MCP servers of the organization: Model Context Protocol servers this server connects to, so
//! that models can use their tools in any chat (the web app included). Administrators define
//! them like local connectors (a program the server starts, or a URL), choose which tools
//! models get, whether people confirm each call, and who may use them.
//!
//! `mcp_stdio` servers are programs started here and talked to over stdin/stdout (one JSON-RPC
//! message per line), with an empty environment except for what is configured; `mcp_http`
//! servers are reached at a URL (streamable HTTP: a POST per message, answered with JSON or a
//! short event stream). A server is started on first use and kept for later calls; changing
//! its settings starts it anew.
//!
//! The server's credentials (environment, headers) are the organization's: every person who
//! is granted a server acts with them. They are stored encrypted and never shown again.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dashmap::DashMap;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app::AppState;
use crate::llm::{Media, ToolDef};

const PROTOCOL_VERSION: &str = "2025-06-18";
/// Starting a server and listing its tools.
const START_TIMEOUT: Duration = Duration::from_secs(30);
/// One tool call.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a person has to confirm a call.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);
/// The most a tool result may be when it goes to the model.
const MAX_RESULT_CHARS: usize = 60_000;
/// Environment variables a started program gets from the server's own (nothing else of it:
/// the server's environment holds its database URL and secret key).
const PASSED_ENV: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TZ", "TMPDIR", "SSL_CERT_FILE", "SSL_CERT_DIR", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"];

// ---- what is stored -------------------------------------------------------------------------

/// The secret part of a server's settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Secrets {
    pub env: HashMap<String, String>,
    pub headers: HashMap<String, String>,
}

impl Secrets {
    pub fn seal(&self, state: &AppState) -> anyhow::Result<Option<Vec<u8>>> {
        if self.env.is_empty() && self.headers.is_empty() {
            return Ok(None);
        }
        Ok(Some(state.secrets.encrypt(&serde_json::to_string(self)?)?))
    }

    pub fn open(state: &AppState, sealed: Option<&[u8]>) -> Secrets {
        sealed.and_then(|s| state.secrets.decrypt(s).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
}

/// A server as defined by administrators, with its secrets.
#[derive(Debug, Clone)]
pub struct Def {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    pub config: Value,
    pub secrets: Secrets,
    /// allowed tools (empty: all)
    pub tools: Vec<String>,
    pub approval: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

type DefRow = (Uuid, String, String, Option<String>, Value, Option<Vec<u8>>, Vec<String>, String, chrono::DateTime<chrono::Utc>);

/// The enabled servers `user` may use.
pub async fn granted(state: &AppState, user: Uuid) -> anyhow::Result<Vec<Def>> {
    let rows: Vec<DefRow> = sqlx::query_as(
        "SELECT s.id, s.name, s.kind, s.description, s.config, s.secrets_enc, s.tools, s.approval, s.updated_at FROM mcp_servers s
          WHERE s.enabled AND (s.access = 'all' OR EXISTS (
                SELECT 1 FROM mcp_server_groups sg JOIN group_members gm ON gm.group_id = sg.group_id WHERE sg.server_id = s.id AND gm.user_id = $1))
          ORDER BY s.name",
    )
    .bind(user)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, kind, description, config, sealed, tools, approval, updated_at)| Def {
            id,
            name,
            kind,
            description,
            config,
            secrets: Secrets::open(state, sealed.as_deref()),
            tools,
            approval,
            updated_at,
        })
        .collect())
}

// ---- a connection ---------------------------------------------------------------------------

/// A tool as the server lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTool {
    pub name: String,
    pub description: String,
    #[serde(skip)]
    pub schema: Value,
    /// the server says it only reads (`annotations.readOnlyHint`)
    pub read_only: bool,
}

type Waiting = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

enum Transport {
    Stdio {
        // kept so the program ends with the connection (kill_on_drop)
        _child: tokio::process::Child,
        stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
        waiting: Waiting,
        stderr: Arc<Mutex<String>>,
    },
    Http {
        url: String,
        headers: Vec<(String, String)>,
        session: Mutex<Option<String>>,
        http: reqwest::Client,
    },
}

/// A started (or reachable) MCP server.
pub struct Conn {
    pub name: String,
    transport: Transport,
    next_id: AtomicU64,
    /// false once the program ended or the server forgot our session
    alive: Arc<AtomicBool>,
    pub tools: Vec<RemoteTool>,
    /// the settings it was started with
    version: chrono::DateTime<chrono::Utc>,
}

/// What a call returned.
pub struct Output {
    pub text: String,
    pub is_error: bool,
    pub media: Vec<Media>,
}

impl Conn {
    /// Starts the server (or reaches it) and asks for its tools. `dir`: where programs run.
    pub async fn start(def: &Def, dir: &std::path::Path, stdio_allowed: bool) -> Result<Conn, String> {
        let alive = Arc::new(AtomicBool::new(true));
        let transport = match def.kind.as_str() {
            "mcp_stdio" if !stdio_allowed => return Err("starting programs is turned off on this server (AGORA_MCP_STDIO)".into()),
            "mcp_stdio" => spawn(def, dir, alive.clone()).await?,
            "mcp_http" => {
                let url = def.config["url"].as_str().unwrap_or_default().to_string();
                if url.is_empty() {
                    return Err("the server has no URL".into());
                }
                let http = reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(10))
                    .timeout(CALL_TIMEOUT + Duration::from_secs(30))
                    .build()
                    .map_err(|e| e.to_string())?;
                Transport::Http { url, headers: def.secrets.headers.clone().into_iter().collect(), session: Mutex::new(None), http }
            }
            other => return Err(format!("unknown kind {other}")),
        };
        let mut conn = Conn { name: def.name.clone(), transport, next_id: AtomicU64::new(1), alive, tools: vec![], version: def.updated_at };
        let none = CancellationToken::new();
        let init = conn
            .request(
                "initialize",
                json!({ "protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": { "name": "Agora", "version": env!("CARGO_PKG_VERSION") } }),
                START_TIMEOUT,
                &none,
            )
            .await?;
        if init["serverInfo"].is_null() && init["capabilities"].is_null() {
            return Err("it didn't answer as an MCP server".into());
        }
        conn.notify("notifications/initialized", json!({})).await?;
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let page = conn.request("tools/list", params, START_TIMEOUT, &none).await?;
            for t in page["tools"].as_array().into_iter().flatten() {
                if let Some(name) = t["name"].as_str().filter(|n| !n.is_empty()) {
                    conn.tools.push(RemoteTool {
                        name: name.into(),
                        description: t["description"].as_str().unwrap_or_default().into(),
                        schema: t["inputSchema"].clone(),
                        read_only: t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
                    });
                }
            }
            cursor = page["nextCursor"].as_str().filter(|c| !c.is_empty()).map(String::from);
            if cursor.is_none() || conn.tools.len() > 500 {
                break;
            }
        }
        Ok(conn)
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        match &self.transport {
            Transport::Stdio { stdin, .. } => write_line(stdin, &msg).await,
            Transport::Http { .. } => self.http(&msg).await.map(|_| ()),
        }
    }

    async fn request(&self, method: &str, params: Value, timeout: Duration, cancel: &CancellationToken) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let answer = match &self.transport {
            Transport::Stdio { stdin, waiting, stderr, .. } => {
                let (tx, rx) = oneshot::channel();
                waiting.lock().expect("mcp waiting").insert(id, tx);
                write_line(stdin, &msg).await?;
                let cancel_it = || async {
                    waiting.lock().expect("mcp waiting").remove(&id);
                    let _ = write_line(stdin, &json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": id } })).await;
                };
                tokio::select! {
                    r = rx => match r {
                        Ok(v) => v,
                        Err(_) => {
                            let said = stderr.lock().expect("mcp stderr").trim().lines().last().unwrap_or_default().to_string();
                            return Err(if said.is_empty() { format!("{} ended", self.name) } else { format!("{} ended: {said}", self.name) });
                        }
                    },
                    _ = cancel.cancelled() => {
                        cancel_it().await;
                        return Err("stopped".into());
                    }
                    _ = tokio::time::sleep(timeout) => {
                        cancel_it().await;
                        return Err(format!("{} didn't answer in time", self.name));
                    }
                }
            }
            Transport::Http { .. } => tokio::select! {
                r = self.http(&msg) => r?.ok_or_else(|| format!("{} sent no answer", self.name))?,
                _ = cancel.cancelled() => return Err("stopped".into()),
                _ = tokio::time::sleep(timeout) => return Err(format!("{} didn't answer in time", self.name)),
            },
        };
        if let Some(e) = answer.get("error").filter(|e| !e.is_null()) {
            return Err(e["message"].as_str().unwrap_or("the server reported an error").to_string());
        }
        Ok(answer["result"].clone())
    }

    /// One message over HTTP; the answer is JSON or the matching message of an event stream.
    async fn http(&self, msg: &Value) -> Result<Option<Value>, String> {
        let Transport::Http { url, headers, session, http } = &self.transport else { return Ok(None) };
        let mut req = http
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", PROTOCOL_VERSION);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let had_session = session.lock().expect("mcp session").clone();
        if let Some(s) = &had_session {
            req = req.header("mcp-session-id", s);
        }
        let res = req.body(msg.to_string()).send().await.map_err(|e| format!("{} can't be reached: {e}", self.name))?;
        if let Some(s) = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *session.lock().expect("mcp session") = Some(s.to_string());
        }
        let status = res.status().as_u16();
        if status == 202 || status == 204 {
            return Ok(None);
        }
        if status == 404 && had_session.is_some() {
            // the server forgot the session: the next use starts a new one
            self.alive.store(false, Ordering::SeqCst);
            return Err(format!("{} ended the session; try again", self.name));
        }
        if status >= 400 {
            let body: String = res.text().await.unwrap_or_default().chars().take(300).collect();
            return Err(format!("{} answered HTTP {status}{}", self.name, if body.trim().is_empty() { String::new() } else { format!(": {}", body.trim()) }));
        }
        let stream = res.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("text/event-stream"));
        if !stream {
            let text = res.text().await.map_err(|e| e.to_string())?;
            return Ok(serde_json::from_str(&text).ok());
        }
        let mut body = res.bytes_stream();
        let mut buf = String::new();
        let mut data = String::new();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(at) = buf.find('\n') {
                let line = buf[..at].trim_end_matches('\r').to_string();
                buf.drain(..=at);
                if let Some(d) = line.strip_prefix("data:") {
                    data.push_str(d.trim_start());
                } else if line.is_empty() && !data.is_empty() {
                    if let Ok(v) = serde_json::from_str::<Value>(&data)
                        && v["id"] == msg["id"]
                        && v["method"].is_null()
                    {
                        return Ok(Some(v));
                    }
                    data.clear();
                }
            }
        }
        Ok(None)
    }

    /// Calls one of the server's tools.
    pub async fn call(&self, tool: &str, input: &Value, cancel: &CancellationToken) -> Output {
        let args = if input.is_object() { input.clone() } else { json!({}) };
        match self.request("tools/call", json!({ "name": tool, "arguments": args }), CALL_TIMEOUT, cancel).await {
            Ok(result) => {
                let mut text = String::new();
                let mut media = Vec::new();
                for item in result["content"].as_array().into_iter().flatten() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    match item["type"].as_str().unwrap_or_default() {
                        "text" => text.push_str(item["text"].as_str().unwrap_or_default()),
                        "image" => {
                            let media_type = item["mimeType"].as_str().unwrap_or_default();
                            match item["data"].as_str() {
                                Some(data) if crate::routes::chats::media_allowed(media_type) && data.len() <= 9_000_000 && media.len() < 4 => {
                                    media.push(Media { name: format!("image {}", media.len() + 1), media_type: media_type.into(), data: data.into() });
                                    text.push_str(&format!("[image {}]", media.len()));
                                }
                                _ => text.push_str("[an image that can't be passed on]"),
                            }
                        }
                        "resource" => text.push_str(item["resource"]["text"].as_str().unwrap_or("[a resource without text]")),
                        "resource_link" => {
                            text.push_str(&format!("[{}]({})", item["name"].as_str().unwrap_or("resource"), item["uri"].as_str().unwrap_or_default()))
                        }
                        other => text.push_str(&format!("[{other} content, which isn't passed on]")),
                    }
                }
                if text.trim().is_empty() && !result["structuredContent"].is_null() {
                    text = result["structuredContent"].to_string();
                }
                if text.trim().is_empty() {
                    text = "(no output)".into();
                }
                let text = text.replace('\0', "");
                let text = if text.chars().count() > MAX_RESULT_CHARS {
                    text.chars().take(MAX_RESULT_CHARS).collect::<String>() + "\n\n[The output was longer; the rest was cut off.]"
                } else {
                    text
                };
                Output { text, is_error: result["isError"].as_bool().unwrap_or(false), media }
            }
            Err(e) => Output { text: format!("{}: {e}", self.name), is_error: true, media: vec![] },
        }
    }
}

async fn spawn(def: &Def, dir: &std::path::Path, alive: Arc<AtomicBool>) -> Result<Transport, String> {
    let command = def.config["command"].as_str().unwrap_or_default().trim().to_string();
    if command.is_empty() {
        return Err("the server has no command".into());
    }
    let args: Vec<String> = def.config["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(String::from)).collect();
    tokio::fs::create_dir_all(dir).await.map_err(|e| format!("its folder {} can't be created: {e}", dir.display()))?;
    let mut cmd = tokio::process::Command::new(&command);
    cmd.args(&args).current_dir(dir).env_clear();
    for k in PASSED_ENV {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    cmd.envs(&def.secrets.env);
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("`{command}` isn't installed on the server (or not on its PATH)")
        } else {
            format!("`{command}` can't be started: {e}")
        }
    })?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let waiting: Waiting = Arc::new(Mutex::new(HashMap::new()));
    let stderr = Arc::new(Mutex::new(String::new()));
    {
        let waiting = waiting.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                // answers to our requests; the server's own requests and notifications are ignored
                if let Some(id) = msg["id"].as_u64().filter(|_| msg["method"].is_null())
                    && let Some(tx) = waiting.lock().expect("mcp waiting").remove(&id)
                {
                    let _ = tx.send(msg);
                }
            }
            // the program ended: whoever waits gets an error instead of a timeout
            alive.store(false, Ordering::SeqCst);
            waiting.lock().expect("mcp waiting").clear();
        });
    }
    if let Some(mut pipe) = child.stderr.take() {
        let stderr = stderr.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok(n) = pipe.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let mut s = stderr.lock().expect("mcp stderr");
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
                if s.len() > 4000 {
                    let cut = s.len() - 4000;
                    let cut = (cut..s.len()).find(|i| s.is_char_boundary(*i)).unwrap_or(0);
                    s.drain(..cut);
                }
            }
        });
    }
    Ok(Transport::Stdio { _child: child, stdin: tokio::sync::Mutex::new(stdin), waiting, stderr })
}

async fn write_line(stdin: &tokio::sync::Mutex<tokio::process::ChildStdin>, msg: &Value) -> Result<(), String> {
    let mut stdin = stdin.lock().await;
    let line = format!("{msg}\n");
    stdin.write_all(line.as_bytes()).await.map_err(|e| format!("the server doesn't take input any more: {e}"))?;
    stdin.flush().await.map_err(|e| format!("the server doesn't take input any more: {e}"))
}

// ---- the running servers --------------------------------------------------------------------

/// Servers started on this replica, by id.
#[derive(Default)]
pub struct Pool {
    conns: DashMap<Uuid, Arc<Conn>>,
    starting: DashMap<Uuid, Arc<tokio::sync::Mutex<()>>>,
}

impl Pool {
    /// The running server of `def`, started when needed (or when its settings changed).
    pub async fn get(&self, state: &AppState, def: &Def) -> Result<Arc<Conn>, String> {
        let fresh = |c: &Arc<Conn>| c.alive() && c.version == def.updated_at;
        if let Some(c) = self.conns.get(&def.id).map(|c| c.clone()).filter(fresh) {
            return Ok(c);
        }
        let lock = self.starting.entry(def.id).or_default().clone();
        let _held = lock.lock().await;
        if let Some(c) = self.conns.get(&def.id).map(|c| c.clone()).filter(fresh) {
            return Ok(c);
        }
        let conn = Arc::new(Conn::start(def, &state.config.mcp_dir.join(def.id.to_string()), state.config.mcp_stdio).await?);
        self.conns.insert(def.id, conn.clone());
        Ok(conn)
    }

    /// Ends a server (removed or changed by an administrator).
    pub fn drop_server(&self, id: Uuid) {
        self.conns.remove(&id);
    }
}

// ---- tools for an answer --------------------------------------------------------------------

/// One tool of an MCP server a model may call in an answer.
#[derive(Debug, Clone)]
pub struct McpTool {
    pub def: ToolDef,
    pub server: Uuid,
    pub server_name: String,
    /// the server's own name for the tool
    pub remote_name: String,
    /// ask | auto
    pub approval: String,
    pub read_only: bool,
}

/// The tools of the servers `user` may use, starting them when needed. A server that can't be
/// started is left out (and logged).
pub async fn tools_for(state: &AppState, user: Uuid) -> Vec<McpTool> {
    let defs = match granted(state, user).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("loading MCP servers failed: {e}");
            return vec![];
        }
    };
    let conns = futures::future::join_all(defs.iter().map(|d| state.mcp.get(state, d))).await;
    let mut out: Vec<McpTool> = Vec::new();
    for (d, conn) in defs.iter().zip(conns) {
        let conn = match conn {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(server = %d.name, "MCP server unavailable: {e}");
                continue;
            }
        };
        let about = d.description.as_deref().filter(|x| !x.trim().is_empty()).map(|x| format!(" ({})", x.trim())).unwrap_or_default();
        for t in conn.tools.iter().filter(|t| d.tools.is_empty() || d.tools.contains(&t.name)) {
            let name = crate::desktop::connector_tool_name(&d.name, &t.name);
            if out.iter().any(|x| x.def.name == name) {
                continue;
            }
            out.push(McpTool {
                def: ToolDef {
                    name,
                    description: format!("[{}{about}] {}", d.name, t.description).chars().take(1500).collect(),
                    schema: if t.schema.is_object() { t.schema.clone() } else { json!({ "type": "object", "properties": {} }) },
                },
                server: d.id,
                server_name: d.name.clone(),
                remote_name: t.name.clone(),
                approval: d.approval.clone(),
                read_only: t.read_only,
            });
        }
    }
    out
}

/// Runs a call, after checking it is still granted (an administrator may have withdrawn it
/// since the answer began).
pub async fn call(state: &AppState, user: Uuid, tool: &McpTool, input: &Value, cancel: &CancellationToken) -> (Output, &'static str) {
    let failed = |status, text: &str| (Output { text: text.into(), is_error: true, media: vec![] }, status);
    let def = match granted(state, user).await {
        Ok(defs) => defs.into_iter().find(|d| d.id == tool.server && (d.tools.is_empty() || d.tools.contains(&tool.remote_name))),
        Err(_) => None,
    };
    let Some(def) = def else {
        return failed("blocked", "Not allowed: the organization doesn't grant this tool to the user (any more).");
    };
    let conn = match state.mcp.get(state, &def).await {
        Ok(c) => c,
        Err(e) => return failed("error", &format!("{} is unavailable: {e}", def.name)),
    };
    let out = conn.call(&tool.remote_name, input, cancel).await;
    let status = if cancel.is_cancelled() {
        "error"
    } else if out.is_error && out.text.ends_with("didn't answer in time") {
        "timeout"
    } else if out.is_error {
        "error"
    } else {
        "ok"
    };
    (out, status)
}

/// A call waiting for the person to allow or decline it.
pub struct Pending {
    pub user_id: Uuid,
    pub tx: oneshot::Sender<bool>,
}

/// Asks the person (through the chat) whether a call may run; waits for the answer, the end of
/// the answer, or the time limit. `None`: no decision.
pub async fn ask(state: &AppState, user: Uuid, id: &str, cancel: &CancellationToken) -> Option<bool> {
    let (tx, rx) = oneshot::channel();
    state.approvals.insert(id.to_string(), Pending { user_id: user, tx });
    let decided = tokio::select! {
        r = rx => r.ok(),
        _ = cancel.cancelled() => None,
        _ = tokio::time::sleep(APPROVAL_TIMEOUT) => None,
    };
    state.approvals.remove(id);
    decided
}

/// Checks a server's settings and keeps only what its kind uses.
pub fn clean_config(kind: &str, c: &Value) -> Result<Value, String> {
    match kind {
        "mcp_stdio" => {
            let command = c["command"].as_str().unwrap_or_default().trim();
            if command.is_empty() {
                return Err("give the command that starts the server".into());
            }
            let args: Vec<&str> = c["args"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            Ok(json!({ "command": command, "args": args }))
        }
        "mcp_http" => {
            let u = c["url"].as_str().unwrap_or_default().trim();
            let parsed = url::Url::parse(u).map_err(|_| "give the server's URL".to_string())?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err("the URL starts with http:// or https://".into());
            }
            Ok(json!({ "url": u }))
        }
        _ => Err("kind is mcp_stdio or mcp_http".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(kind: &str, config: Value) -> Def {
        Def {
            id: Uuid::nil(),
            name: "Test".into(),
            kind: kind.into(),
            description: None,
            config,
            secrets: Secrets { env: HashMap::from([("GREETING".into(), "hi".into())]), headers: HashMap::new() },
            tools: vec![],
            approval: "auto".into(),
            updated_at: chrono::Utc::now(),
        }
    }

    /// A tiny MCP server in shell: answers initialize, tools/list and tools/call (echoing an
    /// environment variable, to show it is passed and the server's own environment isn't).
    const FAKE: &str = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake"}}}\n' "$id";;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"greet","description":"Greets","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}}\n' "$id";;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"%s %s"}]}}\n' "$id" "$GREETING" "${AGORA_SECRET_KEY:-none}";;
  esac
done"#;

    #[tokio::test]
    async fn stdio_round_trip() {
        let dir = std::env::temp_dir().join(format!("agora-mcp-test-{}", Uuid::new_v4()));
        let d = def("mcp_stdio", json!({ "command": "sh", "args": ["-c", FAKE] }));
        let conn = Conn::start(&d, &dir, true).await.expect("starts");
        assert_eq!(conn.tools.len(), 1);
        assert!(conn.tools[0].read_only);
        let out = conn.call("greet", &json!({}), &CancellationToken::new()).await;
        assert!(!out.is_error, "{}", out.text);
        assert_eq!(out.text, "hi none");
        assert!(Conn::start(&d, &dir, false).await.is_err(), "programs can be turned off");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn configs() {
        assert!(clean_config("mcp_http", &json!({ "url": "ftp://x" })).is_err());
        assert_eq!(clean_config("mcp_http", &json!({ "url": " https://x/mcp ", "headers": {} })).unwrap(), json!({ "url": "https://x/mcp" }));
        assert!(clean_config("mcp_stdio", &json!({ "args": [] })).is_err());
    }
}
