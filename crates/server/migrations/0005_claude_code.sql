-- Providers of kind `claude_code`: models that answer through Claude Code on the server.
ALTER TABLE providers DROP CONSTRAINT providers_kind_check;
ALTER TABLE providers ADD CONSTRAINT providers_kind_check CHECK (kind IN ('anthropic', 'openai', 'claude_code'));
