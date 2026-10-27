-- Existing-site upgrade for the three public console content keys.
-- Run as the installation owner after the Core schema exists. This script is
-- also included by grant-core-runtime.sql when console support is enabled.
\set ON_ERROR_STOP on
\if :{?runtime_role}
\else
\set runtime_role lmm_core_runtime
\endif

BEGIN;
ALTER TABLE core_console.options ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS runtime_options_read ON core_console.options;
DROP POLICY IF EXISTS runtime_options_insert_public_content ON core_console.options;
DROP POLICY IF EXISTS runtime_options_update_public_content ON core_console.options;
CREATE POLICY runtime_options_read ON core_console.options
    FOR SELECT TO :"runtime_role" USING (true);
CREATE POLICY runtime_options_insert_public_content ON core_console.options
    FOR INSERT TO :"runtime_role"
    WITH CHECK (key IN ('Notice', 'Announcements', 'console_setting.announcements'));
CREATE POLICY runtime_options_update_public_content ON core_console.options
    FOR UPDATE TO :"runtime_role"
    USING (key IN ('Notice', 'Announcements', 'console_setting.announcements'))
    WITH CHECK (key IN ('Notice', 'Announcements', 'console_setting.announcements'));
GRANT SELECT, INSERT, UPDATE ON core_console.options TO :"runtime_role";
COMMIT;
