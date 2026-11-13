CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_orders_tenant_status_created
ON orders (tenant_id, status, created_at DESC);
ANALYZE orders;
