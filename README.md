# Agora

An internal AI platform for your organization: everyone chats with the models the organization
allows (Claude, any OpenAI-compatible service, models running on your own hardware), answers can
draw on your internal wiki, and the organization stays in control: who used what, what it cost,
what was searched on the web, and a tamper-evident audit trail of all of it.

Built on the same stack as [WikiCraft](https://github.com/ortic/WikiCraft): a **Rust** backend
(axum, sqlx, PostgreSQL) and a **React + TypeScript** web app, shipped as a single binary.
Inspired by [LibreChat](https://github.com/danny-avila/LibreChat) and
[Open WebUI](https://github.com/open-webui/open-webui), with the focus on what companies need
when they roll such a tool out.

![A chat in Agora: a local model answers from the team wiki, citing the runbook it found](docs/screenshots/chat.png)

<sub>Screenshot with demo data. Dark theme: [docs/screenshots/chat-dark.png](docs/screenshots/chat-dark.png)</sub>

## Features

**Chat**
- Streaming answers with Markdown, tables and highlighted code (with copy buttons)
- Model picker per chat; Claude's reasoning and local models' `<think>` shown as a collapsible thought process
- Edit a message or regenerate an answer: each becomes a branch you can flip through (‹ 1 / 2 ›)
- Answers keep being written when you close the tab; reopen the chat and it picks up the stream; *Stop* at any time
- Chat list with search, pinning, renaming and automatic titles; per-chat instructions; light and dark theme; works on phones
- **Images and PDFs** attached to a message (button, drop or paste), for models that read them
- Tokens, cost and time under every answer

**Models**
- **Claude** through the Anthropic Messages API: adaptive thinking, effort levels, prompt caching, server-side fallback on declined requests, Claude's own web search
- **Claude Code** (`claude -p` on the server) as a model, with Agora's tools only
- **Any OpenAI-compatible API**: OpenAI, Azure-style gateways, LiteLLM, OpenRouter, and models in your own network: **Ollama, vLLM, llama.cpp, LM Studio, LocalAI**
- Model discovery (`/models`), list prices for Claude filled in automatically, your own prices for the rest
- Access per model: everyone, or selected groups; per-model instructions, default effort, temperature

**Knowledge (RAG)**
- **WikiCraft** as a knowledge source: the model searches the wiki while answering (hybrid keyword + semantic retrieval) and cites the pages
- Everyone connects their **own read-only token**, so the wiki's permissions apply to every search: Agora never sees a page the person couldn't open

**Web search, tracked**
- SearXNG, Brave Search API, Serper (Google results) or Tavily, plus page reads, as tools the model decides to use; or Claude's own web search
- **Every search and page read is recorded**: who, which chat, query or URL, engine, result URLs, status
- Private network addresses are never fetched (DNS checked and pinned, redirects re-checked); block list for domains

**Usage, cost and budgets**
- One record per model call: tokens (input, output, cache read and write), provider-side searches, cost, latency, status
- Dashboards per day, person, model and group; CSV export; everyone sees their own usage
- **Monthly budgets** per person, per group or a default, enforced before each request; rate limit per person

**Audit**
- Append-only audit log: sign-ins (and failures), every prompt and answer (full text or metadata only, your choice), searches, page reads, knowledge searches, API calls, every administrative change
- **Hash-chained in the database**: every event's hash covers its content and the previous hash; triggers refuse `UPDATE`, `DELETE` and `TRUNCATE`. Verify the chain in the UI or with `agora verify-audit`
- **Auditor** role: reads the audit log, usage and conversations, but changes nothing. Opening someone's conversation is itself audited
- Deleted chats disappear for their owner but stay available to auditors for a retention period; audit retention configurable
- Filters and CSV / JSONL export

**Accounts and sign-in**
- Local accounts, **single sign-on** with OpenID Connect (Microsoft Entra ID, Google, GitLab, Okta, Keycloak, Authentik, …), GitHub, **LDAP / Active Directory** and **SAML 2.0**, several at once, with **group sync** from the identity provider
- Roles: owner, admin, auditor, member; personal API tokens

**On people's computers (Agora Desktop)**
- [Agora Desktop](https://github.com/ortic/AgoraDesktop) is a client of this server: models work on **code in dev containers**, drive **a browser**, and use **local connectors** (MCP servers) on the person's computer
- **Nothing is available until an administrator grants it**: the app itself, code, the browser and each connector, to everyone or to groups; allowed container images, allowed and blocked sites, and whether the person confirms each change
- **Long work**: a model can hand parts of the work to **sub-agents** with a context of their own (admins can turn that off), calls that only read run at the same time, and a conversation that outgrows the model's context window is **summarized** and goes on; a project's `AGENTS.md` or `CLAUDE.md` and its notes in `.agora/memory.md` are given to the model
- **Slash commands** for the app are defined by administrators (`/name` stands for a prompt); commands from people's own files can be turned off
- Devices sign in through the browser (so single sign-on applies), are listed per person and can be signed out by them or by an administrator
- **Every tool call is recorded**: who, which computer, which tool with which input, how it ended; in the audit log too

**API**
- **OpenAI-compatible API** (`/api/v1/models`, `/api/v1/chat/completions`, streaming) with personal tokens: point your editor, scripts or tools at Agora and the same access rules, budgets, usage records and audit apply

## Quick start (Docker)

```bash
export AGORA_DB_PASSWORD=$(openssl rand -hex 16) AGORA_SECRET_KEY=$(openssl rand -hex 32)
docker compose up -d            # builds the image the first time
open http://localhost:8790      # the first visit creates the owner account
```

Then: **Administration → Models → Connect a provider**, add models, and start chatting.
Keep `AGORA_SECRET_KEY` safe and unchanged: it encrypts provider keys and connector tokens.

## Claude Code as a model

A provider of the kind **Claude Code** makes the server answer with `claude -p`: Claude Code's
own agent loop (its context handling, planning, sub-agents) writes the answer, and Agora relays
it. Claude Code must be installed on the server (`AGORA_CLAUDE_BIN`, default `claude`).

- It gets **none of its built-in tools** for files, commands or the web, so nothing runs on
  the server. Its tools are Agora's, offered over MCP for the length of an answer: web search
  and page fetches, your knowledge sources, and the tools on the person's computer through
  Agora Desktop. Grants, confirmations and the record of every call are the same as with any
  other model. Its sub-agents are kept when the Desktop settings allow sub-agents.
- The server account's own Claude Code setup (settings, hooks, commands, MCP servers) isn't used.
- **Who pays:** with an API key on the provider, that Anthropic account; without one, the
  account `claude` is signed in with on the server. Check Anthropic's terms before serving
  several people from one personal subscription.
- A chat continues the Claude Code session of its previous answer; sessions are kept by Claude
  Code in the server account's home (per chat, under `AGORA_CLAUDE_DIR`). A session that is gone
  is replaced by a new one that is given the conversation as text.
- Add the models by name as `claude --model` takes them (`sonnet`, `opus`, `haiku`, or a full
  model name). Set their prices if you want Agora's own cost figures; without prices the cost
  Claude Code reports is shown.

## Running a model on your own hardware

Any server with an OpenAI-compatible API works. With [Ollama](https://ollama.com):

```bash
ollama serve                        # listens on 127.0.0.1:11434
ollama pull gpt-oss:20b             # or qwen3, llama3.3, mistral-small, …
```

Set a sensible context length: Ollama otherwise sizes it from the GPU memory (up to 256k tokens),
and that KV cache alone can exhaust memory: `OLLAMA_CONTEXT_LENGTH=32768 ollama serve`.

**Windows on ARM / WSL (e.g. NVIDIA RTX Spark):** Ollama's Linux ARM64 build uses the GPU through
WSL with its CUDA 13 backend (compute capability 12.1; the CUDA 12 backend skips it). Start it with
`LD_LIBRARY_PATH=/usr/lib/wsl/lib`. The GPU shares memory with WSL, so WSL's memory limit
(`memory=` in `%UserProfile%\.wslconfig`, default half the RAM) caps the model size: raise it
(e.g. `memory=96GB`, then `wsl --shutdown`) for models beyond ~20 GB such as `gpt-oss:120b`.

In Agora: *Connect a provider → Ollama on this machine* (base URL `http://127.0.0.1:11434/v1`),
then *Add models*. Mark it in its description as the model for confidential data: nothing leaves
your network. vLLM (`vllm serve <model>`, base URL `http://host:8000/v1`) is the better choice for
many concurrent users.

## Answers from your wiki (WikiCraft)

1. *Administration → Settings → Knowledge sources → Add a WikiCraft wiki*: its address and a line
   on what's in it.
2. Everyone: *Settings & usage → Knowledge sources → Connect*, with a **read-only** API token
   created in WikiCraft (*Settings → API tokens*).
3. The composer gets a toggle with the wiki's name; with it on, the model searches the wiki
   (WikiCraft's `/api/retrieve`) and cites the pages it used.

For semantic (not only keyword) retrieval, set up embeddings in WikiCraft
(`WIKICRAFT_EMBEDDINGS_URL`, e.g. a local Ollama embedding model).

## Configuration

Everything that changes at runtime (models, groups, budgets, web search, retention, knowledge
sources) is set in the admin UI. The server itself takes flags or environment variables:

| Variable | Default | |
|---|---|---|
| `AGORA_DATABASE_URL` | `postgres://agora:agora@127.0.0.1:55433/agora` | PostgreSQL |
| `AGORA_BIND` | `127.0.0.1:8790` | listen address |
| `AGORA_BASE_URL` | `http://127.0.0.1:8790` | public URL (SSO redirects) |
| `AGORA_SECRET_KEY` | — | encrypts provider keys and tokens (**set it**) |
| `AGORA_SECURE_COOKIES` | `false` | `true` behind HTTPS |
| `AGORA_TRUST_PROXY` | `false` | take client addresses from `X-Forwarded-For` |
| `AGORA_SESSION_DAYS` / `AGORA_SESSION_IDLE_MINUTES` | `14` / `0` | session lifetime, idle timeout |
| `AGORA_CLAUDE_BIN` / `AGORA_CLAUDE_DIR` | `claude` / `./data/claude-code` | Claude Code, for models that answer through it, and where it runs |
| `AGORA_AUTH_PROVIDERS` / `_FILE` | — | single sign-on providers (JSON, below) |
| `AGORA_DISABLE_PASSWORD_LOGIN` | `false` | SSO only (LDAP still works) |
| `AGORA_FETCH_ALLOW_PRIVATE` | `false` | let models open private network addresses |
| `AGORA_MODEL_TIMEOUT_SECS` | `300` | wait for a model's first byte / between chunks |
| `AGORA_LOG_FORMAT` | `text` | `json` for log collectors |

### Single sign-on

```json
[
  { "type": "oidc", "id": "entra", "preset": "microsoft", "tenant": "<tenant id>",
    "clientId": "…", "clientSecret": "…", "syncGroups": true, "groupsClaim": "groups" },
  { "type": "oidc", "id": "keycloak", "name": "Company login", "issuer": "https://sso.example.com/realms/acme",
    "clientId": "agora", "clientSecret": "…", "allowedDomains": ["example.com"] },
  { "type": "ldap", "id": "ad", "url": "ldaps://dc.example.com:636", "baseDn": "dc=example,dc=com",
    "bindDn": "cn=agora,ou=services,dc=example,dc=com", "bindPassword": "…", "syncGroups": true },
  { "type": "github", "id": "github", "clientId": "…", "clientSecret": "…", "allowedOrgs": ["acme"] }
]
```

Redirect URI to register: `{AGORA_BASE_URL}/api/auth/sso/{id}/callback`. SAML needs the image
built with `--build-arg FEATURES=saml` (libxmlsec1); its ACS is `/api/auth/sso/{id}/acs`.
New accounts are created at first sign-in (unless `autoCreate: false`); groups from the provider
are mirrored and can carry budgets and model access.

## Using the API

Create a token under *Settings & usage → API tokens*, then use any OpenAI client:

```bash
curl http://localhost:8790/api/v1/chat/completions \
  -H "Authorization: Bearer agt_…" -H "Content-Type: application/json" \
  -d '{"model": "claude-opus-5-5", "messages": [{"role": "user", "content": "Hello"}], "stream": true}'
```

## Agora Desktop

The desktop app has no models and no rules of its own: it signs in to this server, and the
model's tool loop keeps running here. A tool that needs the person's computer is sent to their
app over a WebSocket (`/api/desktop/ws`) and answered from there.

Under *Administration → Desktop*:

| Setting | What it decides |
|---|---|
| **Agora Desktop** | who may sign in with the app at all (nobody by default); tool calls per answer, seconds a tool may take |
| **Code in dev containers** | who gets file and command tools; the container images projects may use, the default image, network access of containers, whether the person confirms commands and file changes |
| **Browser** | who gets the browser tools; allowed and blocked sites; whether the person confirms clicks and typing |
| **Local connectors** | MCP servers the app may start (`command`, arguments, environment) or reach (URL) on the computer; which of their tools models get; for whom |
| **Devices** | signed-in computers, with a sign-out |
| **Tool activity** | every call a model made on someone's computer |

What is enforced where: the server offers a model only the tools its user was granted, only for
messages the app itself sent, and refuses blocked sites before a call leaves. The app checks each
call against the same policy again and is what asks the person and keeps to the allowed images
and sites on the computer. A connector's settings are sent to the apps of the people it is
granted to: don't put secrets in them.

## Development

```bash
docker run -d --name agora-pg -e POSTGRES_USER=agora -e POSTGRES_PASSWORD=agora -e POSTGRES_DB=agora \
  -p 127.0.0.1:55433:5432 pgvector/pgvector:pg17
cargo run -p agora -- serve                 # API on :8790 (migrations run at start)
cd web && npm install && npm run dev        # web app on :5200, proxies /api
cargo test -p agora                          # unit tests
```

Commands: `agora serve | migrate | verify-audit | healthcheck`.

Layout: `crates/server` (the binary: `llm/` provider adapters, `chat.rs` answering and tool loop,
`connectors.rs` knowledge sources, `desktop.rs` the desktop app's policy and tool bridge, `web.rs` search and page reads, `audit.rs`, `usage.rs`,
`sso/`, `routes/`), `web/` (React app). See [PLAN.md](PLAN.md) for the roadmap and
[WORKLOG.md](WORKLOG.md) for what's been done.
