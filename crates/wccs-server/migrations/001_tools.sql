CREATE TABLE IF NOT EXISTS schema_version (version integer PRIMARY KEY);
INSERT INTO schema_version VALUES (1) ON CONFLICT DO NOTHING;
CREATE TABLE IF NOT EXISTS configuration_head (id integer PRIMARY KEY CHECK (id = 1), version bigint NOT NULL);
INSERT INTO configuration_head VALUES (1, 0) ON CONFLICT DO NOTHING;
CREATE TABLE IF NOT EXISTS configurations (
    version bigint PRIMARY KEY,
    snapshot jsonb NOT NULL,
    targets jsonb NOT NULL,
    created_at bigint NOT NULL
);
CREATE TABLE IF NOT EXISTS devices (
    id text PRIMARY KEY,
    name text NOT NULL,
    enrollment_hash text NOT NULL,
    enrollment_expires bigint NOT NULL,
    credential_hash text,
    last_seen bigint,
    boot_id text,
    agent_version text,
    desired_job text
);
CREATE TABLE IF NOT EXISTS jobs (
    sequence bigserial UNIQUE NOT NULL,
    id text PRIMARY KEY,
    device_id text NOT NULL REFERENCES devices(id),
    config_version bigint NOT NULL REFERENCES configurations(version),
    expires_at bigint NOT NULL,
    result jsonb,
    received_at bigint
);
CREATE UNIQUE INDEX IF NOT EXISTS devices_credential_hash ON devices(credential_hash);
CREATE INDEX IF NOT EXISTS jobs_device_sequence ON jobs(device_id, sequence DESC);
CREATE TABLE IF NOT EXISTS requests (
    id text PRIMARY KEY,
    operation text NOT NULL,
    payload jsonb NOT NULL,
    response jsonb NOT NULL,
    created_at bigint NOT NULL
);
CREATE TABLE IF NOT EXISTS audit_events (
    id bigserial PRIMARY KEY,
    actor text NOT NULL,
    operation text NOT NULL,
    request_id text NOT NULL,
    created_at bigint NOT NULL
);
