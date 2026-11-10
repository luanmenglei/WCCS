ALTER TABLE devices ADD COLUMN lifecycle text NOT NULL DEFAULT 'pending'
    CHECK (lifecycle IN ('pending','enabled','disabled','revoked','rejected','expired'));
ALTER TABLE devices ADD COLUMN lifecycle_version bigint NOT NULL DEFAULT 1;
ALTER TABLE devices ADD COLUMN enrollment_generation bigint NOT NULL DEFAULT 1;
ALTER TABLE devices ADD COLUMN applicant_hash text;
ALTER TABLE devices ADD COLUMN application_id text;
ALTER TABLE devices ADD COLUMN application_at bigint;
ALTER TABLE devices ADD COLUMN rotation_id text;
ALTER TABLE devices ADD COLUMN rotation_expires bigint;
UPDATE devices SET lifecycle='enabled' WHERE credential_hash IS NOT NULL;
CREATE TABLE credential_history (
    hash text PRIMARY KEY,
    device_id text NOT NULL REFERENCES devices(id),
    generation bigint NOT NULL,
    created_at bigint NOT NULL
);
INSERT INTO credential_history SELECT credential_hash,id,1,0 FROM devices WHERE credential_hash IS NOT NULL;
CREATE TABLE credential_rotations (
    id text PRIMARY KEY,
    device_id text NOT NULL REFERENCES devices(id),
    old_hash text NOT NULL,
    new_hash text NOT NULL,
    expires_at bigint NOT NULL
);
CREATE TABLE registration_window (
    id integer PRIMARY KEY CHECK (id=1),
    version bigint NOT NULL,
    closes_at bigint NOT NULL
);
INSERT INTO registration_window VALUES (1,1,0);
ALTER TABLE jobs ADD COLUMN cancelled_at bigint;
ALTER TABLE jobs ADD COLUMN cancellation_reason text;
CREATE TABLE device_events (
    sequence bigserial PRIMARY KEY,
    device_id text NOT NULL REFERENCES devices(id),
    operation text NOT NULL,
    request_id text NOT NULL,
    version bigint NOT NULL,
    created_at bigint NOT NULL
);
UPDATE schema_version SET version=3;
