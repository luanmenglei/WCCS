CREATE TABLE device_health (
    device_id text PRIMARY KEY REFERENCES devices(id),
    enrollment_generation bigint NOT NULL,
    instance_id text NOT NULL,
    sequence bigint NOT NULL,
    report jsonb NOT NULL,
    received_at bigint NOT NULL
);
UPDATE schema_version SET version=10;
