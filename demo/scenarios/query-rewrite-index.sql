DROP INDEX CONCURRENTLY IF EXISTS idx_orders_tenant_status_created;
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_orders_tenant_created
ON orders (tenant_id, created_at DESC);
ANALYZE orders;
