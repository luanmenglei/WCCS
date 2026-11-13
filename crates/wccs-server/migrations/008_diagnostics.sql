CREATE TABLE device_diagnostics (
    request_id text PRIMARY KEY,
    device_id text NOT NULL REFERENCES devices(id),
    enrollment_generation bigint NOT NULL,
    report jsonb NOT NULL,
    received_at bigint NOT NULL
);
CREATE INDEX device_diagnostics_latest ON device_diagnostics(device_id, received_at DESC);
UPDATE schema_version SET version=8;
