-- Agora schema. Single tenant: one organisation per installation.

-- organisation-wide settings (one row)
CREATE TABLE settings (
    id          boolean PRIMARY KEY DEFAULT true CHECK (id),
    data        jsonb NOT NULL DEFAULT '{}',
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE users (
    id                  uuid PRIMARY KEY,
    email               text,
    name                text NOT NULL,
    password_hash       text,
    -- owner | admin | auditor | member
    role                text NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'auditor', 'member')),
    -- active | deactivated
    status              text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'deactivated')),
    -- per-user monthly budget in USD (overrides group and default budgets); NULL = inherit
    monthly_budget_usd  double precision,
    created_at          timestamptz NOT NULL DEFAULT now(),
    last_login_at       timestamptz,
    last_active_at      timestamptz
);
CREATE UNIQUE INDEX users_email ON users (lower(email));

CREATE TABLE sessions (
    token_hash    bytea PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL,
    last_seen_at  timestamptz NOT NULL DEFAULT now(),
    user_agent    text,
    ip            text
);
CREATE INDEX sessions_user ON sessions (user_id);

CREATE TABLE api_tokens (
    id            uuid PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          text NOT NULL,
    token_hash    bytea NOT NULL UNIQUE,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz,
    last_used_at  timestamptz
);

-- external identities (SSO): (provider key, subject) → user
CREATE TABLE identities (
    provider    text NOT NULL,
    subject     text NOT NULL,
    user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    email       text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    last_login  timestamptz,
    PRIMARY KEY (provider, subject)
);

-- short-lived SSO login state (shared by all replicas)
CREATE TABLE auth_states (
    state       text PRIMARY KEY,
    data        jsonb NOT NULL,
    expires_at  timestamptz NOT NULL
);

CREATE TABLE login_attempts (
    email         text PRIMARY KEY,
    failures      integer NOT NULL,
    window_start  timestamptz NOT NULL
);

CREATE TABLE groups (
    id                  uuid PRIMARY KEY,
    name                text NOT NULL,
    description         text,
    -- `sso:<provider>:<name>` for groups mirrored from an identity provider
    external_ref        text,
    monthly_budget_usd  double precision,
    created_at          timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX groups_name ON groups (lower(name));

CREATE TABLE group_members (
    group_id  uuid NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id   uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);
CREATE INDEX group_members_user ON group_members (user_id);

-- model providers (connections)
CREATE TABLE providers (
    id            uuid PRIMARY KEY,
    name          text NOT NULL,
    -- anthropic | openai (any OpenAI-compatible API)
    kind          text NOT NULL CHECK (kind IN ('anthropic', 'openai')),
    base_url      text NOT NULL,
    -- AES-256-GCM (nonce || ciphertext), see crypto.rs
    api_key_enc   bytea,
    api_key_hint  text,
    -- extra HTTP headers sent with every request (e.g. an organisation header)
    headers       jsonb NOT NULL DEFAULT '{}',
    enabled       boolean NOT NULL DEFAULT true,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE models (
    id                 uuid PRIMARY KEY,
    provider_id        uuid NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    -- the model's name at the provider (`claude-opus-5-5`, `llama3.3:70b`, …)
    model              text NOT NULL,
    display_name       text NOT NULL,
    description        text,
    enabled            boolean NOT NULL DEFAULT true,
    -- USD per million tokens
    input_price        double precision NOT NULL DEFAULT 0,
    output_price       double precision NOT NULL DEFAULT 0,
    cache_read_price   double precision NOT NULL DEFAULT 0,
    cache_write_price  double precision NOT NULL DEFAULT 0,
    -- USD per web search done by the provider itself (Claude's server-side search)
    search_price       double precision NOT NULL DEFAULT 0,
    context_window     integer,
    max_output_tokens  integer,
    -- {"vision": bool, "tools": bool, "nativeSearch": bool, "reasoning": bool}
    capabilities       jsonb NOT NULL DEFAULT '{}',
    -- defaults: {"effort": "medium", "systemPrompt": "...", "temperature": 0.7}
    params             jsonb NOT NULL DEFAULT '{}',
    -- all | groups (only members of model_groups)
    access             text NOT NULL DEFAULT 'all' CHECK (access IN ('all', 'groups')),
    position           integer NOT NULL DEFAULT 0,
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider_id, model)
);

CREATE TABLE model_groups (
    model_id  uuid NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    group_id  uuid NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    PRIMARY KEY (model_id, group_id)
);

CREATE TABLE chats (
    id             uuid PRIMARY KEY,
    user_id        uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title          text NOT NULL DEFAULT 'New chat',
    model_id       uuid REFERENCES models(id) ON DELETE SET NULL,
    system_prompt  text,
    -- {"webSearch": bool, "effort": "..."}
    settings       jsonb NOT NULL DEFAULT '{}',
    -- the message at the end of the branch being shown
    leaf_id        uuid,
    pinned         boolean NOT NULL DEFAULT false,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now(),
    -- soft delete: hidden from the user, kept for auditors until the retention purge
    deleted_at     timestamptz
);
CREATE INDEX chats_user ON chats (user_id, updated_at DESC) WHERE deleted_at IS NULL;

CREATE TABLE messages (
    id           uuid PRIMARY KEY,
    chat_id      uuid NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    parent_id    uuid REFERENCES messages(id) ON DELETE CASCADE,
    -- user | assistant
    role         text NOT NULL CHECK (role IN ('user', 'assistant')),
    content      text NOT NULL DEFAULT '',
    -- what the UI shows besides the text: reasoning, tool calls, sources
    parts        jsonb NOT NULL DEFAULT '[]',
    -- the provider's own content blocks (replayed to the same model: thinking signatures, tool use)
    raw          jsonb,
    model_id     uuid REFERENCES models(id) ON DELETE SET NULL,
    model_name   text,
    -- complete | streaming | error | stopped
    status       text NOT NULL DEFAULT 'complete',
    error        text,
    -- tokens and cost of producing this message (all model calls of its tool loop)
    input_tokens   bigint NOT NULL DEFAULT 0,
    output_tokens  bigint NOT NULL DEFAULT 0,
    cost_usd       double precision NOT NULL DEFAULT 0,
    duration_ms    integer,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX messages_chat ON messages (chat_id, created_at);
CREATE INDEX messages_parent ON messages (parent_id);

-- one row per call to a model
CREATE TABLE usage_events (
    id                  uuid PRIMARY KEY,
    user_id             uuid REFERENCES users(id) ON DELETE SET NULL,
    chat_id             uuid,
    message_id          uuid,
    model_id            uuid REFERENCES models(id) ON DELETE SET NULL,
    provider_id         uuid REFERENCES providers(id) ON DELETE SET NULL,
    -- snapshot: survives model deletion
    model_name          text NOT NULL,
    -- chat | title | api
    source              text NOT NULL,
    input_tokens        bigint NOT NULL DEFAULT 0,
    output_tokens       bigint NOT NULL DEFAULT 0,
    cache_read_tokens   bigint NOT NULL DEFAULT 0,
    cache_write_tokens  bigint NOT NULL DEFAULT 0,
    -- searches done by the provider itself (billed by it)
    provider_searches   integer NOT NULL DEFAULT 0,
    cost_usd            double precision NOT NULL DEFAULT 0,
    latency_ms          integer,
    -- ok | error | stopped
    status              text NOT NULL DEFAULT 'ok',
    error               text,
    created_at          timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX usage_events_user ON usage_events (user_id, created_at);
CREATE INDEX usage_events_time ON usage_events (created_at);

-- every web search and page fetch done for someone
CREATE TABLE web_events (
    id           uuid PRIMARY KEY,
    user_id      uuid REFERENCES users(id) ON DELETE SET NULL,
    chat_id      uuid,
    message_id   uuid,
    -- search | fetch
    kind         text NOT NULL,
    -- searxng | brave | tavily | anthropic (Claude's server-side search) | fetch
    engine       text NOT NULL,
    query        text,
    url          text,
    -- [{title, url, snippet}]
    results      jsonb NOT NULL DEFAULT '[]',
    result_count integer NOT NULL DEFAULT 0,
    -- ok | error | blocked
    status       text NOT NULL,
    error        text,
    duration_ms  integer,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX web_events_time ON web_events (created_at);
CREATE INDEX web_events_user ON web_events (user_id, created_at);

-- ---- audit log: append-only, hash-chained ------------------------------------------------
CREATE TABLE audit_events (
    id           bigserial PRIMARY KEY,
    at           timestamptz NOT NULL DEFAULT now(),
    actor_id     uuid,
    -- e-mail or name at the time (survives user deletion)
    actor_label  text,
    ip           text,
    user_agent   text,
    action       text NOT NULL,
    target_type  text,
    target_id    text,
    data         jsonb NOT NULL DEFAULT '{}',
    prev_hash    bytea NOT NULL,
    hash         bytea NOT NULL
);
CREATE INDEX audit_events_at ON audit_events (at);
CREATE INDEX audit_events_actor ON audit_events (actor_id, at);
CREATE INDEX audit_events_action ON audit_events (action, at);

-- what a row's hash covers (verification recomputes exactly this)
CREATE FUNCTION audit_digest(prev bytea, id bigint, at timestamptz, actor_id uuid, actor_label text, ip text, user_agent text,
                             action text, target_type text, target_id text, data jsonb) RETURNS bytea
LANGUAGE sql IMMUTABLE AS $$
    SELECT sha256(prev || convert_to(concat_ws(E'\x1f', id::text, to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US'),
        coalesce(actor_id::text, ''), coalesce(actor_label, ''), coalesce(ip, ''), coalesce(user_agent, ''), action,
        coalesce(target_type, ''), coalesce(target_id, ''), data::text), 'UTF8'))
$$;

-- links each new row to the previous one; inserts are serialized by an advisory lock
CREATE FUNCTION audit_chain() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    last bytea;
BEGIN
    PERFORM pg_advisory_xact_lock(727274);
    SELECT hash INTO last FROM audit_events ORDER BY id DESC LIMIT 1;
    NEW.prev_hash := coalesce(last, '\x'::bytea);
    NEW.at := clock_timestamp();
    NEW.hash := audit_digest(NEW.prev_hash, NEW.id, NEW.at, NEW.actor_id, NEW.actor_label, NEW.ip, NEW.user_agent,
                             NEW.action, NEW.target_type, NEW.target_id, NEW.data);
    RETURN NEW;
END $$;
CREATE TRIGGER audit_chain BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION audit_chain();

-- no edits; deletes only through the retention purge (which sets agora.audit_purge for its transaction)
CREATE FUNCTION audit_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND current_setting('agora.audit_purge', true) = 'on' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'audit_events is append-only';
END $$;
CREATE TRIGGER audit_guard BEFORE UPDATE OR DELETE ON audit_events FOR EACH ROW EXECUTE FUNCTION audit_guard();
CREATE TRIGGER audit_no_truncate BEFORE TRUNCATE ON audit_events FOR EACH STATEMENT EXECUTE FUNCTION audit_guard();
