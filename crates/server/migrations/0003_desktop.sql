-- Agora Desktop: a client on people's computers that runs tools for the model there (code in
-- dev containers, a browser, local connectors). The server decides who gets which of them.

-- what the desktop app may do, and for whom
CREATE TABLE desktop_features (
    -- desktop (signing in with the app at all) | code | browser
    key         text PRIMARY KEY CHECK (key IN ('desktop', 'code', 'browser')),
    -- nobody | all | groups (members of desktop_feature_groups; administrators always)
    access      text NOT NULL DEFAULT 'nobody' CHECK (access IN ('nobody', 'all', 'groups')),
    -- per feature, see desktop.rs (allowed images, allowed domains, approval mode, …)
    config      jsonb NOT NULL DEFAULT '{}',
    updated_at  timestamptz NOT NULL DEFAULT now()
);
INSERT INTO desktop_features (key) VALUES ('desktop'), ('code'), ('browser');

CREATE TABLE desktop_feature_groups (
    key       text NOT NULL REFERENCES desktop_features(key) ON DELETE CASCADE,
    group_id  uuid NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    PRIMARY KEY (key, group_id)
);

-- local connectors: programs and services on the person's computer the model may use through
-- the desktop app. Administrators define them; the app only starts what is listed here.
CREATE TABLE local_connectors (
    id           uuid PRIMARY KEY,
    name         text NOT NULL,
    -- mcp_stdio (a program speaking the Model Context Protocol on stdin/stdout)
    -- mcp_http  (an MCP server at a URL reachable from the computer)
    kind         text NOT NULL CHECK (kind IN ('mcp_stdio', 'mcp_http')),
    description  text,
    -- mcp_stdio: {"command": "...", "args": [...], "env": {...}}; mcp_http: {"url": "...", "headers": {...}}
    config       jsonb NOT NULL DEFAULT '{}',
    -- only these tools of the server are offered to models (empty: all)
    tools        text[] NOT NULL DEFAULT '{}',
    -- ask (the person confirms each call) | auto
    approval     text NOT NULL DEFAULT 'ask' CHECK (approval IN ('ask', 'auto')),
    enabled      boolean NOT NULL DEFAULT true,
    access       text NOT NULL DEFAULT 'groups' CHECK (access IN ('all', 'groups')),
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX local_connectors_name ON local_connectors (lower(name));

CREATE TABLE local_connector_groups (
    connector_id  uuid NOT NULL REFERENCES local_connectors(id) ON DELETE CASCADE,
    group_id      uuid NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    PRIMARY KEY (connector_id, group_id)
);

-- a computer signed in with Agora Desktop
CREATE TABLE devices (
    id            uuid PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          text NOT NULL,
    platform      text,
    app_version   text,
    token_hash    bytea NOT NULL UNIQUE,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_seen_at  timestamptz,
    revoked_at    timestamptz
);
CREATE INDEX devices_user ON devices (user_id);

-- signing a device in: the app shows a code, the person confirms it in the web app
CREATE TABLE device_logins (
    -- sha256 of the secret the app polls with
    poll_hash    bytea PRIMARY KEY,
    user_code    text NOT NULL UNIQUE,
    device_name  text NOT NULL,
    platform     text,
    app_version  text,
    ip           text,
    -- pending | approved | denied
    status       text NOT NULL DEFAULT 'pending',
    user_id      uuid REFERENCES users(id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz NOT NULL
);

-- every tool call a model made on someone's computer
CREATE TABLE tool_events (
    id            uuid PRIMARY KEY,
    user_id       uuid REFERENCES users(id) ON DELETE SET NULL,
    device_id     uuid,
    chat_id       uuid,
    message_id    uuid,
    -- code | browser | connector
    feature       text NOT NULL,
    connector_id  uuid,
    -- the connector's name at the time
    connector     text,
    tool          text NOT NULL,
    input         jsonb NOT NULL DEFAULT '{}',
    -- ok | error | denied (by the person) | blocked (by policy) | timeout
    status        text NOT NULL,
    error         text,
    result_chars  integer NOT NULL DEFAULT 0,
    duration_ms   integer,
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX tool_events_time ON tool_events (created_at);
CREATE INDEX tool_events_user ON tool_events (user_id, created_at);
