CREATE TABLE distribution_artifacts (
    id text PRIMARY KEY,
    manifest jsonb NOT NULL,
    ready boolean NOT NULL DEFAULT false,
    created_at bigint NOT NULL
);
CREATE TABLE distribution_chunks (
    artifact_id text NOT NULL REFERENCES distribution_artifacts(id),
    chunk_index integer NOT NULL,
    content bytea NOT NULL,
    PRIMARY KEY (artifact_id,chunk_index)
);
CREATE TABLE distribution_devices (
    device_id text PRIMARY KEY REFERENCES devices(id),
    adapter text NOT NULL CHECK(adapter IN ('service','simulation')),
    generation bigint NOT NULL,
    last_seen bigint NOT NULL
);
CREATE TABLE distribution_jobs (
    id text PRIMARY KEY,
    request_id text NOT NULL,
    actor_id text NOT NULL REFERENCES operators(id),
    actor_version bigint NOT NULL,
    device_id text NOT NULL REFERENCES devices(id),
    generation bigint NOT NULL,
    room_ids text[] NOT NULL,
    artifact_id text NOT NULL REFERENCES distribution_artifacts(id),
    directory text NOT NULL,
    adapter text NOT NULL,
    publish_at bigint NOT NULL,
    bytes_per_second bigint NOT NULL,
    stage text NOT NULL CHECK(stage IN ('queued','downloading','ready','published','failed','cancelled')),
    verified_chunks integer NOT NULL DEFAULT 0,
    detail text NOT NULL DEFAULT '',
    updated_at bigint NOT NULL
);
CREATE INDEX distribution_job_device ON distribution_jobs(device_id,updated_at);
CREATE UNIQUE INDEX distribution_one_active_artifact ON distribution_jobs(device_id,artifact_id) WHERE stage IN ('queued','downloading','ready');
CREATE FUNCTION distribution_event() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.stage IN ('published','failed','cancelled') AND OLD.stage IS DISTINCT FROM NEW.stage THEN
        INSERT INTO events(source,source_id,category,occurred_at,actor_id,operation,target,device_ids,room_ids,request_id,outcome,detail)
        VALUES('distribution',NEW.id,'device_report',NEW.updated_at,NEW.actor_id,'file_distribution',NEW.device_id,
            ARRAY[NEW.device_id],NEW.room_ids,NEW.request_id,NEW.stage,jsonb_build_object('artifact_id',NEW.artifact_id,'detail',NEW.detail,'adapter',NEW.adapter))
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER distribution_result_event AFTER UPDATE ON distribution_jobs FOR EACH ROW EXECUTE FUNCTION distribution_event();
UPDATE schema_version SET version=15;
