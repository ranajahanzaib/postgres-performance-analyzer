# Troubleshooting

## `pg_stat_statements` unavailable

Run `pgpa doctor`. Ask the database owner to preload the extension library and create the extension in this database. `pgpa` never changes server settings or installs extensions. Use the PostgreSQL-version-specific installation procedure and restart policy for your server.

## Statistics do not change

Check that `pg_stat_statements.track` includes the statements of interest and that the workload uses this database. Cumulative statistics become visible after statement completion. A capture records matching counter deltas and does not reset global statistics. New/disappeared query entries or decreased counters appear under `capture_issues`.

## Query text or activity is hidden

PostgreSQL restricts other roles' statistics and query text. `pg_read_all_stats` provides broader visibility; grant it deliberately. Without that access, pgpa can still report statistics visible to the current role.

## I/O or planning numbers are missing

`track_io_timing` and `track_planning` are optional settings and may be disabled or unsupported. `doctor` reports their observed status. Missing metrics remain unavailable rather than being shown as zero.

## Capture includes unrelated queries or sees no locks

`pg_stat_statements` aggregates statements database-wide. An observation window may include other clients. Lock sampling observes relationships only at sample times; waits between samples can be missed. Use an isolated demo database/workload for attribution and choose a conservative sampling interval.

## EXPLAIN ANALYZE is refused

It executes the SQL. Supply both `--analyze` and `--allow-execution`; non-SELECT-looking SQL also requires `--allow-non-select`. Review the file first. SELECT functions can have side effects, and this CLI does not sandbox SQL.

## Workload returns errors or drops requests

Check the API health endpoint, manifest paths, database URL, and workload permissions. Run summaries group failed responses by safe error category, report dropped requests at the in-flight cap, and include scheduling delay. Comparisons with overload, high error rate, or too few samples are marked not comparable.

## Performance varies between runs

This version compares individual run artifacts and does not orchestrate repeated trials. Repeat the same manifest and environment, preserve each artifact, and compare trial-level distributions manually. Cache state, concurrent traffic, and server settings affect measurements.

## Candidate index does not improve the demo

Check the captured plan, data distribution, index validity, and buffers. PostgreSQL may prefer a sequential scan for small or broad scans. The demo's measurements are machine-dependent; the analyzer must report inconclusive or regressed outcomes honestly.
