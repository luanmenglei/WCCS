CREATE TABLE domjudge_jobs (
    id text PRIMARY KEY,
    actor_id text NOT NULL REFERENCES operators(id),
    actor_version bigint NOT NULL,
    stage text NOT NULL CHECK (stage IN ('queued','running','succeeded','failed','interrupted')),
    payload jsonb NOT NULL,
    plan jsonb NOT NULL,
    results jsonb NOT NULL DEFAULT '[]',
    detail text NOT NULL DEFAULT '',
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE UNIQUE INDEX domjudge_one_active_job ON domjudge_jobs ((true)) WHERE stage IN ('queued','running');
CREATE TABLE domjudge_bindings (
    participant_id text PRIMARY KEY REFERENCES participants(id),
    remote_contest_id text NOT NULL,
    username text NOT NULL UNIQUE,
    ip text NOT NULL,
    target_digest text NOT NULL,
    updated_at bigint NOT NULL
);
UPDATE schema_version SET version=14;
