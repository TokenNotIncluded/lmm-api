-- Explicit installer-only complement to enable_tool_market. Canonical CREDIT v2
-- must already be installed. Runtime owns no schema, policy seed or code binding.
GRANT SELECT,UPDATE(version,platform_account_id,platform_user_id,enabled,fee_bps,updated_by,updated_at) ON core_tool_market.credit_policy TO :"runtime_role";
GRANT SELECT,INSERT(id,seller_user_id,seller_account_id,platform_account_id,fee_bps,policy_version,snapshot,ceiling_credits),UPDATE(approved,suspended,visibility,approved_by,approved_at) ON core_tool_market.credit_quotes TO :"runtime_role";
GRANT SELECT ON core_tool_market.credit_builtin_bindings,core_tool_market.credit_result_contracts TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(revoked_at,reserved_calls,successful_calls,reserved_credits,spent_credits) ON core_tool_market.credit_consents TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(request,limit_credits,reserved_credits,spent_credits) ON core_tool_market.credit_budgets TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(revoked_at) ON core_tool_market.credit_tokens TO :"runtime_role";
GRANT SELECT,INSERT ON core_tool_market.credit_access_operations,core_tool_market.credit_supplier_receipts,core_tool_market.credit_fixed_witness_receipts TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(state,dispatch_id,result_digest,usage,receipt,settlement_journal_id,started_at,finished_at) ON core_tool_market.credit_calls TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE(active_version_id,paused,deleted,suspended,revision,updated_by,updated_at) ON core_tool_market.credit_services TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_tool_market.lock_builtin_context(TEXT) TO :"runtime_role";
