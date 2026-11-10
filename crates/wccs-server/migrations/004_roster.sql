CREATE TABLE roster_head (id integer PRIMARY KEY CHECK (id=1), version bigint NOT NULL);
INSERT INTO roster_head VALUES (1,0);
CREATE TABLE contests (
    id text PRIMARY KEY,
    name text NOT NULL,
    version bigint NOT NULL,
    created_at bigint NOT NULL
);
CREATE TABLE participants (
    id text PRIMARY KEY,
    contest_id text NOT NULL REFERENCES contests(id),
    code text NOT NULL,
    name text NOT NULL,
    school text NOT NULL,
    UNIQUE (contest_id,code) DEFERRABLE INITIALLY DEFERRED,
    UNIQUE (contest_id,id)
);
CREATE TABLE assignments (
    contest_id text NOT NULL,
    participant_id text NOT NULL,
    seat_id text NOT NULL REFERENCES seats(id),
    PRIMARY KEY (contest_id,participant_id),
    FOREIGN KEY (contest_id,participant_id) REFERENCES participants(contest_id,id),
    UNIQUE (contest_id,seat_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE roster_events (
    sequence bigserial PRIMARY KEY,
    contest_id text NOT NULL REFERENCES contests(id),
    request_id text NOT NULL UNIQUE REFERENCES requests(id),
    version bigint NOT NULL,
    result jsonb NOT NULL,
    created_at bigint NOT NULL
);
UPDATE schema_version SET version=4;
