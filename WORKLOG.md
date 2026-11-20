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

Desktop, third round (same day):

- Attachments: `attachments` table (migration 0004), `attachments` on a sent message (images up
  to 5 MB, PDFs, 20 MB per message, only for models with the vision capability), named in the
  user message's `parts`, `GET /api/attachments/{id}` for the owner; `Block::Document`; a tool
  result may carry files (`media`), which Claude gets inside the tool result and other models
  in a user message after it
- Dev container tools: `run_command` with `background`, `command_output`, `stop_command`
- Slash commands: `commands` and `fileCommands` in the `desktop` feature's settings (admin page
  has an editor); a message with `command: "compact"` summarizes instead of answering
- Web app: Attach button, drop and paste in the composer; files shown on messages
- Verified with the mock model: a background command read while running, stopped, exit code
  kept; a PNG and a PDF read by the model and attached by the person arrive as `image_url` and
  `file` parts; organization and project commands expand; files-off hides project commands;
  `/compact` and the turn after it; the web pages in Chromium without console errors
- Not verified: PDFs and tool-returned files against a real Claude or OpenAI model (the mock
  only shows what is sent)

- Commands give the model their output while they run: `run_command` returns after
  `checkAfterSecs` (default 30) or when the output matches `until`, the command goes on under
  its time limit, `command_output` reads on (also with `until`). Verified live: partial output
  after 2 s, a pattern reached at the right moment, the limit ending a command, stop ending a
  command with child processes

Claude Code as a backend (same day): provider kind `claude_code` (migration 0005),
`chat/claude_code.rs`.

- An answer runs `claude -p --output-format stream-json --input-format stream-json --tools ""
  (or `Task`) --strict-mcp-config --mcp-config … --allowedTools mcp__agora --permission-prompts
  none --setting-sources "" --disable-slash-commands --append-system-prompt …`, in an empty
  folder per chat; flags checked against Claude Code 2.1.328
- Its tools are ours: an MCP endpoint per running answer (`POST /api/mcp/{token}`, JSON-RPC
  over HTTP) calls the same `run_tool` as the tool loop, so device tools, approvals,
  `tool_events` and audit are unchanged; the per-answer tool limit applies
- Sessions: `raw.claudeCode.sessionId` on the answer, `--resume … --fork-session` for the next
  one; fresh session with the conversation as text when there is none (or it is gone)
- `collect_files` (device tool, also for the other models): terms from the model and from its
  sentence, files ranked by matches, names and what belongs together, content within a budget
- Verified with the real CLI (Haiku): one `collect_files`, two edits, a test run in the dev
  container, 13 s; a follow-up that resumed the session; a follow-up after the session was
  removed; stopping an answer (no process left)
- Not verified: with an API key instead of the signed-in account; several answers at once;
  images returned by tools through MCP; Claude Code's own sub-agents (`Task`)

### Projects (2026-11-20)
- `projects`, `project_groups`, `project_files`, `chats.project_id` (migration 0006);
  `routes/projects.rs`: list, create, change, share, delete, files
- A project belongs to its maker. Shared with everyone or groups (members: their own groups,
  admins: all), others read it and start chats of their own; chats are never shared
- For an answer: `projects::Context` on the `Job`. Instructions (only when
  `allowUserSystemPrompt`) and text files go into the system prompt, images and PDFs in front of
  the first message of every request (also after a summary), for models with vision
- Claude Code: instructions over 60 kB go through `--append-system-prompt-file`; a new session
  gets the project's images and PDFs, a resumed one has them (files added later reach it only
  with a new session)
- The size of the system prompt counts toward the point where a conversation is summarized
- Web: `/projects`, `/projects/{id}`, project chip and picker in a chat, tag in the chat list.
  Desktop: project list above the chats, folder remembered per project, `agora-agent ask --project`
- Verified with the mock model (two users, a group) and in Chromium; not with a real model, and
  not through Claude Code
- Open: auditors don't see a project's files from a conversation; no search inside files (all of
  it is sent each time); the OpenAI-compatible API has no projects

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
