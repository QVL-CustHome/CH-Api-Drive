ALTER TABLE nodes
    ADD CONSTRAINT nodes_media_type_check
    CHECK (media_type IS NULL OR media_type IN ('image', 'video'));
