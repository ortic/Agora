-- Knowledge sources the models can search (RAG): WikiCraft for now. Each person connects their
-- own token, so results are filtered by the source's own permissions.
CREATE TABLE connectors (
    id           uuid PRIMARY KEY,
    name         text NOT NULL,
    -- wikicraft
    kind         text NOT NULL CHECK (kind IN ('wikicraft')),
    base_url     text NOT NULL,
    -- what the model is told the source contains
    description  text,
    enabled      boolean NOT NULL DEFAULT true,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE connector_tokens (
    connector_id  uuid NOT NULL REFERENCES connectors(id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_enc     bytea NOT NULL,
    token_hint    text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz,
    PRIMARY KEY (connector_id, user_id)
);
