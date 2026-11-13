DO $$
DECLARE differences bigint;
BEGIN
  WITH baseline AS (
    SELECT id, customer_id, status, total_cents, created_at
    FROM orders
    WHERE tenant_id = 3 AND status = 'pending'
      AND DATE(created_at AT TIME ZONE 'UTC') = DATE '2026-03-10'
    ORDER BY created_at DESC
  ), optimized AS (
    SELECT id, customer_id, status, total_cents, created_at
    FROM orders
    WHERE tenant_id = 3 AND status = 'pending'
      AND created_at >= (DATE '2026-03-10'::timestamp AT TIME ZONE 'UTC')
      AND created_at < (DATE '2026-03-11'::timestamp AT TIME ZONE 'UTC')
    ORDER BY created_at DESC
  )
  SELECT count(*) INTO differences FROM (
    (SELECT * FROM baseline EXCEPT ALL SELECT * FROM optimized)
    UNION ALL
    (SELECT * FROM optimized EXCEPT ALL SELECT * FROM baseline)
  ) AS changed_rows;
  IF differences <> 0 THEN
    RAISE EXCEPTION 'baseline and rewritten query returned different row multisets (% rows differ)', differences;
  END IF;
  RAISE NOTICE 'baseline and rewritten query returned identical row multisets';
END $$;
