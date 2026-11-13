SELECT id, customer_id, status, total_cents, created_at
FROM orders
WHERE tenant_id = 3
  AND status = 'pending'
  AND created_at >= TIMESTAMPTZ '2026-03-10 00:00:00+00'
  AND created_at < TIMESTAMPTZ '2026-03-11 00:00:00+00'
ORDER BY created_at DESC
LIMIT 50;
