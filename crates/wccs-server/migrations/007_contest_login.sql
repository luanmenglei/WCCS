CREATE OR REPLACE FUNCTION project_waiting_event() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='UPDATE' AND NEW.stage=OLD.stage AND NEW.report IS NOT DISTINCT FROM OLD.report THEN RETURN NEW; END IF;
    INSERT INTO events(source,source_id,category,occurred_at,actor_id,actor_name,operation,target,device_ids,room_ids,contest_id,request_id,version,outcome,detail)
    VALUES('waiting',NEW.id||':'||NEW.stage,
      CASE WHEN TG_OP='INSERT' THEN 'management' WHEN NEW.stage IN ('cancelled','expired') THEN 'system' ELSE 'device_report' END,
      NEW.updated_at,CASE WHEN TG_OP='INSERT' THEN NEW.actor_id END,
      CASE WHEN TG_OP='INSERT' THEN (SELECT username FROM operators WHERE id=NEW.actor_id) END,
      CASE WHEN NEW.command->>'action' IN ('prepare_login','restore_teaching') THEN 'login_' ELSE 'waiting_' END||NEW.stage,NEW.id,ARRAY[NEW.device_id],NEW.room_ids,NEW.contest_id,NEW.request_id,NEW.sequence,NEW.stage,
      jsonb_build_object('command',NEW.command,'report',NEW.report)) ON CONFLICT(source,source_id) DO NOTHING;
    RETURN NEW;
END $$;
CREATE OR REPLACE FUNCTION project_control_actual() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE command_row waiting_commands%ROWTYPE;
BEGIN
    IF TG_OP='UPDATE' AND NEW.actual=OLD.actual THEN RETURN NEW; END IF;
    SELECT * INTO command_row FROM waiting_commands WHERE device_id=NEW.device_id ORDER BY sequence DESC LIMIT 1;
    INSERT INTO events(source,source_id,category,occurred_at,operation,target,device_ids,room_ids,contest_id,request_id,version,outcome,detail)
    VALUES('control_actual',NEW.connection_id||':'||nextval('events_id_seq')::text,'device_report',NEW.last_seen,CASE WHEN NEW.actual->'account' IS DISTINCT FROM OLD.actual->'account' THEN 'login_observed' ELSE 'waiting_observed' END,NEW.device_id,
        ARRAY[NEW.device_id],COALESCE(command_row.room_ids,device_room_scope(NEW.device_id)),command_row.contest_id,command_row.request_id,
        command_row.sequence,NEW.actual->>'state',jsonb_build_object('actual',NEW.actual,'connection_id',NEW.connection_id,'instance_id',NEW.instance_id,'generation',NEW.generation,'command_id',command_row.id));
    RETURN NEW;
END $$;
UPDATE schema_version SET version=7;
