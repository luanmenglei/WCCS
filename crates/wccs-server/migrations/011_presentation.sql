CREATE TABLE presentation_settings (
    contest_id text PRIMARY KEY REFERENCES contests(id),
    version bigint NOT NULL,
    settings jsonb NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE presentation_receipts (
    device_id text PRIMARY KEY REFERENCES devices(id),
    command_id text NOT NULL,
    report jsonb NOT NULL,
    received_at bigint NOT NULL
);
UPDATE schema_version SET version=11;
