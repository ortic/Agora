-- MCP servers the Agora server itself connects to: their tools are offered to models in every
-- chat (web app included) of the people they are granted to. Like local connectors, but they
-- run here instead of on people's computers.
CREATE TABLE mcp_servers (
    id            uuid PRIMARY KEY,
    name          text NOT NULL,
    -- mcp_stdio (a program the server starts, speaking MCP on stdin/stdout)
    -- mcp_http  (an MCP server at a URL, streamable HTTP)
    kind          text NOT NULL CHECK (kind IN ('mcp_stdio', 'mcp_http')),
    description   text,
    -- mcp_stdio: {"command": "...", "args": [...]}; mcp_http: {"url": "..."}
    config        jsonb NOT NULL DEFAULT '{}',
    -- encrypted (crypto.rs) JSON {"env": {...}, "headers": {...}}: API keys and tokens
    secrets_enc   bytea,
    -- only these tools of the server are offered to models (empty: all)
    tools         text[] NOT NULL DEFAULT '{}',
    -- ask (the person confirms each call in the chat) | auto
    approval      text NOT NULL DEFAULT 'ask' CHECK (approval IN ('ask', 'auto')),
    enabled       boolean NOT NULL DEFAULT true,
    access        text NOT NULL DEFAULT 'groups' CHECK (access IN ('all', 'groups')),
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX mcp_servers_name ON mcp_servers (lower(name));

CREATE TABLE mcp_server_groups (
    server_id  uuid NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    group_id   uuid NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    PRIMARY KEY (server_id, group_id)
);

-- calls of these tools are recorded in tool_events too: feature 'mcp', no device,
-- connector_id / connector = the server
