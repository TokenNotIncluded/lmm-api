-- Fresh installation only. Extension role receives no Core schema privileges.
CREATE SCHEMA IF NOT EXISTS lmm_bounties;
CREATE TABLE lmm_bounties.contract(singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),version integer NOT NULL CHECK(version=1));
INSERT INTO lmm_bounties.contract VALUES(true,1);
CREATE TABLE lmm_bounties.records(bucket text NOT NULL CHECK(bucket IN ('projects','challenges','disputes','operations','notifications','ledger','web_operations')),id text NOT NULL CHECK(length(id) BETWEEN 1 AND 128),body jsonb NOT NULL CHECK(jsonb_typeof(body)='object'),PRIMARY KEY(bucket,id));
CREATE INDEX bounty_records_project ON lmm_bounties.records(bucket,(body->>'project_id'),id);
CREATE INDEX bounty_records_owner ON lmm_bounties.records(bucket,(body->>'owner_user_id'),id);
CREATE INDEX bounty_records_participant ON lmm_bounties.records(bucket,(body->>'participant_user_id'),id);
CREATE INDEX bounty_records_publication ON lmm_bounties.records ((COALESCE((body->>'published_at')::bigint,0)) DESC,id DESC) WHERE bucket='projects' AND body->>'status' IN ('published','paused');
CREATE INDEX bounty_records_notifications ON lmm_bounties.records((body->>'participant_user_id'),(COALESCE((body->>'created_at')::bigint,0)) DESC,id DESC) WHERE bucket='notifications';
