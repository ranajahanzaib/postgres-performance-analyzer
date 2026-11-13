# Workloads and comparisons

`pgpa run --workload FILE --label LABEL --output DIR` parses a versioned TOML HTTP workload, warms up at the configured rate, captures a PostgreSQL counter snapshot, schedules the measured arrivals at fixed intervals, waits for launched requests to finish, optionally cools down, then captures a second PostgreSQL snapshot. Each run stores `summary.json` (HTTP metrics and workload/environment identifiers) and `investigation.json` (query counter deltas). A SHA-256 digest identifies the exact manifest without copying URLs or paths that may contain sensitive data. The saved target origin excludes path, credentials, query, and fragment; credentials/query/fragments are rejected in the configured base URL.

The manifest runner allows GET and HEAD. POST requires `--allow-writes`, and the endpoint must be safe for the explicit configured workload. It substitutes deterministic values for `{tenant_id}`, `{order_id}`, `{customer_id}`, `{status}`, `{date}`, and `{next_date}` from the fixed seed and request sequence. Values use the demo data's assumed key ranges; adapt them for another service. A 10-second request timeout, 512 maximum in-flight requests, 5,000 requests/second, and 100,000 scheduled requests bound the runner. If the in-flight cap is reached, the scheduled request is counted as dropped rather than queued silently.

Latency is measured with a monotonic clock and summarized from individual completed request durations; API latency is separate from PostgreSQL execution time. Error rate is failures divided by completed requests; dropped requests and achieved throughput are reported separately. Percentiles are nearest-rank sample quantiles. The single-run summary is not a trial distribution.

`pgpa compare` checks run settings and logical operation IDs/weights, compares HTTP distributions and PostgreSQL counters, and marks a one-trial outcome inconclusive. Matching a workload manifest does not prove application behavior or dataset equivalence; changed manifest hashes require review of route/query semantics. Repeat each configuration and preserve trial artifacts before making performance claims. The tool does not automate repeated trials, validate response bodies, fingerprint datasets, or use scheduling drift as a validity verdict.

Example:

```sh
PGPA_DATABASE_URL='postgresql://pgpa:pgpa@localhost:55432/pgpa_demo' \
  cargo run -- run --workload demo/workloads/orders-search.toml --label baseline --output runs/baseline
```
