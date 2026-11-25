ALTER TABLE maintenance_jobs
    ADD COLUMN grant_connection text, ADD COLUMN grant_epoch text,
    ADD COLUMN recovery_reference jsonb, ADD COLUMN delivery_command jsonb,
    ADD COLUMN delivered boolean NOT NULL DEFAULT true;
-- Legacy jobs may have been delivered before the upgrade.
ALTER TABLE maintenance_jobs ALTER COLUMN delivered SET DEFAULT false;
CREATE FUNCTION recovery_collected_count(files jsonb, extensions jsonb) RETURNS integer
    LANGUAGE sql IMMUTABLE STRICT AS $$
    SELECT count(*)::integer FROM jsonb_array_elements(files) f
    WHERE strpos(f->>'path','.')>0 AND EXISTS (
        SELECT 1 FROM jsonb_array_elements_text(extensions) ext
        WHERE lower(ext COLLATE "C")=lower(split_part(f->>'path','.',-1) COLLATE "C")
    )
$$;
-- Keep batch approval reads independent of the immutable command's file list.
ALTER TABLE maintenance_jobs
    ADD COLUMN recovery_volume jsonb GENERATED ALWAYS AS (result#>'{recovery,volume}') STORED,
    ADD COLUMN recovery_collected integer GENERATED ALWAYS AS (recovery_collected_count(result#>'{recovery,files}',command#>'{template,options,extensions}')) STORED,
    ADD COLUMN recovery_instance text GENERATED ALWAYS AS (command->>'instance_id') STORED,
    ADD COLUMN recovery_generation bigint GENERATED ALWAYS AS ((command->>'generation')::bigint) STORED,
    ADD COLUMN inspection_source text GENERATED ALWAYS AS (COALESCE(recovery_reference->>'inspection_job',command#>>'{recovery,inspection_job}')) STORED,
    ADD COLUMN archive_source text GENERATED ALWAYS AS (COALESCE(recovery_reference->>'archive_job',command#>>'{recovery,archive_job}')) STORED;
UPDATE events e SET contest_id=j.recovery_contest,detail=e.detail - 'recovery'
FROM maintenance_jobs j WHERE e.operation='maintenance_result' AND e.target=j.id AND j.recovery_contest IS NOT NULL;
UPDATE events SET detail=jsonb_build_object('request_id',request_id)
WHERE operation='maintenance_commit' AND detail#>'{receipt,preview,targets}' IS NOT NULL;
UPDATE schema_version SET version=21;
