SELECT id, customer_id, status, total_cents, created_at
FROM orders
WHERE tenant_id = 3
  AND status = 'pending'
  AND created_at >= (DATE '2026-03-10'::timestamp AT TIME ZONE 'UTC')
  AND created_at < (DATE '2026-03-11'::timestamp AT TIME ZONE 'UTC')
ORDER BY created_at DESC
LIMIT 50;
