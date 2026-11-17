CREATE TABLE recovery_archives (
    job_id text PRIMARY KEY REFERENCES maintenance_jobs(id),
    snapshot jsonb NOT NULL,
    options jsonb NOT NULL,
    verified boolean NOT NULL DEFAULT false
);
CREATE TABLE recovery_chunks (
    job_id text NOT NULL REFERENCES recovery_archives(job_id),
    file_index integer NOT NULL,
    chunk_index integer NOT NULL,
    content bytea NOT NULL,
    PRIMARY KEY(job_id,file_index,chunk_index)
);
UPDATE schema_version SET version=18;
