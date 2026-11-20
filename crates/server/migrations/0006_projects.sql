-- Projects: chats that belong together, with instructions and reference files the model gets
-- with every message. A project belongs to one person, who can share it; the chats in it stay
-- each person's own.
CREATE TABLE projects (
    id            uuid PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          text NOT NULL,
    description   text NOT NULL DEFAULT '',
    instructions  text NOT NULL DEFAULT '',
    -- private | everyone | groups (only members of project_groups)
    access        text NOT NULL DEFAULT 'private' CHECK (access IN ('private', 'everyone', 'groups')),
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX projects_user ON projects (user_id);

CREATE TABLE project_groups (
    project_id  uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    group_id    uuid NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, group_id)
);

-- Text files are given to the model as text (`chars` is their length), images and PDFs as files.
CREATE TABLE project_files (
    id          uuid PRIMARY KEY,
    project_id  uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name        text NOT NULL,
    media_type  text NOT NULL,
    size_bytes  integer NOT NULL,
    chars       integer,
    data        bytea NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX project_files_project ON project_files (project_id, created_at);

-- A chat keeps existing when its project is deleted.
ALTER TABLE chats ADD COLUMN project_id uuid REFERENCES projects(id) ON DELETE SET NULL;
CREATE INDEX chats_project ON chats (project_id) WHERE project_id IS NOT NULL;
