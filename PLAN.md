# Agora — Product & Implementation Plan

> A self-hosted, internal AI platform: everyone in the organisation talks to the models the
> organisation allows (Claude, OpenAI-compatible services, models in your own network), and the
> organisation keeps control: who used what, what it cost, what was searched on the web, and a
> tamper-evident audit trail.
>
> Stack: the same as WikiCraft — a **Rust** backend (axum 0.8, sqlx/PostgreSQL, tokio), a
> **React 19 + Vite + TypeScript** frontend (React Query, React Router, lucide icons, plain CSS
> with design tokens), shipped as a single binary with the web app embedded.
>
> Status: in development (see WORKLOG.md) · Started 2026-11-17

## 0. How to read this plan

`[ ]` not started · `[~]` in progress · `[x]` done. Priorities: **P0** MVP (a team can use it
daily) · **P1** v1.0 (credible replacement for LibreChat / Open WebUI in a company) · **P2**
enterprise / compliance · **P3** later.

Who has it: `L` LibreChat · `W` Open WebUI. No tag = table stakes or a new idea.

## 1. Positioning

Open WebUI and LibreChat are good chat front-ends, but companies that roll them out keep asking
for the same things they are weak at:

| Need | LibreChat | Open WebUI | Agora's answer |
|---|---|---|---|
| Audit trail | basic logs | audit log (EE-ish, text logs) | **append-only, hash-chained audit table** in Postgres, DB trigger blocks edits, verify endpoint, export |
| Who saw what on the web | not tracked | not tracked | **every web search and page fetch recorded** (user, query, engine, result URLs, chat) |
| Usage & cost per user | token "balance" credits | usage stats (basic) | per request: tokens, cache tokens, searches, cost, latency; per user/group/model/day dashboards; **budgets** |
| Deleted chats | gone | gone | soft-deleted, hidden from the user, kept for the retention period for compliance |
| Operations | Node + Mongo + Meili + RAG API | Python + many services | **one Rust binary + Postgres** |
| SSO | OIDC, SAML, LDAP | OIDC, LDAP, trusted header | OIDC (Entra ID, Google, Okta, Keycloak…), GitHub, LDAP/AD, SAML 2.0, group sync |

## 2. Inspiration (what we take from them)

- **LibreChat**: `librechat.yaml` endpoints with per-endpoint model lists and pricing; balance /
  token accounting; conversation branching (edit → new branch, sibling navigation); presets;
  agents with tools; web search with a reranker; MCP; artifacts; shared links; prompts library.
- **Open WebUI**: admin-managed connections with model discovery (`/models`); "Workspace"
  models (a base model + system prompt + knowledge + tools); per-group model access; RBAC;
  web search with many engines (SearXNG, Brave, Tavily, …); RAG/knowledge; arena; channels;
  OpenAI-compatible API for users.

## 3. Architecture

```
browser ── /api (JSON, SSE) ──► agora (axum)
                                 ├─ auth: sessions, API tokens, OIDC/GitHub/LDAP/SAML (from WikiCraft)
                                 ├─ llm: provider adapters ─► Anthropic Messages API (raw HTTP, SSE)
                                 │                         └► OpenAI-compatible /chat/completions (OpenAI, Azure-style
                                 │                            gateways, Ollama, vLLM, LM Studio, LiteLLM, OpenRouter…)
                                 ├─ chat: orchestration (context, tool loop, persistence, cancel)
                                 ├─ tools: web_search (SearXNG / Brave / Tavily), fetch_url (SSRF-safe)
                                 ├─ usage: metering, pricing, budgets, rate limits
                                 ├─ audit: hash-chained, append-only (DB trigger), retention
                                 └─ /api/v1/chat/completions: OpenAI-compatible API for users' tools
                                PostgreSQL (everything; no Redis, no queue)
```

- Generation runs in a **background task**, not tied to the HTTP request: closing the tab doesn't
  lose the answer; the browser streams it over SSE; *Stop* cancels the task.
- Provider API keys are **encrypted at rest** (AES-256-GCM, key from `AGORA_SECRET_KEY`) and never
  sent back to the browser (only the last 4 characters).
- Single tenant (one organisation per installation) — simpler than WikiCraft's workspaces.

## 4. Feature plan

### 4.1 Accounts, SSO, roles
- [x] P0 First-run setup wizard (creates the owner)
- [x] P0 Local accounts (argon2), sessions (HttpOnly cookie, CSRF header), login throttling
- [x] P0 OIDC providers (Entra ID, Google, GitLab presets; any issuer), GitHub, LDAP/AD (ported from WikiCraft)
- [~] P0 SAML 2.0 behind cargo feature `saml` (ported; needs libxmlsec1 at build time; not yet compiled with the feature)
- [x] P0 Just-in-time provisioning, allowed domains, group sync from the IdP
- [x] P0 Roles: `owner`, `admin`, `auditor` (read-only access to audit, usage and conversations), `member`
- [x] P0 Personal API tokens (for the OpenAI-compatible API)
- [ ] P1 Two-factor (TOTP) and passkeys for local accounts (port from WikiCraft)
- [ ] P1 SCIM 2.0 provisioning
- [ ] P2 Trusted-header auth (behind an authenticating proxy) `W`

### 4.2 Models & providers (admin)
- [x] P0 Providers: Anthropic, OpenAI-compatible (base URL + optional key + extra headers)
- [x] P0 Model discovery (`GET /models`) and manual entry `L` `W`
- [x] P0 Per model: display name, description, enabled, prices (input / output / cache read / cache write per MTok), context window, max output, capabilities (vision, tools, native web search), default effort / system prompt
- [x] P0 Access: everyone or selected groups `W`
- [x] P0 Connection test
- [ ] P1 Custom "assistants" = base model + system prompt + tools + knowledge (`W` workspace models, `L` agents)
- [ ] P1 Fallback / routing rules (model A unavailable → model B)
- [ ] P2 Bedrock / Vertex / Azure OpenAI native auth

### 4.3 Chat
- [x] P0 Streaming chat (SSE), markdown, code highlighting + copy, stop, regenerate
- [x] P0 Model picker per chat, system prompt per chat
- [x] P0 Branching: edit a message or regenerate → sibling branch; ‹ 1/2 › navigation `L`
- [x] P0 Chat list: search, rename, pin, delete (soft), auto title
- [x] P0 Reasoning display (Claude summarized thinking, `<think>` of local models)
- [x] P0 Web search toggle with sources shown under the answer
- [x] P0 Per message: tokens, cost, model, duration
- [ ] P1 File attachments: images (vision), text/code files, PDF
- [x] P1 Knowledge from WikiCraft: per-user read-only tokens, `/api/retrieve` as a model tool, cited sources, every search logged
- [ ] P1 TaskCraft as a knowledge source (location / API to be confirmed)
- [ ] P1 Agora's own knowledge bases (uploads) with pgvector `W` `L`
- [ ] P1 Prompt library with variables `L` `W`
- [ ] P1 Share a chat read-only with colleagues (audited) `L` `W`
- [ ] P1 Export chat (Markdown / JSON)
- [ ] P2 MCP servers as tools (admin-approved) `L` `W`
- [ ] P2 Artifacts / HTML preview `L`
- [ ] P3 Arena / side-by-side comparison `W`

### 4.4 Web search & tracking
- [x] P0 Engines: SearXNG, Brave Search API, Serper, Tavily; `fetch_url` for pages (SSRF-safe, size limits)
- [x] P0 Tool calling on both provider types (the model decides when to search)
- [x] P0 Claude's native server-side web search as an option (results also tracked)
- [x] P0 **Every search and fetch recorded**: user, chat, message, engine, query/URL, result URLs, status, duration
- [x] P0 Admin view: search log with filters; domain block list
- [ ] P1 Allow-list mode (only these domains), per-group enablement
- [ ] P2 Reranking / result caching

### 4.5 Usage, cost, budgets
- [x] P0 Usage event per model call: user, model, provider, tokens (in / out / cache read / cache write), searches, cost, latency, status, source (chat / title / API)
- [x] P0 Dashboards: totals, per day, per user, per model, per group
- [x] P0 Monthly budgets (USD): default, per group, per user override; enforced before each request
- [x] P0 Rate limit per user (requests per minute)
- [x] P0 Users see their own usage
- [x] P0 CSV export
- [ ] P1 Budget alerts (e-mail at 80 % / 100 %)
- [ ] P2 Chargeback reports per cost centre (group)

### 4.6 Audit
- [x] P0 Append-only `audit_events`: actor, IP, user agent, action, target, details
- [x] P0 **Hash chain** (each row includes the previous row's hash), computed in the database; a trigger forbids UPDATE/DELETE except through the retention purge
- [x] P0 Verify endpoint (recomputes the chain), shown in the admin UI
- [x] P0 Audited: sign-in/out (+ failures), admin changes (users, groups, providers, models, settings), chat created/deleted, every prompt (content per setting: `full` or `metadata`), searches, fetches, API use, auditors opening someone's conversation
- [x] P0 Auditor conversation viewer (read-only; access itself audited)
- [x] P0 Filters, JSONL/CSV export
- [x] P0 Retention: purge soft-deleted chats after N days; audit retention (default: keep forever)
- [ ] P1 Stream audit events to a SIEM (syslog / webhook / OTLP)
- [ ] P1 Legal hold per user (no purge)
- [ ] P2 External anchoring of the chain head (e.g. daily hash to a WORM bucket)
- [ ] P2 DLP: block/redact prompts matching patterns (credit cards, secrets) before they leave

### 4.7 Integration
- [x] P0 OpenAI-compatible API (`/api/v1/models`, `/api/v1/chat/completions`, streaming) with personal tokens: same access rules, budgets, usage and audit
- [ ] P1 Anthropic-compatible `/api/v1/messages` passthrough for Claude-native tools
- [ ] P2 Webhooks

### 4.8 Agora Desktop (client in ../AgoraDesktop)
- [x] P1 Device sign-in confirmed in the web app (code shown by the app; works with SSO); device tokens (`agd_…`); devices listed per person, sign-out by the person or an admin
- [x] P1 Features granted per person: `desktop`, `code`, `browser`: nobody / everyone / groups, each with its settings (allowed images, default image, container network, approval mode; allowed and blocked sites)
- [x] P1 Local connectors defined by admins (MCP over stdio or HTTP), tool allow-list, approval mode, access per group
- [x] P1 Tool bridge: the tool loop stays on the server; calls go to the person's app over a WebSocket, with progress, cancel and timeout; tools only for messages sent from the app
- [x] P1 `tool_events` log + audit (`desktop.tool`, `device.*`, `desktop.*_updated`); admin view; tool calls shown in chats (web and desktop)
- [x] P1 Policy pushed to connected apps when an admin changes it; revoking the app disconnects them
- [ ] P1 Tool results of earlier turns replayed to non-Claude models (today: only the answer's text)
- [ ] P1 Several replicas: a device is connected to one replica; calls must be routed to it
- [ ] P2 Per-connector secrets held server-side and handed to the app encrypted per device
- [ ] P2 Images (screenshots) as tool results
- [ ] P2 People adding their own local connectors, if the admin allows it

### 4.9 Operations
- [x] P0 Single binary, embedded web app, `agora serve|migrate|verify-audit|healthcheck`
- [ ] P0 Integration tests (mock provider, fresh database per test) in CI
- [x] P0 Dockerfile, docker-compose (Postgres)
- [x] P0 Health/readiness endpoints, JSON logs, request ids
- [ ] P1 Several replicas: stop/reattach across replicas (today: the replica that generates)
- [ ] P1 Kubernetes manifests, Prometheus metrics
- [ ] P1 Backup/restore command

## 5. Data model (P0)

`settings` (singleton JSON) · `users` · `sessions` · `api_tokens` · `identities` · `auth_states` ·
`login_attempts` · `groups` · `group_members` · `providers` · `models` · `model_groups` ·
`chats` · `messages` (tree: `parent_id`; `chats.leaf_id` = current branch) · `usage_events` ·
`web_events` (searches + fetches) · `audit_events` (hash chain) · desktop: `desktop_features` ·
`desktop_feature_groups` · `local_connectors` · `local_connector_groups` · `devices` ·
`device_logins` · `tool_events`.

## 6. Security notes

- CSRF: mutating cookie requests need the `x-agora` header; no CORS.
- Strict CSP; markdown rendered without raw HTML.
- Provider keys encrypted; secrets never logged.
- `fetch_url` refuses private, loopback and link-local addresses (DNS resolved and checked, redirects re-checked) unless `AGORA_FETCH_ALLOW_PRIVATE`.
- Model access is enforced server-side on every request (UI filtering is cosmetic).
