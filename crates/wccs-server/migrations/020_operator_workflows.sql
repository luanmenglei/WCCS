ALTER TABLE contests ADD COLUMN remote_contest_id text UNIQUE;
CREATE TABLE pause_seats (
    remote_contest_id text NOT NULL,
    seat_id text NOT NULL,
    PRIMARY KEY(remote_contest_id,seat_id)
);
WITH last_action AS (
    SELECT DISTINCT ON (remote,seat) remote,seat,payload->>'action' AS action
    FROM contest_control_jobs
    CROSS JOIN LATERAL (SELECT payload->>'remote_contest_id' AS remote) binding
    CROSS JOIN LATERAL jsonb_array_elements_text(payload->'seat_ids') seats(seat)
    WHERE (payload->>'action'='pause' AND stage IN ('running','succeeded','failed','interrupted'))
       OR (payload->>'action' IN ('start','resume','end') AND stage='succeeded')
    ORDER BY remote,seat,updated_at DESC,(payload->>'action'='pause') DESC,id
)
INSERT INTO pause_seats SELECT remote,seat FROM last_action WHERE action='pause';
UPDATE schema_version SET version=20;
