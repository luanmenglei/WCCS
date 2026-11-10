CREATE TABLE control_devices (
    device_id text PRIMARY KEY REFERENCES devices(id),
    connection_id text NOT NULL,
    server_epoch text NOT NULL,
    instance_id text NOT NULL,
    session_key text,
    generation bigint NOT NULL,
    last_seen bigint NOT NULL,
    connected boolean NOT NULL,
    actual jsonb NOT NULL
);
CREATE TABLE waiting_commands (
    sequence bigserial PRIMARY KEY,
    id text NOT NULL UNIQUE,
    request_id text NOT NULL,
    device_id text NOT NULL REFERENCES devices(id),
    actor_id text NOT NULL REFERENCES operators(id),
    actor_version bigint NOT NULL,
    room_ids text[] NOT NULL,
    contest_id text NOT NULL,
    generation bigint NOT NULL,
    instance_id text NOT NULL,
    session_key text NOT NULL,
    expires_at bigint NOT NULL,
    command jsonb NOT NULL,
    stage text NOT NULL DEFAULT 'pending',
    report jsonb,
    updated_at bigint NOT NULL
);
CREATE INDEX waiting_device ON waiting_commands(device_id,sequence DESC);
CREATE INDEX waiting_request ON waiting_commands(request_id);
CREATE FUNCTION project_waiting_event() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='UPDATE' AND NEW.stage=OLD.stage AND NEW.report IS NOT DISTINCT FROM OLD.report THEN RETURN NEW; END IF;
    INSERT INTO events(source,source_id,category,occurred_at,actor_id,actor_name,operation,target,device_ids,room_ids,contest_id,request_id,version,outcome,detail)
    VALUES('waiting',NEW.id||':'||NEW.stage,
      CASE WHEN TG_OP='INSERT' THEN 'management' WHEN NEW.stage IN ('cancelled','expired') THEN 'system' ELSE 'device_report' END,
      NEW.updated_at,CASE WHEN TG_OP='INSERT' THEN NEW.actor_id END,
      CASE WHEN TG_OP='INSERT' THEN (SELECT username FROM operators WHERE id=NEW.actor_id) END,
      'waiting_'||NEW.stage,NEW.id,ARRAY[NEW.device_id],NEW.room_ids,NEW.contest_id,NEW.request_id,NEW.sequence,NEW.stage,
      jsonb_build_object('command',NEW.command,'report',NEW.report)) ON CONFLICT(source,source_id) DO NOTHING;
    RETURN NEW;
END $$;
CREATE TRIGGER project_waiting AFTER INSERT OR UPDATE ON waiting_commands FOR EACH ROW EXECUTE FUNCTION project_waiting_event();
CREATE FUNCTION project_control_actual() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE command_row waiting_commands%ROWTYPE;
BEGIN
    IF TG_OP='UPDATE' AND NEW.actual=OLD.actual THEN RETURN NEW; END IF;
    SELECT * INTO command_row FROM waiting_commands WHERE device_id=NEW.device_id ORDER BY sequence DESC LIMIT 1;
    INSERT INTO events(source,source_id,category,occurred_at,operation,target,device_ids,room_ids,contest_id,request_id,version,outcome,detail)
    VALUES('control_actual',NEW.connection_id||':'||nextval('events_id_seq')::text,'device_report',NEW.last_seen,'waiting_observed',NEW.device_id,
        ARRAY[NEW.device_id],COALESCE(command_row.room_ids,device_room_scope(NEW.device_id)),command_row.contest_id,command_row.request_id,
        command_row.sequence,NEW.actual->>'state',jsonb_build_object('actual',NEW.actual,'connection_id',NEW.connection_id,'instance_id',NEW.instance_id,'generation',NEW.generation,'command_id',command_row.id));
    RETURN NEW;
END $$;
CREATE TRIGGER project_control_actual AFTER INSERT OR UPDATE ON control_devices FOR EACH ROW EXECUTE FUNCTION project_control_actual();
UPDATE schema_version SET version=6;
