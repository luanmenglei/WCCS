CREATE TABLE maintenance_jobs (
    id text PRIMARY KEY,
    request_id text NOT NULL,
    device_id text NOT NULL REFERENCES devices(id),
    command jsonb NOT NULL,
    command_digest text NOT NULL,
    actor_id text NOT NULL REFERENCES operators(id),
    actor_version bigint NOT NULL,
    room_ids text[],
    created_at bigint NOT NULL,
    expires_at bigint NOT NULL,
    stage text NOT NULL DEFAULT 'queued',
    cancel_requested boolean NOT NULL DEFAULT false,
    result jsonb,
    updated_at bigint NOT NULL,
    UNIQUE(request_id,device_id)
);
CREATE INDEX maintenance_device_queue ON maintenance_jobs(device_id,created_at);
CREATE TABLE maintenance_devices (
    device_id text PRIMARY KEY REFERENCES devices(id),
    instance_id text NOT NULL,
    session_key text,
    adapter text NOT NULL,
    capabilities jsonb NOT NULL,
    last_seen bigint NOT NULL
);
UPDATE schema_version SET version=9;
