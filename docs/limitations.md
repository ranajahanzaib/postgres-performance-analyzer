# Limitations

`pgpa` provides PostgreSQL capability checks, query-statistics inspection, bounded capture, lock sampling, JSON execution-plan analysis, deterministic HTTP workload runs, artifact comparison, and offline reports. Its findings help guide investigation; they do not establish a cause or prove that a change improved performance.

## Measurement limits

- PostgreSQL cumulative statistics can include unrelated clients and concurrent activity.
- Capture compares matching query identities across two snapshots. New, disappeared, or decreased counters limit the available deltas; maximum execution time is not an interval metric.
- Table and index metadata are ending snapshots rather than before-and-after deltas.
- Lock sampling can miss waits between samples and does not measure exact total wait duration.
- A workload comparison checks recorded configuration and operation metadata, but does not validate HTTP response bodies or prove dataset equivalence.
- One run per configuration is marked inconclusive. The tool does not orchestrate repeated trials or perform statistical significance testing.
- Plan analysis depends on fields returned by PostgreSQL and cannot establish that a proposed index or query rewrite will help under other data and workload conditions.

## Scope limits

The workload runner supports HTTP, not arbitrary SQL workloads. Plans are collected through the explicit `explain` command rather than automatically during HTTP runs. Diagnosis thresholds are built in rather than user-configurable. The project does not provide schema-migration safety checks or a versioned JSON Schema for every artifact.

The disposable demonstrations require Docker and should only be used with their dedicated database. See [safety](safety.md) and [benchmarking](benchmarking.md) for operational and experimental guidance.
