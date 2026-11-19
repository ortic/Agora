-- Files people attach to a message (images, PDFs). The message's `parts` name them
-- (`{"type":"attachment","id":…,"name":…,"mediaType":…,"size":…}`); the bytes are here.
CREATE TABLE attachments (
    id          uuid PRIMARY KEY,
    message_id  uuid NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    name        text NOT NULL,
    media_type  text NOT NULL,
    size_bytes  integer NOT NULL,
    data        bytea NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX attachments_message ON attachments (message_id);
