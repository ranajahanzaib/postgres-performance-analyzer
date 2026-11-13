COMPOSE = docker compose -f demo/compose.yaml
DB_URL ?= postgresql://pgpa:pgpa@localhost:55432/pgpa_demo

.PHONY: demo-up demo-down demo-doctor demo-baseline demo-index demo-optimized demo-compare demo-query-baseline demo-query-optimized demo-lock demo-lock-optimized demo-report demo-plan demo-plan-analyze demo-query-verify

demo-up:
	$(COMPOSE) up -d --build --wait

demo-down:
	$(COMPOSE) down -v

demo-doctor:
	PGPA_DATABASE_URL='$(DB_URL)' cargo run -- doctor

demo-baseline:
	$(COMPOSE) exec -T postgres psql -v ON_ERROR_STOP=1 -U pgpa_admin -d pgpa_demo < demo/scenarios/reset-order-indexes.sql
	PGPA_DATABASE_URL='$(DB_URL)' cargo run -- run --workload demo/workloads/orders-search.toml --label baseline --output runs/missing-index/baseline

demo-index:
	$(COMPOSE) exec -T postgres psql -v ON_ERROR_STOP=1 -U pgpa_admin -d pgpa_demo < demo/scenarios/missing-index.sql

demo-optimized:
	$(COMPOSE) -f demo/compose.missing-index-optimized.yaml up -d --build api
	PGPA_DATABASE_URL='$(DB_URL)' cargo run -- run --workload demo/workloads/orders-search-optimized.toml --label indexed --output runs/missing-index/optimized

demo-compare:
	cargo run -- compare --before runs/missing-index/baseline --after runs/missing-index/optimized

demo-query-baseline:
	$(COMPOSE) -f demo/compose.query-rewrite-baseline.yaml up -d --build api
	$(COMPOSE) exec -T postgres psql -v ON_ERROR_STOP=1 -U pgpa_admin -d pgpa_demo < demo/scenarios/query-rewrite-index.sql
	$(COMPOSE) exec -T postgres psql -v ON_ERROR_STOP=1 -U pgpa_admin -d pgpa_demo < demo/scenarios/verify-query-rewrite.sql
	PGPA_DATABASE_URL='$(DB_URL)' cargo run -- run --workload demo/workloads/query-rewrite-baseline.toml --label expression-predicate --output runs/query-rewrite/baseline

demo-query-optimized:
	$(COMPOSE) -f demo/compose.query-rewrite-optimized.yaml up -d --build api
	PGPA_DATABASE_URL='$(DB_URL)' cargo run -- run --workload demo/workloads/query-rewrite-optimized.toml --label range-predicate --output runs/query-rewrite/optimized

demo-lock:
	$(COMPOSE) -f demo/compose.lock-baseline.yaml up -d --build api
	cargo build --release
	@set -e; PGPA_DATABASE_URL='$(DB_URL)' target/release/pgpa locks --duration 10s --interval 200ms --output runs/lock-contention/locks.json & lock_pid=$$!; \
	PGPA_DATABASE_URL='$(DB_URL)' target/release/pgpa run --workload demo/workloads/lock-contention.toml --label long-transaction --allow-writes --output runs/lock-contention/baseline; \
	wait $$lock_pid
	$(COMPOSE) exec -T postgres psql -v ON_ERROR_STOP=1 -U pgpa_admin -d pgpa_demo < demo/scenarios/verify-lock-state.sql

# The HTML report is standalone and can be opened without a service.
demo-report:
	cargo run -- report --before runs/missing-index/baseline --after runs/missing-index/optimized --format html --output reports/missing-index.html

demo-lock-optimized:
	$(COMPOSE) -f demo/compose.lock-optimized.yaml up -d --build api
	PGPA_DATABASE_URL='$(DB_URL)' cargo run -- run --workload demo/workloads/lock-contention-optimized.toml --label short-transaction --allow-writes --output runs/lock-contention/optimized
	$(COMPOSE) exec -T postgres psql -v ON_ERROR_STOP=1 -U pgpa_admin -d pgpa_demo < demo/scenarios/verify-lock-state.sql
	PGPA_DATABASE_URL='$(DB_URL)' cargo run -- compare --before runs/lock-contention/baseline --after runs/lock-contention/optimized

demo-plan:
	PGPA_DATABASE_URL='postgresql://pgpa:pgpa@localhost:55432/pgpa_demo' cargo run -- explain --sql-file demo/queries/orders-search.sql --output runs/demo-plan.json

demo-plan-analyze:
	PGPA_DATABASE_URL='postgresql://pgpa:pgpa@localhost:55432/pgpa_demo' cargo run -- explain --sql-file demo/queries/orders-search.sql --analyze --allow-execution --output runs/demo-plan-analyze.json

demo-query-verify:
	$(COMPOSE) exec -T postgres psql -v ON_ERROR_STOP=1 -U pgpa_admin -d pgpa_demo < demo/scenarios/verify-query-rewrite.sql
