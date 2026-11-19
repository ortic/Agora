# Agora — Work log

## 2026-11-17

Started from an empty directory, on WikiCraft's stack. Dev Postgres: docker container `agora-pg` on
`127.0.0.1:55433` (agora/agora/agora); server on 8790, Vite on 5200. Node isn't installed on the
host: npm runs in the `node:22-bookworm-slim` image.

- Backend (crates/server): config, sessions and API tokens (from WikiCraft), SSO ported (OIDC,
  GitHub, LDAP, SAML behind a feature; workspaces removed: single tenant)
- Audit log hash-chained by a `BEFORE INSERT` trigger (advisory lock, sha256 over the row and the
  previous hash); `audit_guard` trigger refuses UPDATE/DELETE/TRUNCATE except inside the retention
  purge (`set_config('agora.audit_purge', 'on', true)`). Verified: tampering attempts fail.
- Provider adapters: Anthropic Messages API over raw HTTP/SSE (`anthropic-version: 2026-10-21`: no
  sampling parameters; adaptive thinking with summaries; `fallbacks: "default"` for the 5.x
  models; automatic prompt caching; `web_search_20260209`), OpenAI-compatible chat completions
  (tool calls, `reasoning_content`, `<think>` splitting, usage with cached tokens)
- Claude replays: an answer's raw content blocks (thinking signatures, server tool results) are
  stored and sent back unchanged on later turns; other models get the text
- Chat: generation in a background task with a hub (snapshot + broadcast with sequence numbers, so a
  reattaching tab sees every event once), branching via `parent_id` + `chats.leaf_id`, titles,
  stop, restart recovery
- Tools: web_search (SearXNG/Brave/Tavily), fetch_url (SSRF-safe), WikiCraft retrieval per user
- Usage per model call with cost from model prices; budgets (user > max(group) > default); rate limit
- OpenAI-compatible API for personal tokens
- Web app: chat, settings, admin (overview, users, groups, models, usage, audit, web activity,
  conversation inspector, settings incl. knowledge sources)
- Verified end to end against a mock server speaking both APIs (scratchpad, not committed):
  streaming, cost arithmetic, thinking replay, blocked private fetch, tool loop, WikiCraft
  retrieval contract, OpenAI API (stream and not), audit chain; UI walkthrough in Chromium
  (Playwright in docker) without console errors, light/dark/mobile
- RAG research: WikiCraft has `POST /api/retrieve` (hybrid, permission-filtered per token) and an
  MCP server, no impersonation → per-user read-only tokens. TaskCraft: not on this machine and
  not reachable on GitHub from here; waiting for its location.
- Local model: the machine has an NVIDIA RTX Spark N1X (~96 GB GPU memory, CUDA 13.4 in WSL; WSL
  itself has 24 GB RAM). Ollama for linux-arm64 is unpacked in `~/.local/ollama` (CUDA 12/13
  backends); starting it is left to the user.

- Local model on the GPU, verified (2026-11-18): Ollama 0.40 linux-arm64 in WSL sees the RTX Spark
  N1X through the CUDA 13 backend (cc 12.1, `type=iGPU`, 110 GiB total but only ~21.7 GiB
  available: shared with WSL's 24 GB limit). qwen3:4b: 37/37 layers on the GPU, ~45 tok/s output,
  ~5,000 tok/s prompt. Through Agora it called the wiki tool on its own and answered with a
  citation. But Ollama's default context (262,144 tokens, sized from the GPU memory) made the
  chat take ~3 min and drove WSL out of memory → set `OLLAMA_CONTEXT_LENGTH` (e.g. 32768) and
  raise `memory=` in .wslconfig for bigger models.

## 2026-11-19

Agora Desktop: a desktop client (new repository `../AgoraDesktop`, egui on DbCraft's foundation)
with Agora as its backend.

- Server (`desktop.rs`, `routes/desktop.rs`, migration 0003): features `desktop` / `code` /
  `browser` granted to nobody, everyone or groups, each with settings; local connectors (MCP);
  device sign-in confirmed in the web app; device tokens; a WebSocket per connected app over
  which the chat's tool loop runs tools on the person's computer (progress, cancel, timeout);
  `tool_events` and audit events; policy pushed to connected apps on change
- Chat: a message sent from the app may name the tools of that computer it wants; the server
  intersects that with the person's grants. Tool budget per answer is the desktop's own
  (default 60) when its tools are in use. Web sessions never get a device's tools
- Web app: *Administration → Desktop* (features, connectors, devices, tool activity),
  `/desktop/link` to confirm a device (the code survives signing in), devices under Settings,
  tool calls shown in chats and the conversation inspector
- Verified end to end with a scripted mock model and a throwaway Postgres: sign-in refused
  before the grant; code tools in a `debian:bookworm-slim` dev container (list, read, edit,
  write, search, run, timeout); approval `ask` declined and approved; revoking `code` removes the
  tools; a web session asking for device tools gets none; an MCP stdio connector with a tool
  allow-list (and one that fails to start); browser tools against headless Chromium with an
  allowed-sites policy (leaving the allowed site is caught, password fields refused); audit
  chain verifies. Web pages checked in Chromium (Playwright in docker), no console errors
- Not verified: Windows and macOS (CI only), several replicas (a device connects to one)

Desktop, second round (same day): what long coding work needs, and a review of the first round.

- Tool loop (`chat.rs`): `task` tool runs a sub-agent (same model and tools without `task`, empty
  context, budget `maxSubagentToolCalls`, default 30; its calls show under it in the chat and are
  recorded like any other); consecutive read-only calls of a turn run concurrently; a conversation
  is summarized at 70% of the context window (Claude models without one on record: 200k), before
  an answer or between tool rounds, stored as `raw.compact` and used by `history()` afterwards
- System prompt gets the project's instructions and notes the app sends (clipped)
- From the review: device calls carry an id of our own (model ids repeat between answers and
  sub-agents), the person's policy is checked again before every call, NUL characters are removed
  from tool input and output (Postgres refused them, which lost the answer or the record), the
  progress buffer is bounded
- Verified with the mock model: three reads in one turn, two sub-agents at once writing files,
  summary during an answer and before one, the follow-up turn starting from the summary
- Review findings not fixed yet are listed in the README of AgoraDesktop ("Known weaknesses")
  and below

### Next
- Desktop review, server side: a device token is a full account credential (can approve another
  device, create API tokens, use admin routes): restrict what `agd_` tokens may call; group and
  user changes in `routes/admin.rs` don't push policies; `tool_events.input` ignores the
  `audit_content` setting; `login_start` rate limit is per IP behind a proxy
- Integration tests in `crates/server/tests` (mock provider, fresh database per test)
- TaskCraft connector once its API is known; MCP servers as tools on the server side
- Desktop: packages and installers, replay of tool results for non-Claude models, calls routed between replicas
- File attachments (images, PDFs), knowledge bases of Agora's own (pgvector)
- TOTP/passkeys (port from WikiCraft), SIEM export of audit events, budget alerts
