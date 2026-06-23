CREATE TABLE upload_sessions (
    id             UUID PRIMARY KEY,
    owner_id       TEXT NOT NULL REFERENCES drive_users(user_id) ON DELETE CASCADE,
    parent_id      UUID NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    file_name      TEXT NOT NULL,
    declared_mime  TEXT,
    declared_size  BIGINT NOT NULL,
    reserved_bytes BIGINT NOT NULL DEFAULT 0 CHECK (reserved_bytes >= 0),
    chunk_size     INT NOT NULL,
    chunk_count    INT NOT NULL,
    checksum       TEXT,
    storage_key    TEXT NOT NULL,
    tmp_key        TEXT NOT NULL,
    state          TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'completing', 'completed', 'aborted')),
    received_bytes BIGINT NOT NULL DEFAULT 0,
    node_id        UUID,
    expires_at     TIMESTAMPTZ NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE upload_chunks (
    session_id   UUID NOT NULL REFERENCES upload_sessions(id) ON DELETE CASCADE,
    chunk_index  INT NOT NULL,
    size_bytes   INT NOT NULL,
    chunk_sha256 TEXT,
    received_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (session_id, chunk_index)
);

CREATE INDEX upload_sessions_gc ON upload_sessions (state, expires_at);
CREATE INDEX upload_sessions_owner ON upload_sessions (owner_id);
