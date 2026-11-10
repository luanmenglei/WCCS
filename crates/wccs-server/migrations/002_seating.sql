CREATE TABLE rooms (
    id text PRIMARY KEY,
    name text NOT NULL,
    rows integer NOT NULL CHECK (rows BETWEEN 1 AND 50),
    columns integer NOT NULL CHECK (columns BETWEEN 1 AND 50),
    start_number integer NOT NULL,
    numbering text NOT NULL,
    origin text NOT NULL,
    version bigint NOT NULL
);
CREATE TABLE seats (
    id text PRIMARY KEY,
    room_id text NOT NULL REFERENCES rooms(id),
    row_index integer NOT NULL,
    column_index integer NOT NULL,
    number integer NOT NULL,
    expected_ip text,
    UNIQUE (room_id, row_index, column_index),
    UNIQUE (room_id, number) DEFERRABLE INITIALLY DEFERRED,
    UNIQUE (expected_ip) DEFERRABLE INITIALLY DEFERRED
);
ALTER TABLE devices ADD COLUMN peer_ip text;
ALTER TABLE devices ADD COLUMN observed_ip text;
ALTER TABLE devices ADD COLUMN address_source text;
ALTER TABLE devices ADD COLUMN reported_ips jsonb NOT NULL DEFAULT '[]';
ALTER TABLE devices ADD COLUMN previous_ip text;
ALTER TABLE devices ADD COLUMN address_version bigint NOT NULL DEFAULT 0;
ALTER TABLE devices ADD COLUMN needs_review boolean NOT NULL DEFAULT false;
ALTER TABLE devices ADD COLUMN excluded boolean NOT NULL DEFAULT false;
UPDATE schema_version SET version=2;
