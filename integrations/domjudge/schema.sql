CREATE TABLE IF NOT EXISTS wccs_contest_control (
    cid int unsigned NOT NULL PRIMARY KEY,
    revision bigint NOT NULL DEFAULT 0,
    phase varchar(20) NOT NULL,
    paused_at decimal(32,9) DEFAULT NULL,
    interval_id int unsigned DEFAULT NULL,
    checkpoint longtext NOT NULL
);
CREATE TABLE IF NOT EXISTS wccs_control_requests (
    request_id varchar(80) NOT NULL PRIMARY KEY,
    fingerprint char(64) NOT NULL,
    response longtext NOT NULL
);
