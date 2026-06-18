CREATE TABLE drive_users (
    user_id      TEXT PRIMARY KEY,
    quota_bytes  BIGINT NOT NULL,
    used_bytes   BIGINT NOT NULL DEFAULT 0,
    root_node_id UUID NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE nodes (
    id           UUID PRIMARY KEY,
    owner_id     TEXT NOT NULL REFERENCES drive_users(user_id) ON DELETE CASCADE,
    parent_id    UUID REFERENCES nodes(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('folder', 'file')),
    name         TEXT NOT NULL,
    mime         TEXT,
    size_bytes   BIGINT NOT NULL DEFAULT 0,
    storage_key  TEXT,
    content_hash TEXT,
    is_media     BOOLEAN NOT NULL DEFAULT FALSE,
    media_type   TEXT,
    taken_at     TIMESTAMPTZ,
    width        INT,
    height       INT,
    duration_ms  INT,
    trashed_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX nodes_unique_name ON nodes (owner_id, parent_id, name) WHERE trashed_at IS NULL;
CREATE INDEX nodes_listing ON nodes (owner_id, parent_id);
CREATE INDEX nodes_gallery ON nodes (owner_id, is_media, taken_at DESC);
CREATE INDEX nodes_hash ON nodes (owner_id, content_hash);
