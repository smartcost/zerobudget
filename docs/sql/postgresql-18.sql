-- Diagnostic only; no settings, data or privileges are changed.
SELECT name, setting, unit, source, pending_restart
FROM pg_settings
WHERE name IN ('server_version', 'io_method', 'io_workers',
               'effective_io_concurrency', 'maintenance_io_concurrency',
               'io_max_concurrency')
ORDER BY name;

SELECT n.nspname AS schema_name, c.relname AS table_name,
       pg_get_expr(d.adbin, d.adrelid) AS id_default,
       EXISTS (SELECT 1 FROM pg_trigger t WHERE t.tgrelid=c.oid
               AND t.tgname='require_uuid_v7' AND t.tgenabled='O') AS validates_uuid_v7
FROM pg_class c
JOIN pg_namespace n ON n.oid=c.relnamespace
JOIN pg_attribute a ON a.attrelid=c.oid AND a.attname='id' AND NOT a.attisdropped
LEFT JOIN pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum
WHERE c.relkind='r' AND n.nspname IN ('auth','budgeting','banking')
ORDER BY n.nspname,c.relname;

SELECT n.nspname AS schema_name,c.relname AS table_name,a.attname,
       CASE a.attgenerated WHEN 's' THEN 'stored' WHEN 'v' THEN 'virtual' END AS generation
FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid
JOIN pg_namespace n ON n.oid=c.relnamespace
WHERE a.attgenerated<>'' AND NOT a.attisdropped
AND n.nspname IN ('auth','budgeting','banking')
ORDER BY n.nspname,c.relname,a.attnum;
