CREATE TABLE event_actions (
    event_id bigint PRIMARY KEY REFERENCES events(id),
    version bigint NOT NULL,
    status text NOT NULL CHECK(status IN ('open','acknowledged','resolved')),
    note text NOT NULL,
    actor_id text NOT NULL REFERENCES operators(id),
    updated_at bigint NOT NULL
);
UPDATE schema_version SET version=17;
