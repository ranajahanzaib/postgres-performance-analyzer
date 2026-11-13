# Metrics

`pg_stat_statements.total_exec_time`, `mean_exec_time`, and `max_exec_time` are milliseconds accumulated for normalized statements, not API request latency. The module does not expose request-level samples, so percentiles cannot be derived from these values. `calls` counts executions.

Shared block hits and reads count PostgreSQL shared-buffer events; a read does not necessarily imply physical storage I/O. Temporary blocks indicate temporary-file activity in blocks. Row and table tuple counts are estimates. Planner costs are relative planner units, never milliseconds.

Capture computes counter deltas across a bounded interval, matching database/user/query IDs. New, disappeared, or decreased counters are recorded as capture issues and excluded where a reliable delta cannot be computed. Decreased counters may indicate a reset or restart. Lock blockers are snapshots, not a complete timeline. Index scans observed over a short period do not establish whether an index is unused.

For a bounded capture, `max_exec_time` is null: the source value is a cumulative maximum with no interval delta. New or disappeared statement entries and any decreased counters are recorded in `capture_issues` and excluded from per-window deltas rather than silently treated as zero.
