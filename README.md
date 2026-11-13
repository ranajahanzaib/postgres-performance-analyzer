# PostgreSQL Performance Analyzer

Find expensive PostgreSQL queries, inspect execution plans and lock waits, and measure whether a workload change helped.

`pgpa` combines PostgreSQL's existing statistics with repeatable HTTP workloads and offline before/after artifacts. Findings are investigation leads; test proposed changes with representative data and repeated trials before drawing a performance conclusion.

## What it does

- Checks PostgreSQL capabilities and permissions (`doctor`)
- Ranks real `pg_stat_statements` data (`inspect`)
- Captures bounded query-counter deltas, table/index metadata, settings, and counter-integrity issues (`capture`)
- Samples lock waiters, blockers, `pg_locks`, and session metadata at a bounded interval (`locks`)
- Runs nonexecuting JSON EXPLAIN by default and recursively analyzes actual JSON plans (`explain`)
- Executes deterministic, fixed-rate HTTP workloads from TOML manifests (`run`)
- Compares request latency and PostgreSQL counters, checks workload/environment compatibility, and stays inconclusive for single trials (`compare`)
- Writes offline terminal, JSON, Markdown, or standalone HTML reports (`report`)

The workload engine bounds request rate, in-flight requests, duration, and total scheduled requests. POST workloads require `--allow-writes`. Captures and run summaries omit SQL unless explicitly requested; connection credentials are not written to artifacts.

## Quick start

Requirements: stable Rust and PostgreSQL 16–18. Query statistics need `pg_stat_statements`; `track_io_timing` and `track_planning` improve optional evidence.

```sh
cargo install --path .
export PGPA_DATABASE_URL='postgresql://localhost/mydb'
pgpa doctor                    # exit 0 = full, 2 = partial, 1 = connection/command failure
pgpa inspect --top 10 --sort total-time
pgpa capture --duration 30s --output runs/observe
pgpa diagnose --capture runs/observe
```

The database URL comes from `PGPA_DATABASE_URL` or `DATABASE_URL`. Keep credentials in a protected environment/configuration source. `doctor` does not install extensions or change settings. `pg_read_all_stats` may be needed to see other roles' query text/activity.

## Commands

```text
pgpa doctor
pgpa inspect [--top N] [--sort total-time|mean-time|max-time|calls|shared-reads|temp-blocks|rows]
            [--query-id ID] [--format table|json] [--include-sql] [--output FILE]
pgpa capture --duration 30s --output DIR [--include-sql]
pgpa locks --duration 15s --interval 250ms [--include-sql] [--output FILE]
pgpa explain --sql-file query.sql [--output FILE]
pgpa explain --sql-file query.sql --analyze --allow-execution [--allow-non-select]
pgpa diagnose --capture DIR [--plan normalized-plan.json]
pgpa run --workload FILE.toml --label LABEL --output DIR [--allow-writes]
pgpa compare --before DIR --after DIR
pgpa report --before DIR --after DIR --format terminal|json|markdown|html --output FILE
```

`inspect` total time is cumulative PostgreSQL execution time, not wall-clock time. Capture compares matching `(dbid, userid, queryid)` counters. New query entries and decreased/disappeared counters are recorded as capture limitations; window maximum latency is unavailable from cumulative statistics and is left null. Tables and indexes are ending snapshots. Lock observations are samples, not an exact wait history.

EXPLAIN without `--analyze` plans without executing the statement. EXPLAIN ANALYZE executes it and needs `--allow-execution`; statements that are not plainly SELECT-looking need the additional `--allow-non-select`. This conservative prefix check is not a SQL safety boundary. SELECT can call side-effecting functions. Review the SQL and target before enabling execution.

The HTTP workload manifest format and measurement limits are documented in [benchmarking.md](docs/benchmarking.md). A run writes `summary.json` for HTTP measurements and environment fingerprints plus `investigation.json` for PostgreSQL counter deltas. `compare` marks one-trial results inconclusive and checks basic compatibility; it does not establish statistical significance or SQL result equivalence.

## Disposable local demonstration

The Compose demo binds PostgreSQL and the API to loopback, creates a dedicated non-superuser app/collector role, seeds deterministic multi-tenant data, and uses `pg_stat_statements`. It is isolated from arbitrary external databases.

```sh
make demo-up
make demo-doctor
make demo-baseline
make demo-plan-analyze       # explicitly executes the supplied demo SELECT
make demo-index
make demo-optimized
make demo-compare
make demo-report
```

Query rewrite and lock scenarios:

```sh
make demo-query-baseline     # runs a row-multiset equivalence check
make demo-query-optimized
cargo run -- compare --before runs/query-rewrite/baseline --after runs/query-rewrite/optimized
make demo-lock               # samples a deliberately long row-lock transaction
make demo-lock-optimized     # shortens the transaction and verifies final row state
```

The scenarios use real PostgreSQL and HTTP requests. `CREATE INDEX CONCURRENTLY` is confined to the disposable Compose database and is run outside a transaction. `make demo-down` removes the database volume and its data.

## Development and CI

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
cargo run -- --help
```

CI builds on Linux and macOS and runs a real PostgreSQL integration test against PostgreSQL 16, 17, and 18. The local integration test skips unless `PGPA_TEST_DATABASE_URL` points to a dedicated disposable test database; it creates an extension and a temporary named lock-test table.

## Limitations

The workload runner supports HTTP workloads, not arbitrary SQL workloads. Repeated-trial orchestration, automatic plan collection during HTTP runs, configurable diagnosis thresholds, schema-migration safety checks, and statistical significance tests are outside the current implementation. Docker-based end-to-end demo results have not been verified in this checkout. See [limitations](docs/limitations.md), [safety](docs/safety.md), and [troubleshooting](docs/troubleshooting.md).

This project is MIT licensed. Recommendations should be tested against representative workloads; an observed plan or index change alone is not proof of improvement.
