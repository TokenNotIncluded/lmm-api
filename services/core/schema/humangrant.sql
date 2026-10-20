CREATE SCHEMA IF NOT EXISTS core_humangrant;

CREATE TABLE IF NOT EXISTS core_humangrant.human_grant_records (
    kind VARCHAR(32) NOT NULL,
    id VARCHAR(128) NOT NULL,
    owner VARCHAR(128),
    expires_at BIGINT,
    payload TEXT NOT NULL,
    PRIMARY KEY (kind, id)
);
CREATE INDEX IF NOT EXISTS idx_core_hg_owner ON core_humangrant.human_grant_records(kind, owner);
CREATE INDEX IF NOT EXISTS idx_core_hg_expiry ON core_humangrant.human_grant_records(expires_at);

CREATE TABLE IF NOT EXISTS core_humangrant.human_grant_gate (
    id INT PRIMARY KEY,
    revision BIGINT NOT NULL DEFAULT 1
);
INSERT INTO core_humangrant.human_grant_gate (id, revision) VALUES (1, 1) ON CONFLICT (id) DO NOTHING;

REVOKE ALL ON SCHEMA core_humangrant FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_humangrant FROM PUBLIC;
