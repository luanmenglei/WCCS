ALTER TABLE control_devices ADD COLUMN pause_lease boolean NOT NULL DEFAULT false;
ALTER TABLE maintenance_jobs ADD COLUMN reboot_observed_at bigint, ADD COLUMN reboot_requested_at bigint;
UPDATE maintenance_jobs SET reboot_requested_at=updated_at WHERE stage='succeeded'
    AND command#>>'{template,options,action}'='reboot' AND result->>'detail'='reboot_requested';
ALTER TABLE distribution_jobs ADD COLUMN publication_claimed boolean NOT NULL DEFAULT false;
-- Existing volume jobs predate per-device volume approval and need a new preview.
UPDATE maintenance_jobs SET cancel_requested=true WHERE stage IN ('queued','accepted','executing')
    AND command#>>'{template,options,scope}'='volume';
-- Legacy servers could already have issued a release permission for these jobs.
UPDATE distribution_jobs SET publication_claimed=true WHERE stage IN ('queued','downloading','ready')
    AND publish_at <= EXTRACT(EPOCH FROM CURRENT_TIMESTAMP)::bigint;

-- Keep recurring dashboard reads independent of manifest size, including old jobs.
CREATE FUNCTION recovery_file_bytes(files jsonb) RETURNS numeric
    LANGUAGE sql IMMUTABLE STRICT AS $$
    SELECT COALESCE(sum((f->>'size')::numeric),0) FROM jsonb_array_elements(files) f
$$;
ALTER TABLE maintenance_jobs
    ADD COLUMN recovery_contest text GENERATED ALWAYS AS (CASE WHEN command#>>'{template,operation}'='recovery' THEN command#>>'{template,options,contest_id}' END) STORED,
    ADD COLUMN recovery_template jsonb GENERATED ALWAYS AS (CASE WHEN command#>>'{template,operation}'='recovery' THEN command->'template' END) STORED,
    ADD COLUMN recovery_summary jsonb GENERATED ALWAYS AS (CASE WHEN command#>>'{template,operation}'='recovery' THEN result - 'recovery' END) STORED,
    ADD COLUMN recovery_files integer GENERATED ALWAYS AS (jsonb_array_length(result#>'{recovery,files}')) STORED,
    ADD COLUMN recovery_bytes numeric GENERATED ALWAYS AS (recovery_file_bytes(result#>'{recovery,files}')) STORED,
    ADD COLUMN recovery_directories integer GENERATED ALWAYS AS (jsonb_array_length(result#>'{recovery,directories}')) STORED;
CREATE INDEX maintenance_recovery_history ON maintenance_jobs (recovery_contest, created_at DESC, id)
    WHERE recovery_contest IS NOT NULL;
UPDATE schema_version SET version=19;
