CREATE TABLE preparation_profiles (
    version bigint PRIMARY KEY,
    profile jsonb NOT NULL,
    created_at bigint NOT NULL
);
UPDATE schema_version SET version=12;
