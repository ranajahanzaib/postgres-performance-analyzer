# Architecture

`pgpa` is a Rust workspace with a command-line analyzer and a small HTTP API used by the disposable demonstration. The CLI coordinates PostgreSQL collection, plan inspection, HTTP workload execution, artifact comparison, and offline report generation. The demo API is a separate workspace member under `demo/api`.

## Data flow

Commands read connection settings from `PGPA_DATABASE_URL` or `DATABASE_URL`. The collector uses PostgreSQL catalog and statistics views to inspect capabilities, query counters, table and index metadata, session state, and locks. `capture` records bounded beginning and ending snapshots and derives query-counter deltas for matching database, user, and query identities. It does not reset PostgreSQL statistics.

`explain` submits explicitly supplied SQL to PostgreSQL using JSON EXPLAIN. Execution profiling is opt-in and guarded by `--analyze` and `--allow-execution`. The plan analyzer traverses the returned plan tree and reports evidence with limitations.

`run` reads a versioned TOML HTTP workload, schedules bounded requests, and writes an HTTP summary and PostgreSQL investigation artifact. `compare` loads those artifacts without requiring a live connection, checks compatibility, and compares request measurements and query counters. `report` renders comparison results as terminal output, JSON, Markdown, or standalone HTML.

## Boundaries and limitations

The CLI currently keeps command and domain logic in `src/main.rs`, with plan-specific analysis in `src/plan.rs`. PostgreSQL access uses `tokio-postgres`; the collector uses a single asynchronous connection for its operations. Artifact formats are implemented with Serde and JSON. The demo API uses Axum and a PostgreSQL connection pool.

The analyzer does not claim causal attribution when other clients share the database. Captures are bounded snapshots, lock observations are sampled, and HTTP request measurements are not PostgreSQL execution-time measurements. See [metrics](metrics.md), [benchmarking](benchmarking.md), and [limitations](limitations.md) for interpretation details.
