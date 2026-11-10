CREATE TABLE operators (
    id text PRIMARY KEY,
    username text NOT NULL,
    login_key text NOT NULL UNIQUE,
    display_name text NOT NULL,
    role text NOT NULL CHECK (role IN ('admin','operator','viewer')),
    enabled boolean NOT NULL,
    all_rooms boolean NOT NULL,
    room_ids text[] NOT NULL,
    version bigint NOT NULL,
    password_hash text NOT NULL,
    created_at bigint NOT NULL,
    CHECK (role <> 'admin' OR (all_rooms AND cardinality(room_ids)=0)),
    CHECK (NOT all_rooms OR cardinality(room_ids)=0)
);
ALTER TABLE requests ADD COLUMN actor_id text REFERENCES operators(id);
ALTER TABLE jobs ADD COLUMN room_ids text[];
ALTER TABLE jobs ADD COLUMN request_id text;
ALTER TABLE device_events ADD COLUMN room_ids text[];

CREATE TABLE events (
    id bigserial PRIMARY KEY,
    source text NOT NULL,
    source_id text NOT NULL,
    category text NOT NULL,
    occurred_at bigint NOT NULL,
    actor_id text,
    actor_name text,
    operation text NOT NULL,
    target text,
    device_ids text[] NOT NULL DEFAULT '{}',
    room_ids text[],
    contest_id text,
    request_id text,
    version bigint,
    outcome text NOT NULL,
    detail jsonb NOT NULL,
    legacy boolean NOT NULL DEFAULT false,
    UNIQUE(source,source_id)
);
CREATE INDEX events_order ON events(occurred_at DESC,id DESC);
CREATE INDEX events_request ON events(request_id);
CREATE INDEX events_rooms ON events USING gin(room_ids);

-- Historical scope and stable actor identity were never recorded. Preserve unknowns.
INSERT INTO events(source,source_id,category,occurred_at,actor_name,operation,request_id,version,outcome,detail,legacy)
SELECT 'audit_events',a.id::text,'management',a.created_at,a.actor,a.operation,a.request_id,
       (r.response->>'version')::bigint,'succeeded',jsonb_build_object('receipt',r.response),true
FROM audit_events a LEFT JOIN requests r ON r.id=a.request_id ORDER BY a.id;
INSERT INTO events(source,source_id,category,occurred_at,operation,target,device_ids,request_id,version,outcome,detail,legacy)
SELECT 'device_events',sequence::text,'historical',created_at,operation,device_id,ARRAY[device_id],request_id,version,'recorded','{}',true
FROM device_events ORDER BY sequence;
INSERT INTO events(source,source_id,category,occurred_at,operation,target,device_ids,version,outcome,detail,legacy)
SELECT 'jobs',id,'task_result',COALESCE(received_at,cancelled_at,0),'tool_check',id,ARRAY[device_id],config_version,
       CASE WHEN cancelled_at IS NOT NULL THEN 'cancelled' WHEN result IS NULL THEN 'unknown'
            WHEN EXISTS(SELECT 1 FROM jsonb_array_elements(result->'tools') t WHERE t->'check'->>'status'='unavailable') THEN 'failed' ELSE 'succeeded' END,
       jsonb_build_object('result',result,'cancellation_reason',cancellation_reason),true FROM jobs ORDER BY sequence;
INSERT INTO events(source,source_id,category,occurred_at,operation,target,contest_id,request_id,version,outcome,detail,legacy)
SELECT 'roster_events',sequence::text,'management',created_at,'roster_result',contest_id,contest_id,request_id,version,'succeeded',result,true
FROM roster_events ORDER BY sequence;

CREATE FUNCTION device_room_scope(device text) RETURNS text[] LANGUAGE sql STABLE AS $$
    SELECT array_agg(DISTINCT s.room_id ORDER BY s.room_id)
    FROM devices d JOIN seats s ON s.expected_ip=d.observed_ip
    WHERE d.id=device AND NOT d.excluded AND NOT d.needs_review
      AND d.credential_hash IS NOT NULL
$$;

CREATE FUNCTION project_audit_event() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE receipt jsonb; payload jsonb;
BEGIN
    SELECT r.response,r.payload INTO receipt,payload FROM requests r WHERE r.id=NEW.request_id;
    INSERT INTO events(source,source_id,category,occurred_at,actor_id,actor_name,operation,target,device_ids,room_ids,contest_id,request_id,version,outcome,detail)
    VALUES('audit_events',NEW.id::text,'management',NEW.created_at,NULLIF(current_setting('wccs.actor_id',true),''),NEW.actor,
        NEW.operation,COALESCE(payload->>'device_id',payload->>'id',payload->>'contest_id',NEW.request_id),
        COALESCE(NULLIF(current_setting('wccs.device_ids',true),'')::text[],'{}'),
        NULLIF(current_setting('wccs.room_ids',true),'')::text[],payload->>'contest_id',NEW.request_id,
        (receipt->>'version')::bigint,'succeeded',jsonb_build_object('receipt',receipt));
    RETURN NEW;
END $$;
CREATE TRIGGER project_audit AFTER INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION project_audit_event();

CREATE FUNCTION project_device_event() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.room_ids := CASE WHEN NEW.operation='expired' THEN device_room_scope(NEW.device_id) ELSE COALESCE(NULLIF(current_setting('wccs.room_ids',true),'')::text[],device_room_scope(NEW.device_id)) END;
    INSERT INTO events(source,source_id,category,occurred_at,actor_id,actor_name,operation,target,device_ids,room_ids,request_id,version,outcome,detail)
    VALUES('device_events',NEW.sequence::text,
        CASE WHEN NEW.operation='expired' THEN 'system' WHEN NULLIF(current_setting('wccs.actor_id',true),'') IS NOT NULL THEN 'management' ELSE 'device_report' END,
        NEW.created_at,CASE WHEN NEW.operation<>'expired' THEN NULLIF(current_setting('wccs.actor_id',true),'') END,CASE WHEN NEW.operation<>'expired' THEN NULLIF(current_setting('wccs.actor_name',true),'') END,
        NEW.operation,NEW.device_id,ARRAY[NEW.device_id],NEW.room_ids,NEW.request_id,NEW.version,'recorded','{}');
    RETURN NEW;
END $$;
CREATE TRIGGER project_device BEFORE INSERT ON device_events FOR EACH ROW EXECUTE FUNCTION project_device_event();

CREATE FUNCTION project_job_event() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE stage text; result_state text; event_time bigint;
BEGIN
    IF TG_OP='INSERT' THEN
        NEW.room_ids := device_room_scope(NEW.device_id);
        NEW.request_id := NULLIF(current_setting('wccs.request_id',true),'');
        stage := 'issued'; result_state := 'pending'; event_time := floor(extract(epoch FROM clock_timestamp()))::bigint;
    ELSIF OLD.result IS NULL AND NEW.result IS NOT NULL THEN
        stage := 'result'; event_time := NEW.received_at;
        result_state := CASE WHEN EXISTS(SELECT 1 FROM jsonb_array_elements(NEW.result->'tools') t WHERE t->'check'->>'status'='unavailable') THEN 'failed' ELSE 'succeeded' END;
    ELSIF OLD.cancelled_at IS NULL AND NEW.cancelled_at IS NOT NULL THEN
        stage := 'cancelled'; result_state := 'cancelled'; event_time := NEW.cancelled_at;
    ELSE RETURN NEW;
    END IF;
    INSERT INTO events(source,source_id,category,occurred_at,actor_id,actor_name,operation,target,device_ids,room_ids,request_id,version,outcome,detail)
    VALUES('jobs_'||stage,NEW.id,CASE WHEN stage='issued' THEN 'task' ELSE 'task_result' END,event_time,
        NULLIF(current_setting('wccs.actor_id',true),''),NULLIF(current_setting('wccs.actor_name',true),''),
        'tool_check_'||stage,NEW.id,ARRAY[NEW.device_id],NEW.room_ids,
        CASE WHEN stage='cancelled' THEN COALESCE(NULLIF(current_setting('wccs.request_id',true),''),NEW.request_id) ELSE NEW.request_id END,
        NEW.config_version,result_state,
        jsonb_build_object('result',NEW.result,'cancellation_reason',NEW.cancellation_reason,'expires_at',NEW.expires_at,'issued_request_id',NEW.request_id));
    RETURN NEW;
END $$;
CREATE TRIGGER project_job BEFORE INSERT OR UPDATE ON jobs FOR EACH ROW EXECUTE FUNCTION project_job_event();

CREATE FUNCTION project_roster_event() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO events(source,source_id,category,occurred_at,actor_id,actor_name,operation,target,contest_id,request_id,version,outcome,detail)
    VALUES('roster_events',NEW.sequence::text,'management',NEW.created_at,NULLIF(current_setting('wccs.actor_id',true),''),
        NULLIF(current_setting('wccs.actor_name',true),''),'roster_result',NEW.contest_id,NEW.contest_id,NEW.request_id,NEW.version,'succeeded',NEW.result);
    RETURN NEW;
END $$;
CREATE TRIGGER project_roster AFTER INSERT ON roster_events FOR EACH ROW EXECUTE FUNCTION project_roster_event();
UPDATE schema_version SET version=5;
