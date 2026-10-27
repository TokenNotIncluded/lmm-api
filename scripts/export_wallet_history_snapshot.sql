-- Run only against an OFFLINE restore of the reviewed legacy dump.
-- psql --no-psqlrc -XAt --set ON_ERROR_STOP=1 --file this.sql > topups.json
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT jsonb_build_object(
 'source_namespace','lmm_api.lmm_prod_20260802.top_ups',
 'observed_at',floor(extract(epoch FROM transaction_timestamp()))::bigint,
 'source_row_count',(SELECT count(*) FROM lmm_prod_20260802.top_ups),
 'source_user_ids',(SELECT coalesce(jsonb_agg(id ORDER BY id),'[]'::jsonb) FROM lmm_prod_20260802.users),
 'rows',(SELECT coalesce(jsonb_agg(jsonb_build_object(
  'id',id,'user_id',user_id,'amount',amount,'money_text',money::text,
  'trade_no',trade_no,'payment_method',payment_method,'status',status,
  'create_time',create_time,'complete_time',complete_time,
  'credited_quota',credited_quota,'platform_amount_micros',platform_amount_micros,
  'settlement_currency',settlement_currency
 ) ORDER BY id),'[]'::jsonb) FROM lmm_prod_20260802.top_ups)
);
ROLLBACK;
