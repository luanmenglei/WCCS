CREATE TABLE image_applications (
    id text PRIMARY KEY,
    device_id text NOT NULL REFERENCES devices(id),
    machine_name text NOT NULL,
    enrollment_hash text NOT NULL,
    credential_hash text NOT NULL,
    observed_ip text,
    created_at bigint NOT NULL,
    expires_at bigint NOT NULL,
    status text NOT NULL CHECK (status IN ('pending','approved','rejected','superseded')),
    bootstrap_status text NOT NULL
);
CREATE INDEX image_applications_device ON image_applications(device_id,created_at);
UPDATE schema_version SET version=13;
