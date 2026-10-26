
WITH catalog_path AS MATERIALIZED (
 SELECT pg_catalog.set_config('search_path','pg_catalog',true)
), ns AS MATERIALIZED (
 SELECT n.oid,n.nspname FROM pg_namespace n CROSS JOIN catalog_path
 WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
), definitions AS (
 SELECT json_build_array('relation',n.nspname,c.relname,c.relkind)::text AS definition
 FROM pg_class c JOIN ns n ON n.oid=c.relnamespace
 UNION ALL
 SELECT json_build_array('column',n.nspname,c.relname,a.attnum,a.attname,
   format_type(a.atttypid,a.atttypmod),a.attnotnull,a.attidentity,a.attgenerated,
   pg_get_expr(d.adbin,d.adrelid))::text
 FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN ns n ON n.oid=c.relnamespace
 LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
 WHERE a.attnum>0 AND NOT a.attisdropped
 UNION ALL
 SELECT json_build_array('constraint',n.nspname,c.conname,pg_get_constraintdef(c.oid,true))::text
 FROM pg_constraint c JOIN ns n ON n.oid=c.connamespace
 UNION ALL
 SELECT json_build_array('index',n.nspname,c.relname,pg_get_indexdef(i.indexrelid))::text
 FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid JOIN ns n ON n.oid=c.relnamespace
 UNION ALL
 SELECT json_build_array('function',n.nspname,p.proname,pg_get_functiondef(p.oid))::text
 FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace WHERE p.prokind IN ('f','p')
 UNION ALL
 SELECT json_build_array('trigger',n.nspname,c.relname,t.tgname,t.tgenabled,
   pg_get_triggerdef(t.oid,true))::text
 FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN ns n ON n.oid=c.relnamespace
 WHERE NOT t.tgisinternal
 UNION ALL
 -- RI trigger names embed installation OIDs, which change on pg_dump/restore.
 -- Hash their stable constraint/function/trigger properties instead.
 SELECT json_build_array('internal-trigger',n.nspname,c.relname,k.conname,
   pn.nspname,p.proname,t.tgtype,t.tgenabled,t.tgdeferrable,t.tginitdeferred,
   t.tgattr::text,encode(t.tgargs,'hex'),pg_get_expr(t.tgqual,t.tgrelid))::text
 FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN ns n ON n.oid=c.relnamespace
 JOIN pg_proc p ON p.oid=t.tgfoid JOIN pg_namespace pn ON pn.oid=p.pronamespace
 LEFT JOIN pg_constraint k ON k.oid=t.tgconstraint WHERE t.tgisinternal
) SELECT definition FROM definitions ORDER BY definition COLLATE "C"
