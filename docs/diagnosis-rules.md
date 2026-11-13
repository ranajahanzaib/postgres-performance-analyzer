# Implemented diagnosis rules

- `HIGH_BUFFER_READS` (medium): emits for a query with more than 100 ms captured execution time and over one shared read per returned row. Shared-buffer reads may be cached and this ratio is only a triage signal.
- `REPEATED_EXECUTION_COST` (medium): emits when more than 100 calls contribute over 500 ms. Batching or frequency changes may alter semantics.
- `LOCK_WAIT_OBSERVED` (high for the observed blocker relationship): reports a visible waiting PID and blocker PIDs. A point sample does not establish duration or total impact.

No finding proves an index is needed or that a proposed change will help. A quiet result does not imply a healthy database.
