SELECT id, customer_id, status, total_cents, created_at
FROM orders
WHERE tenant_id = 3
  AND status = 'pending'
  AND DATE(created_at AT TIME ZONE 'UTC') = DATE '2026-03-10'
ORDER BY created_at DESC
LIMIT 50;
