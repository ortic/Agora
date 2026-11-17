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

### Next
- Integration tests in `crates/server/tests` (mock provider, fresh database per test)
- TaskCraft connector once its API is known; MCP servers as tools
- File attachments (images, PDFs), knowledge bases of Agora's own (pgvector)
- TOTP/passkeys (port from WikiCraft), SIEM export of audit events, budget alerts
