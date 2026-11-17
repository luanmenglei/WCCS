CREATE TABLE contest_control_jobs (
    id text PRIMARY KEY,
    actor_id text NOT NULL REFERENCES operators(id),
    actor_version bigint NOT NULL,
    stage text NOT NULL CHECK(stage IN ('scheduled','running','succeeded','failed','cancelled','interrupted')),
    execute_at bigint NOT NULL,
    payload jsonb NOT NULL,
    plan jsonb NOT NULL,
    results jsonb NOT NULL DEFAULT '{}',
    detail text NOT NULL DEFAULT '',
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE UNIQUE INDEX contest_control_one_active ON contest_control_jobs ((true)) WHERE stage IN ('scheduled','running');
UPDATE schema_version SET version=16;
