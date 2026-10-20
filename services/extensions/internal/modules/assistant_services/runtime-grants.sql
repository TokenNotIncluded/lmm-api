-- Explicit installer operation only. Set psql variable runtime_role to the
-- dedicated assistant runtime login. No privileges in any Core database.
GRANT USAGE ON SCHEMA lmm_assistant TO :"runtime_role";
GRANT SELECT ON lmm_assistant.services_contract TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE,DELETE ON lmm_assistant.memories,lmm_assistant.profiles TO :"runtime_role";
GRANT SELECT,INSERT ON lmm_assistant.service_operations,lmm_assistant.service_network_dispatch,lmm_assistant.service_confirmations,lmm_assistant.site_issue_events,lmm_assistant.platform_support_messages TO :"runtime_role";
GRANT UPDATE(consumed_at,result) ON lmm_assistant.service_confirmations TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE ON lmm_assistant.site_issues,lmm_assistant.platform_support,lmm_assistant.platform_support_todos TO :"runtime_role";
GRANT USAGE ON SEQUENCE lmm_assistant.memories_id_seq,lmm_assistant.site_issues_id_seq,lmm_assistant.site_issue_events_id_seq,lmm_assistant.platform_support_id_seq,lmm_assistant.platform_support_messages_id_seq TO :"runtime_role";
