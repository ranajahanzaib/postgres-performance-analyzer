use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};
use tokio_postgres::Client;
mod plan;

#[derive(Parser)]
#[command(
    name = "pgpa",
    version,
    about = "Find expensive PostgreSQL work and measure whether a change helped"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Doctor,
    Inspect {
        #[arg(long, default_value_t = 20)]
        top: usize,
        #[arg(long, value_enum, default_value = "total-time")]
        sort: Sort,
        #[arg(long)]
        query_id: Option<i64>,
        #[arg(long)]
        include_sql: bool,
        #[arg(long, value_enum, default_value = "table")]
        format: Format,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Capture {
        #[arg(long, default_value = "30s")]
        duration: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        include_sql: bool,
    },
    Locks {
        #[arg(long, default_value = "15s")]
        duration: String,
        #[arg(long, default_value = "250ms")]
        interval: String,
        #[arg(long)]
        include_sql: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Run {
        #[arg(long)]
        workload: PathBuf,
        #[arg(long)]
        label: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        allow_writes: bool,
    },
    Explain {
        #[arg(long)]
        sql_file: PathBuf,
        #[arg(long)]
        analyze: bool,
        #[arg(long)]
        allow_execution: bool,
        #[arg(long)]
        allow_non_select: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Diagnose {
        #[arg(long)]
        capture: PathBuf,
        #[arg(long)]
        plan: Option<PathBuf>,
    },
    Compare {
        #[arg(long)]
        before: PathBuf,
        #[arg(long)]
        after: PathBuf,
    },
    Report {
        #[arg(long)]
        before: PathBuf,
        #[arg(long)]
        after: PathBuf,
        #[arg(long, value_enum, default_value = "markdown")]
        format: ReportFormat,
        #[arg(long)]
        output: PathBuf,
    },
}
#[derive(Clone, Copy, ValueEnum)]
enum Sort {
    TotalTime,
    MeanTime,
    MaxTime,
    Calls,
    SharedReads,
    TempBlocks,
    Rows,
}
#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Format {
    Table,
    Json,
}
#[derive(Clone, Copy, ValueEnum)]
enum ReportFormat {
    Terminal,
    Markdown,
    Html,
    Json,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Snapshot {
    schema_version: u32,
    id: String,
    captured_at: DateTime<Utc>,
    duration_seconds: f64,
    server_version: String,
    database: String,
    role: String,
    #[serde(default)]
    settings: BTreeMap<String, String>,
    statements_available: bool,
    include_sql: bool,
    queries: Vec<QueryStat>,
    tables: Vec<TableStat>,
    indexes: Vec<IndexStat>,
    locks: Vec<LockStat>,
    #[serde(default)]
    capture_issues: Vec<DeltaIssue>,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
struct DeltaIssue {
    query_id: Option<i64>,
    reason: String,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
struct QueryStat {
    dbid: i64,
    userid: i64,
    queryid: i64,
    toplevel: bool,
    calls: i64,
    total_exec_time: f64,
    plans: i64,
    total_plan_time: f64,
    mean_exec_time: f64,
    max_exec_time: Option<f64>,
    rows: i64,
    shared_blks_hit: i64,
    shared_blks_read: i64,
    shared_blks_dirtied: i64,
    shared_blks_written: i64,
    temp_blks_read: i64,
    temp_blks_written: i64,
    blk_read_time: f64,
    blk_write_time: f64,
    wal_bytes: f64,
    query: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
struct TableStat {
    schema: String,
    name: String,
    live_tuples: f64,
    seq_scan: i64,
    idx_scan: i64,
    size_bytes: i64,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
struct IndexStat {
    schema: String,
    table: String,
    name: String,
    definition: String,
    valid: bool,
    scans: i64,
    size_bytes: i64,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
struct LockStat {
    pid: i32,
    #[serde(default)]
    wait_event_type: Option<String>,
    wait_event: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    query_start: Option<DateTime<Utc>>,
    #[serde(default)]
    transaction_start: Option<DateTime<Utc>>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    application_name: Option<String>,
    #[serde(default)]
    query_id: Option<i64>,
    #[serde(default)]
    lock_details: Vec<serde_json::Value>,
    #[serde(default)]
    blocker_sessions: Vec<serde_json::Value>,
    query: Option<String>,
    blockers: Vec<i32>,
}
#[derive(Debug, Serialize, Deserialize)]
struct LockSample {
    sampled_at: DateTime<Utc>,
    waiters: Vec<LockStat>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkloadManifest {
    workload: WorkloadConfig,
    target: TargetConfig,
    operations: Vec<OperationConfig>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkloadConfig {
    name: String,
    version: u32,
    duration_seconds: u32,
    #[serde(default)]
    warmup_seconds: u32,
    #[serde(default)]
    cooldown_seconds: u32,
    target_rps: u32,
    max_in_flight: usize,
    seed: u64,
    #[serde(default)]
    application_version: Option<String>,
    #[serde(default)]
    query_variant: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetConfig {
    #[serde(rename = "type")]
    target_type: String,
    base_url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationConfig {
    id: String,
    method: String,
    path: String,
    weight: u32,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
struct RunArtifact {
    schema_version: u32,
    id: String,
    label: String,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    workload_name: String,
    workload_version: u32,
    application_version: Option<String>,
    query_variant: Option<String>,
    workload_hash_sha256: String,
    target_origin: String,
    database: String,
    postgres_version: String,
    dataset_fingerprint_approx_sha256: String,
    database_settings: BTreeMap<String, String>,
    index_definitions: Vec<String>,
    duration_seconds: u32,
    warmup_seconds: u32,
    cooldown_seconds: u32,
    target_rps: u32,
    max_in_flight: usize,
    seed: u64,
    operations: Vec<OperationDescriptor>,
    metrics: RequestMetrics,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct OperationDescriptor {
    id: String,
    method: String,
    weight: u32,
}
#[derive(Debug, Serialize, Deserialize)]
struct RequestMetrics {
    scheduled: u64,
    started: u64,
    completed: u64,
    successful: u64,
    failed: u64,
    dropped_at_concurrency_cap: u64,
    completion_wall_time_seconds: f64,
    throughput_requests_per_second: f64,
    error_rate: Option<f64>,
    failure_counts: BTreeMap<String, u64>,
    latency_ms: LatencySummary,
    scheduling_delay_ms: LatencySummary,
    by_operation: BTreeMap<String, OperationMetrics>,
}
#[derive(Debug, Serialize, Deserialize)]
struct OperationMetrics {
    scheduled: u64,
    completed: u64,
    failed: u64,
    dropped_at_concurrency_cap: u64,
    latency_ms: LatencySummary,
}
#[derive(Debug, Default, Serialize, Deserialize)]
struct LatencySummary {
    samples: usize,
    min: Option<f64>,
    mean: Option<f64>,
    median: Option<f64>,
    p90: Option<f64>,
    p95: Option<f64>,
    p99: Option<f64>,
    max: Option<f64>,
    standard_deviation: Option<f64>,
}
#[derive(Debug)]
struct RequestResult {
    operation: String,
    latency_ms: f64,
    scheduling_delay_ms: f64,
    successful: bool,
    failure_kind: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match execute(cli).await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
async fn execute(cli: Cli) -> Result<u8> {
    let code = match cli.command {
        Command::Doctor => {
            if doctor().await? {
                2
            } else {
                0
            }
        }
        Command::Inspect {
            top,
            sort,
            query_id,
            include_sql,
            format,
            output,
        } => {
            inspect(top, sort, query_id, include_sql, format, output).await?;
            0
        }
        Command::Capture {
            duration,
            output,
            include_sql,
        } => {
            capture(&duration, output, include_sql).await?;
            0
        }
        Command::Locks {
            duration,
            interval,
            include_sql,
            output,
        } => {
            locks(&duration, &interval, include_sql, output).await?;
            0
        }
        Command::Run {
            workload,
            label,
            output,
            allow_writes,
        } => {
            run_workload(workload, label, output, allow_writes).await?;
            0
        }
        Command::Explain {
            sql_file,
            analyze,
            allow_execution,
            allow_non_select,
            output,
        } => {
            explain(sql_file, analyze, allow_execution, allow_non_select, output).await?;
            0
        }
        Command::Diagnose { capture, plan } => {
            diagnose(capture, plan)?;
            0
        }
        Command::Compare { before, after } => {
            compare(before, after)?;
            0
        }
        Command::Report {
            before,
            after,
            format,
            output,
        } => {
            report(before, after, format, output)?;
            0
        }
    };
    Ok(code)
}

async fn connect() -> Result<Client> {
    let url = std::env::var("PGPA_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .context(
            "Set PGPA_DATABASE_URL to connect (credentials should come from the environment)",
        )?;
    let mut config: tokio_postgres::Config = url
        .parse()
        .context("PGPA_DATABASE_URL is not a valid PostgreSQL URL")?;
    config.application_name("pgpa");
    prepare_db_tls(&mut config)?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let tls = if config.get_ssl_mode() == tokio_postgres::config::SslMode::Disable {
        tokio_postgres_rustls::MakeRustlsConnect::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth(),
        )
    } else {
        let (tls, root_errors) = tokio_postgres_rustls::MakeRustlsConnect::with_native_certs()
            .map_err(|errors| {
                anyhow::anyhow!(
                    "no platform TLS root certificates could be loaded: {}",
                    errors.len()
                )
            })?;
        if !root_errors.is_empty() {
            eprintln!(
                "Warning: {} platform TLS root certificate error(s) were reported; verified TLS handshakes may fail.",
                root_errors.len()
            );
        }
        tls
    };
    let (c, conn) = config
        .connect(tls)
        .await
        .context("PostgreSQL connection failed")?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("database connection closed: {e}");
        }
    });
    Ok(c)
}
fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}
fn prepare_db_tls(config: &mut tokio_postgres::Config) -> Result<bool> {
    let remote = config.get_hosts().iter().any(|host| match host {
        tokio_postgres::config::Host::Tcp(host) => !is_loopback_host(host),
        #[cfg(unix)]
        tokio_postgres::config::Host::Unix(_) => false,
    });
    if remote && config.get_ssl_mode() == tokio_postgres::config::SslMode::Disable {
        bail!(
            "plaintext PostgreSQL connections to non-loopback hosts are refused; use TLS (sslmode=require) or a local encrypted tunnel"
        );
    }
    if remote {
        config.ssl_mode(tokio_postgres::config::SslMode::Require);
    } else if config.get_ssl_mode() == tokio_postgres::config::SslMode::Prefer {
        config.ssl_mode(tokio_postgres::config::SslMode::Disable);
    }
    Ok(remote)
}
async fn doctor() -> Result<bool> {
    let c = connect().await?;
    let row = c
        .query_one(
            "SELECT current_setting('server_version'), current_database(), current_user",
            &[],
        )
        .await?;
    let ext = c
        .query_opt(
            "SELECT extversion FROM pg_extension WHERE extname='pg_stat_statements'",
            &[],
        )
        .await?;
    println!(
        "PostgreSQL Performance Analyzer\nConnection: OK\nServer: PostgreSQL {}\nDatabase: {}\nRole: {}\npg_stat_statements: {}",
        row.get::<_, String>(0),
        row.get::<_, String>(1),
        row.get::<_, String>(2),
        if ext.is_some() {
            "installed"
        } else {
            "unavailable (query statistics disabled)"
        }
    );
    let mut partial = ext.is_none();
    if ext.is_none() {
        println!(
            "\nEnable shared_preload_libraries and install pg_stat_statements using your normal DBA process; pgpa does not change server configuration."
        );
    } else {
        match c
            .query_one("SELECT count(*)::bigint FROM pg_stat_statements", &[])
            .await
        {
            Ok(_) => println!("Query statistics: available (query IDs available)"),
            Err(e) => {
                partial = true;
                println!("Query statistics: installed but inaccessible ({e})");
            }
        }
    }
    let settings = c
        .query_one(
            "SELECT current_setting('track_io_timing'), current_setting('track_planning', true)",
            &[],
        )
        .await?;
    let io_timing: String = settings.get(0);
    let planning: Option<String> = settings.get(1);
    if io_timing != "on" || planning.as_deref() != Some("on") {
        partial = true;
    }
    println!(
        "Execution plans: available when the role has SELECT privileges on referenced objects\nLock inspection: available for visible sessions\nI/O timing: {}\nPlanning statistics: {}",
        io_timing,
        planning.as_deref().unwrap_or("unsupported")
    );
    if partial {
        println!("\nDiagnostics: partial capability set (exit code 2)");
    }
    Ok(partial)
}
async fn query_stats(c: &Client, include_sql: bool) -> Result<Vec<QueryStat>> {
    let ext = c
        .query_opt(
            "SELECT 1 FROM pg_extension WHERE extname='pg_stat_statements'",
            &[],
        )
        .await?;
    if ext.is_none() {
        return Ok(vec![]);
    }
    let sql = "SELECT dbid::bigint, userid::bigint, queryid::bigint, toplevel, calls::bigint, total_exec_time::float8, mean_exec_time::float8, max_exec_time::float8, rows::bigint, shared_blks_hit::bigint, shared_blks_read::bigint, temp_blks_read::bigint, temp_blks_written::bigint, query, plans::bigint, total_plan_time::float8, shared_blks_dirtied::bigint, shared_blks_written::bigint, blk_read_time::float8, blk_write_time::float8, wal_bytes::float8 FROM pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database())";
    let rows = c
        .query(sql, &[])
        .await
        .context("Cannot read pg_stat_statements; check extension and role permissions")?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let text: Option<String> = r.get(13);
            let normalized = text.as_deref().unwrap_or("").to_ascii_lowercase();
            if normalized.contains("from pg_stat_statements") && normalized.contains("where dbid") {
                return None;
            }
            Some(QueryStat {
                dbid: r.get(0),
                userid: r.get(1),
                queryid: r.get(2),
                toplevel: r.get(3),
                calls: r.get(4),
                total_exec_time: r.get(5),
                plans: r.get(14),
                total_plan_time: r.get(15),
                mean_exec_time: r.get(6),
                max_exec_time: Some(r.get(7)),
                rows: r.get(8),
                shared_blks_hit: r.get(9),
                shared_blks_read: r.get(10),
                shared_blks_dirtied: r.get(16),
                shared_blks_written: r.get(17),
                temp_blks_read: r.get(11),
                temp_blks_written: r.get(12),
                blk_read_time: r.get(18),
                blk_write_time: r.get(19),
                wal_bytes: r.get(20),
                query: if include_sql { text } else { None },
            })
        })
        .collect())
}
fn sort_queries(q: &mut [QueryStat], s: Sort) {
    q.sort_by(|a, b| {
        let (x, y) = match s {
            Sort::TotalTime => (a.total_exec_time, b.total_exec_time),
            Sort::MeanTime => (a.mean_exec_time, b.mean_exec_time),
            Sort::MaxTime => (
                a.max_exec_time.unwrap_or(0.0),
                b.max_exec_time.unwrap_or(0.0),
            ),
            Sort::Calls => (a.calls as f64, b.calls as f64),
            Sort::SharedReads => (a.shared_blks_read as f64, b.shared_blks_read as f64),
            Sort::TempBlocks => (
                (a.temp_blks_read + a.temp_blks_written) as f64,
                (b.temp_blks_read + b.temp_blks_written) as f64,
            ),
            Sort::Rows => (a.rows as f64, b.rows as f64),
        };
        y.total_cmp(&x)
    })
}
async fn inspect(
    top: usize,
    sort: Sort,
    query_id: Option<i64>,
    include_sql: bool,
    format: Format,
    output: Option<PathBuf>,
) -> Result<()> {
    let c = connect().await?;
    let mut q = query_stats(&c, include_sql).await?;
    if let Some(id) = query_id {
        q.retain(|x| x.queryid == id)
    }
    sort_queries(&mut q, sort);
    q.truncate(top);
    let out = if format == Format::Json {
        serde_json::to_string_pretty(&q)?
    } else {
        let mut s = String::from(
            "QUERY ID | CALLS | TOTAL ms | MEAN ms | MAX ms | ROWS/CALL | SHARED READS | TEMP BLOCKS\n",
        );
        for x in q {
            s += &format!(
                "{} | {} | {:.2} | {:.3} | {} | {:.1} | {} | {}\n",
                x.queryid,
                x.calls,
                x.total_exec_time,
                x.mean_exec_time,
                x.max_exec_time
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "unavailable".into()),
                if x.calls > 0 {
                    x.rows as f64 / x.calls as f64
                } else {
                    0.0
                },
                x.shared_blks_read,
                x.temp_blks_read + x.temp_blks_written
            );
            if include_sql && let Some(sql) = x.query {
                s += &format!("  {sql}\n")
            }
        }
        s += "Total time is cumulative execution time recorded by pg_stat_statements, not wall-clock time. Workload impact denominator is query execution time recorded for this database.";
        s
    };
    write_or_print(out, output)
}
fn write_or_print(s: String, p: Option<PathBuf>) -> Result<()> {
    if let Some(p) = p {
        fs::write(&p, s).with_context(|| format!("writing {}", p.display()))?
    } else {
        println!("{s}")
    }
    Ok(())
}
const LOCK_SAMPLE_SQL: &str = "SELECT a.pid, a.wait_event_type, a.wait_event, a.state, a.query_start, a.xact_start, a.usename, a.application_name, a.query, a.query_id::bigint, pg_blocking_pids(a.pid), COALESCE((SELECT jsonb_agg(jsonb_build_object('locktype',l.locktype,'mode',l.mode,'granted',l.granted,'relation',CASE WHEN l.relation IS NULL THEN NULL ELSE l.relation::regclass::text END,'transactionid',l.transactionid::text,'virtualxid',l.virtualxid)) FROM pg_locks l WHERE l.pid=a.pid),'[]'::jsonb), COALESCE((SELECT jsonb_agg(jsonb_build_object('pid',b.pid,'state',b.state,'query_start',b.query_start,'xact_start',b.xact_start,'role',b.usename,'application_name',b.application_name,'wait_event_type',b.wait_event_type,'wait_event',b.wait_event,'query_id',b.query_id)) FROM pg_stat_activity b WHERE b.pid=ANY(pg_blocking_pids(a.pid))),'[]'::jsonb) FROM pg_stat_activity a WHERE a.datname=current_database() AND a.application_name IS DISTINCT FROM 'pgpa' AND cardinality(pg_blocking_pids(a.pid)) > 0 ORDER BY a.pid";

fn lock_stat(row: &tokio_postgres::Row, include_sql: bool) -> LockStat {
    let mut blockers: Vec<i32> = row.get(10);
    blockers.sort_unstable();
    blockers.dedup();
    LockStat {
        pid: row.get(0),
        wait_event_type: row.get(1),
        wait_event: row.get(2),
        state: row.get(3),
        query_start: row.get(4),
        transaction_start: row.get(5),
        role: row.get(6),
        application_name: row.get(7),
        query_id: row.get(9),
        lock_details: row.get(11),
        blocker_sessions: row.get(12),
        query: if include_sql { row.get(8) } else { None },
        blockers,
    }
}

async fn locks(
    duration: &str,
    interval: &str,
    include_sql: bool,
    output: Option<PathBuf>,
) -> Result<()> {
    let duration_secs = parse_duration(duration)?;
    let interval_secs = parse_duration(interval)?;
    if !(0.1..=5.0).contains(&interval_secs) {
        bail!("lock sample interval must be between 100ms and 5s to limit monitoring overhead")
    }
    if duration_secs > 300.0 {
        bail!("lock sampling is limited to 300 seconds per invocation")
    }
    let client = connect().await?;
    let start = tokio::time::Instant::now();
    let deadline = start + Duration::from_secs_f64(duration_secs);
    let mut samples = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let rows = client.query(LOCK_SAMPLE_SQL, &[]).await.context("sampling pg_stat_activity and blocker relationships; check activity visibility permissions")?;
        let waiters = rows.iter().map(|row| lock_stat(row, include_sql)).collect();
        samples.push(LockSample {
            sampled_at: Utc::now(),
            waiters,
        });
        let now = tokio::time::Instant::now();
        if now < deadline {
            tokio::time::sleep(Duration::from_secs_f64(interval_secs).min(deadline - now)).await;
        }
    }
    let encoded = serde_json::to_string_pretty(&samples)?;
    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, encoded.as_bytes())?;
        println!(
            "{} lock samples written to {}",
            samples.len(),
            path.display()
        );
    } else {
        let waiter_count: usize = samples.iter().map(|sample| sample.waiters.len()).sum();
        println!(
            "LOCK SAMPLE SUMMARY\nSamples: {}\nObserved waiter samples: {}",
            samples.len(),
            waiter_count
        );
        for sample in &samples {
            for waiter in &sample.waiters {
                let blockers = waiter
                    .blockers
                    .iter()
                    .map(|pid| {
                        if *pid == 0 {
                            "0 (prepared transaction/unknown)".to_string()
                        } else {
                            pid.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "{} waiting PID {} queryid={:?} {:?}/{:?} state={:?} blockers=[{}] transaction_start={:?} role={:?} app={:?}",
                    sample.sampled_at,
                    waiter.pid,
                    waiter.query_id,
                    waiter.wait_event_type,
                    waiter.wait_event,
                    waiter.state,
                    blockers,
                    waiter.transaction_start,
                    waiter.role,
                    waiter.application_name
                );
                if include_sql {
                    println!("  query: {}", waiter.query.as_deref().unwrap_or("<hidden>"));
                }
            }
        }
        if waiter_count == 0 {
            println!(
                "No lock blocking relationship was observed. Sampling cannot prove no contention occurred between samples."
            );
        }
    }
    Ok(())
}
#[derive(Debug, Default)]
struct PhaseRun {
    scheduled: u64,
    started: u64,
    dropped: u64,
    completion_wall_time_seconds: f64,
    scheduled_by_operation: BTreeMap<String, u64>,
    results: Vec<RequestResult>,
}

fn validate_manifest(m: &WorkloadManifest) -> Result<()> {
    if m.workload.version != 1 {
        bail!(
            "unsupported workload manifest version {}; supported version is 1",
            m.workload.version
        );
    }
    if m.workload.name.trim().is_empty() {
        bail!("workload.name must not be empty");
    }
    if m.target.target_type != "http" {
        bail!("only target.type = 'http' is currently supported");
    }
    let base = reqwest::Url::parse(&m.target.base_url)
        .context("target.base_url must be an absolute HTTP(S) URL")?;
    if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
        bail!("target.base_url must use http or https and include a host");
    }
    if !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        bail!("target.base_url must not contain credentials, query parameters, or a fragment");
    }
    if m.workload.duration_seconds == 0 || m.workload.duration_seconds > 3600 {
        bail!("duration_seconds must be between 1 and 3600");
    }
    if m.workload.warmup_seconds > 300 || m.workload.cooldown_seconds > 60 {
        bail!("warmup_seconds is capped at 300 and cooldown_seconds at 60");
    }
    if m.workload.target_rps == 0 || m.workload.target_rps > 5000 {
        bail!("target_rps must be between 1 and 5000");
    }
    if m.workload.max_in_flight == 0 || m.workload.max_in_flight > 512 {
        bail!("max_in_flight must be between 1 and 512");
    }
    let planned = (m.workload.duration_seconds as u64 + m.workload.warmup_seconds as u64)
        * m.workload.target_rps as u64;
    if planned > 100_000 {
        bail!("workload exceeds the 100,000 scheduled request safety cap");
    }
    if m.operations.is_empty() {
        bail!("at least one [[operations]] entry is required");
    }
    if m.operations.len() > 100 {
        bail!("at most 100 operations are allowed in one manifest");
    }
    let mut ids = std::collections::HashSet::new();
    for op in &m.operations {
        if op.id.trim().is_empty() || !ids.insert(&op.id) {
            bail!("operation IDs must be non-empty and unique");
        }
        if !op.path.starts_with('/') || op.path.starts_with("//") {
            bail!(
                "operation {} path must be a local absolute path beginning with one slash",
                op.id
            );
        }
        if op.path.contains('{')
            && ![
                "{tenant_id}",
                "{order_id}",
                "{customer_id}",
                "{status}",
                "{date}",
                "{next_date}",
            ]
            .iter()
            .any(|token| op.path.contains(token))
        {
            bail!(
                "operation {} contains an unsupported path placeholder",
                op.id
            );
        }
        let expanded = expand_values(&op.path, m.workload.seed);
        if expanded.contains('{') || expanded.contains('}') {
            bail!(
                "operation {} contains an unsupported path placeholder",
                op.id
            );
        }
        if op.weight == 0 {
            bail!("operation {} weight must be positive", op.id);
        }
        match op.method.as_str() {
            "GET" | "HEAD" => {}
            "POST" if op.body.is_some() => {}
            "POST" => bail!(
                "POST operation {} requires a body value (use an empty string for an empty body)",
                op.id
            ),
            other => bail!(
                "operation {} uses unsupported HTTP method {}; supported methods are GET, HEAD, POST",
                op.id,
                other
            ),
        }
    }
    Ok(())
}

async fn run_workload(
    path: PathBuf,
    label: String,
    output: PathBuf,
    allow_writes: bool,
) -> Result<()> {
    if label.trim().is_empty() {
        bail!("--label must not be empty");
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("reading workload manifest {}", path.display()))?;
    let manifest: WorkloadManifest =
        toml::from_str(&raw).context("invalid TOML workload manifest")?;
    validate_manifest(&manifest)?;
    if !allow_writes && manifest.operations.iter().any(|op| op.method == "POST") {
        bail!(
            "workload contains POST operations. Pass --allow-writes only for a target where these writes are intended."
        );
    }
    let db = connect()
        .await
        .context("workload execution requires PGPA_DATABASE_URL to capture PostgreSQL evidence")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("pgpa/", env!("CARGO_PKG_VERSION")))
        .build()?;
    if manifest.workload.query_variant.is_some() || manifest.workload.application_version.is_some()
    {
        let health_url = format!("{}/health", manifest.target.base_url.trim_end_matches('/'));
        let health: serde_json::Value = client.get(health_url).send().await.context("checking configured application health endpoint")?.error_for_status().context("application health endpoint returned an error")?.json().await.context("application health response must be JSON when manifest fingerprints are configured")?;
        if let Some(expected) = &manifest.workload.query_variant {
            let actual = health
                .get("query_variant")
                .and_then(serde_json::Value::as_str)
                .context("application health response lacks query_variant")?;
            if actual != expected {
                bail!(
                    "workload declares query_variant={expected}, but application reports {actual}; refusing to measure a mismatched scenario"
                );
            }
        }
        if let Some(expected) = &manifest.workload.application_version {
            let actual = health
                .get("application_version")
                .and_then(serde_json::Value::as_str)
                .context("application health response lacks application_version")?;
            if actual != expected {
                bail!(
                    "workload declares application_version={expected}, but application reports {actual}"
                );
            }
        }
    }
    if manifest.workload.warmup_seconds > 0 {
        eprintln!(
            "Warmup: {} seconds at target rate",
            manifest.workload.warmup_seconds
        );
        let _ = execute_phase(
            &client,
            &manifest,
            manifest.workload.warmup_seconds,
            manifest.workload.seed,
            manifest.workload.max_in_flight,
        )
        .await?;
    }
    let before = collect(&db, 0.0, false).await?;
    let started_at = Utc::now();
    let measured = execute_phase(
        &client,
        &manifest,
        manifest.workload.duration_seconds,
        manifest.workload.seed,
        manifest.workload.max_in_flight,
    )
    .await?;
    if manifest.workload.cooldown_seconds > 0 {
        tokio::time::sleep(Duration::from_secs(
            manifest.workload.cooldown_seconds as u64,
        ))
        .await;
    }
    let after = collect(&db, manifest.workload.duration_seconds as f64, false).await?;
    let pg_delta = delta_snapshots(before, after)?;
    let mut dataset = pg_delta
        .tables
        .iter()
        .map(|t| format!("{}.{}:{:.0}", t.schema, t.name, t.live_tuples))
        .collect::<Vec<_>>();
    dataset.sort();
    let mut dataset_hasher = Sha256::new();
    dataset_hasher.update(dataset.join("\n"));
    let dataset_fingerprint = format!("{:x}", dataset_hasher.finalize());
    let mut index_definitions = pg_delta
        .indexes
        .iter()
        .filter(|i| i.valid)
        .map(|i| i.definition.clone())
        .collect::<Vec<_>>();
    index_definitions.sort();
    let metrics = summarize_phase(&manifest, measured);
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let workload_hash = format!("{:x}", hasher.finalize());
    let parsed_url = reqwest::Url::parse(&manifest.target.base_url)?;
    let target_origin = parsed_url.origin().ascii_serialization();
    let artifact = RunArtifact {
        schema_version: 1,
        id: uuid::Uuid::new_v4().to_string(),
        label,
        started_at,
        finished_at: Utc::now(),
        workload_name: manifest.workload.name.clone(),
        workload_version: manifest.workload.version,
        application_version: manifest.workload.application_version.clone(),
        query_variant: manifest.workload.query_variant.clone(),
        workload_hash_sha256: workload_hash,
        target_origin,
        database: pg_delta.database.clone(),
        postgres_version: pg_delta.server_version.clone(),
        dataset_fingerprint_approx_sha256: dataset_fingerprint,
        database_settings: pg_delta.settings.clone(),
        index_definitions,
        duration_seconds: manifest.workload.duration_seconds,
        warmup_seconds: manifest.workload.warmup_seconds,
        cooldown_seconds: manifest.workload.cooldown_seconds,
        target_rps: manifest.workload.target_rps,
        max_in_flight: manifest.workload.max_in_flight,
        seed: manifest.workload.seed,
        operations: manifest
            .operations
            .iter()
            .map(|op| OperationDescriptor {
                id: op.id.clone(),
                method: op.method.clone(),
                weight: op.weight,
            })
            .collect(),
        metrics,
    };
    fs::create_dir_all(&output)?;
    atomic_write(
        &output.join("summary.json"),
        &serde_json::to_vec_pretty(&artifact)?,
    )?;
    atomic_write(
        &output.join("investigation.json"),
        &serde_json::to_vec_pretty(&pg_delta)?,
    )?;
    println!(
        "Run {} saved to {}\nRequests: scheduled={} completed={} failed={} dropped={} throughput={:.2} req/s\np95 latency: {} ms; PostgreSQL query deltas: {}",
        artifact.label,
        output.display(),
        artifact.metrics.scheduled,
        artifact.metrics.completed,
        artifact.metrics.failed,
        artifact.metrics.dropped_at_concurrency_cap,
        artifact.metrics.throughput_requests_per_second,
        artifact
            .metrics
            .latency_ms
            .p95
            .map(|x| format!("{x:.2}"))
            .unwrap_or_else(|| "unavailable".into()),
        pg_delta.queries.len()
    );
    Ok(())
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    fs::write(&temp, data)?;
    fs::rename(&temp, path).with_context(|| format!("atomically writing {}", path.display()))?;
    Ok(())
}

async fn execute_phase(
    client: &reqwest::Client,
    manifest: &WorkloadManifest,
    seconds: u32,
    seed: u64,
    max_in_flight: usize,
) -> Result<PhaseRun> {
    let scheduled = seconds as u64 * manifest.workload.target_rps as u64;
    let period = 1.0 / manifest.workload.target_rps as f64;
    let phase_start = tokio::time::Instant::now();
    let wall_start = phase_start;
    let mut tasks = tokio::task::JoinSet::new();
    let mut phase = PhaseRun::default();
    for index in 0..scheduled {
        let operation = choose_operation(&manifest.operations, seed.wrapping_add(index))
            .context("validated workload has no selectable operation")?;
        *phase
            .scheduled_by_operation
            .entry(operation.id.clone())
            .or_default() += 1;
        phase.scheduled += 1;
        let due = phase_start + Duration::from_secs_f64(period * index as f64);
        tokio::time::sleep_until(due).await;
        while let Some(joined) = tasks.try_join_next() {
            phase.results.push(joined.context("request task failed")?);
        }
        if tasks.len() >= max_in_flight {
            phase.dropped += 1;
            continue;
        }
        let request_start = tokio::time::Instant::now();
        let scheduling_delay_ms =
            request_start.saturating_duration_since(due).as_secs_f64() * 1000.0;
        let url = expand_request_path(
            &manifest.target.base_url,
            &operation.path,
            seed.wrapping_add(index),
        );
        let method = reqwest::Method::from_bytes(operation.method.as_bytes())
            .context("invalid HTTP method")?;
        let body = operation
            .body
            .clone()
            .map(|body| expand_values(&body, seed.wrapping_add(index)));
        let content_type = operation.content_type.clone();
        let http = client.clone();
        let operation_id = operation.id.clone();
        tasks.spawn(async move {
            let started = tokio::time::Instant::now();
            let mut request = http.request(method, url);
            if let Some(body) = body {
                request = request.body(body);
            }
            if let Some(content_type) = content_type {
                request = request.header(reqwest::header::CONTENT_TYPE, content_type);
            }
            let failure_kind = match request.send().await {
                Ok(response) if response.status().is_success() => None,
                Ok(response) => Some(format!("http_status_{}", response.status().as_u16())),
                Err(error) if error.is_timeout() => Some("timeout".into()),
                Err(error) if error.is_connect() => Some("connection".into()),
                Err(_) => Some("request_error".into()),
            };
            RequestResult {
                operation: operation_id,
                latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                scheduling_delay_ms,
                successful: failure_kind.is_none(),
                failure_kind,
            }
        });
        phase.started += 1;
    }
    while let Some(joined) = tasks.join_next().await {
        phase.results.push(joined.context("request task failed")?);
    }
    phase.completion_wall_time_seconds = wall_start.elapsed().as_secs_f64();
    Ok(phase)
}

fn choose_operation(operations: &[OperationConfig], mut state: u64) -> Option<&OperationConfig> {
    // Small deterministic xorshift PRNG keeps identical manifests and seeds repeatable.
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    let total: u64 = operations.iter().map(|op| op.weight as u64).sum();
    if total == 0 {
        return operations.first();
    }
    let mut pick = state % total;
    for op in operations {
        if pick < op.weight as u64 {
            return Some(op);
        }
        pick -= op.weight as u64;
    }
    operations.last()
}

fn expand_values(input: &str, value: u64) -> String {
    let statuses = ["pending", "complete", "cancelled"];
    input
        .replace("{tenant_id}", &((value % 10) + 1).to_string())
        .replace("{order_id}", &((value % 250_000) + 1).to_string())
        .replace("{customer_id}", &((value % 25_000) + 1).to_string())
        .replace("{status}", statuses[(value as usize) % statuses.len()])
        .replace("{date}", &format!("2026-01-{:02}", (value % 28) + 1))
        .replace("{next_date}", &format!("2026-01-{:02}", ((value % 28) + 2)))
}
fn expand_request_path(base: &str, path: &str, value: u64) -> String {
    format!(
        "{}{}",
        base.trim_end_matches('/'),
        expand_values(path, value)
    )
}

fn latency_summary(values: &[f64]) -> LatencySummary {
    if values.is_empty() {
        return LatencySummary::default();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let sum: f64 = sorted.iter().sum();
    let mean = sum / sorted.len() as f64;
    let variance = if sorted.len() > 1 {
        sorted.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (sorted.len() - 1) as f64
    } else {
        0.0
    };
    let quantile = |p: f64| {
        sorted[((p * sorted.len() as f64).ceil() as usize)
            .saturating_sub(1)
            .min(sorted.len() - 1)]
    };
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    } else {
        quantile(0.5)
    };
    LatencySummary {
        samples: sorted.len(),
        min: Some(sorted[0]),
        mean: Some(mean),
        median: Some(median),
        p90: Some(quantile(0.90)),
        p95: Some(quantile(0.95)),
        p99: Some(quantile(0.99)),
        max: sorted.last().copied(),
        standard_deviation: Some(variance.sqrt()),
    }
}

fn summarize_phase(manifest: &WorkloadManifest, phase: PhaseRun) -> RequestMetrics {
    let completed = phase.results.len() as u64;
    let failed = phase.results.iter().filter(|r| !r.successful).count() as u64;
    let latencies: Vec<_> = phase.results.iter().map(|r| r.latency_ms).collect();
    let delays: Vec<_> = phase
        .results
        .iter()
        .map(|r| r.scheduling_delay_ms)
        .collect();
    let mut failure_counts = BTreeMap::new();
    for result in &phase.results {
        if let Some(kind) = &result.failure_kind {
            *failure_counts.entry(kind.clone()).or_insert(0) += 1;
        }
    }
    let mut by_operation = BTreeMap::new();
    for op in &manifest.operations {
        let results: Vec<_> = phase
            .results
            .iter()
            .filter(|r| r.operation == op.id)
            .collect();
        let values: Vec<_> = results.iter().map(|r| r.latency_ms).collect();
        by_operation.insert(
            op.id.clone(),
            OperationMetrics {
                scheduled: *phase.scheduled_by_operation.get(&op.id).unwrap_or(&0),
                completed: results.len() as u64,
                failed: results.iter().filter(|r| !r.successful).count() as u64,
                dropped_at_concurrency_cap: phase
                    .scheduled_by_operation
                    .get(&op.id)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(results.len() as u64),
                latency_ms: latency_summary(&values),
            },
        );
    }
    RequestMetrics {
        scheduled: phase.scheduled,
        started: phase.started,
        completed,
        successful: completed - failed,
        failed,
        dropped_at_concurrency_cap: phase.dropped,
        completion_wall_time_seconds: phase.completion_wall_time_seconds,
        throughput_requests_per_second: completed as f64
            / phase.completion_wall_time_seconds.max(f64::EPSILON),
        error_rate: (completed > 0).then_some(failed as f64 / completed as f64),
        failure_counts,
        latency_ms: latency_summary(&latencies),
        scheduling_delay_ms: latency_summary(&delays),
        by_operation,
    }
}

async fn collect(c: &Client, duration: f64, include_sql: bool) -> Result<Snapshot> {
    let started = Utc::now();
    let row = c
        .query_one(
            "SELECT current_setting('server_version'),current_database(),current_user",
            &[],
        )
        .await?;
    let queries = query_stats(c, include_sql).await?;
    let statements_available = c
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname='pg_stat_statements')",
            &[],
        )
        .await?
        .get::<_, bool>(0);
    let settings = c.query("SELECT name, setting || COALESCE(unit,'') FROM pg_settings WHERE name IN ('track_io_timing','track_planning','shared_buffers','work_mem','effective_cache_size','random_page_cost','effective_io_concurrency','max_parallel_workers_per_gather','default_statistics_target','TimeZone','jit') ORDER BY name", &[]).await?.into_iter().map(|r| (r.get::<_,String>(0),r.get::<_,String>(1))).collect();
    let tables=c.query("SELECT schemaname,relname,COALESCE(n_live_tup,0)::float8,seq_scan::bigint,idx_scan::bigint,pg_relation_size(relid)::bigint FROM pg_stat_user_tables",&[]).await?.into_iter().map(|r|TableStat{schema:r.get(0),name:r.get(1),live_tuples:r.get(2),seq_scan:r.get(3),idx_scan:r.get(4),size_bytes:r.get(5)}).collect();
    let indexes=c.query("SELECT n.schemaname,t.relname,i.relname,pg_get_indexdef(i.oid),x.indisvalid,COALESCE(s.idx_scan,0)::bigint,pg_relation_size(i.oid)::bigint FROM pg_index x JOIN pg_class i ON i.oid=x.indexrelid JOIN pg_class t ON t.oid=x.indrelid JOIN pg_namespace n ON n.oid=t.relnamespace LEFT JOIN pg_stat_user_indexes s ON s.indexrelid=i.oid",&[]).await?.into_iter().map(|r|IndexStat{schema:r.get(0),table:r.get(1),name:r.get(2),definition:r.get(3),valid:r.get(4),scans:r.get(5),size_bytes:r.get(6)}).collect();
    let locks = c
        .query(LOCK_SAMPLE_SQL, &[])
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| lock_stat(r, include_sql))
        .collect();
    Ok(Snapshot {
        schema_version: 1,
        id: uuid::Uuid::new_v4().to_string(),
        captured_at: started,
        duration_seconds: duration,
        server_version: row.get(0),
        database: row.get(1),
        role: row.get(2),
        settings,
        statements_available,
        include_sql,
        queries,
        tables,
        indexes,
        locks,
        capture_issues: Vec::new(),
    })
}
async fn capture(duration: &str, output: PathBuf, include_sql: bool) -> Result<()> {
    let secs = parse_duration(duration)?;
    let c = connect().await?;
    let before = collect(&c, 0.0, include_sql).await?;
    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
    let after = collect(&c, secs, include_sql).await?;
    let snap = delta_snapshots(before, after)?;
    fs::create_dir_all(&output)?;
    let path = output.join("investigation.json");
    atomic_write(&path, &serde_json::to_vec_pretty(&snap)?)?;
    println!(
        "Capture saved to {} ({} statements, {} lock observations)",
        path.display(),
        snap.queries.len(),
        snap.locks.len()
    );
    Ok(())
}
fn parse_duration(s: &str) -> Result<f64> {
    let n = s
        .strip_suffix("ms")
        .map(|v| (v, 0.001))
        .or_else(|| s.strip_suffix('s').map(|v| (v, 1.0)))
        .context("duration must look like 500ms or 30s")?;
    let v: f64 = n.0.parse()?;
    if !(0.0..=3600.0).contains(&(v * n.1)) {
        bail!("duration must be between 0 and 3600 seconds")
    };
    if v <= 0.0 {
        bail!("duration must be greater than zero")
    }
    Ok(v * n.1)
}
fn delta_snapshots(a: Snapshot, mut b: Snapshot) -> Result<Snapshot> {
    let map: std::collections::HashMap<(i64, i64, i64, bool), QueryStat> =
        std::collections::HashMap::from_iter(
            a.queries
                .into_iter()
                .map(|q| ((q.dbid, q.userid, q.queryid, q.toplevel), q)),
        );
    let ending_keys: std::collections::HashSet<_> = b
        .queries
        .iter()
        .map(|q| (q.dbid, q.userid, q.queryid, q.toplevel))
        .collect();
    let mut out = Vec::new();
    let mut issues = Vec::new();
    for mut q in b.queries.drain(..) {
        let Some(old) = map.get(&(q.dbid, q.userid, q.queryid, q.toplevel)) else {
            issues.push(DeltaIssue { query_id: Some(q.queryid), reason: "query statistics entry appeared after the starting snapshot; no reliable window delta is available".into() });
            continue;
        };
        if q.calls < old.calls
            || q.plans < old.plans
            || q.total_exec_time < old.total_exec_time
            || q.total_plan_time < old.total_plan_time
            || q.rows < old.rows
            || q.shared_blks_hit < old.shared_blks_hit
            || q.shared_blks_read < old.shared_blks_read
            || q.shared_blks_dirtied < old.shared_blks_dirtied
            || q.shared_blks_written < old.shared_blks_written
            || q.temp_blks_read < old.temp_blks_read
            || q.temp_blks_written < old.temp_blks_written
            || q.blk_read_time < old.blk_read_time
            || q.blk_write_time < old.blk_write_time
            || q.wal_bytes < old.wal_bytes
        {
            issues.push(DeltaIssue { query_id: Some(q.queryid), reason: "a cumulative counter decreased; statistics may have reset or restarted during the capture".into() });
            continue;
        }
        q.calls -= old.calls;
        q.plans -= old.plans;
        q.total_exec_time -= old.total_exec_time;
        q.total_plan_time -= old.total_plan_time;
        q.rows -= old.rows;
        q.shared_blks_hit -= old.shared_blks_hit;
        q.shared_blks_read -= old.shared_blks_read;
        q.shared_blks_dirtied -= old.shared_blks_dirtied;
        q.shared_blks_written -= old.shared_blks_written;
        q.temp_blks_read -= old.temp_blks_read;
        q.temp_blks_written -= old.temp_blks_written;
        q.blk_read_time -= old.blk_read_time;
        q.blk_write_time -= old.blk_write_time;
        q.wal_bytes -= old.wal_bytes;
        q.mean_exec_time = if q.calls > 0 {
            q.total_exec_time / q.calls as f64
        } else {
            0.0
        };
        q.max_exec_time = None;
        out.push(q)
    }
    for key in map.keys() {
        if !ending_keys.contains(key) {
            issues.push(DeltaIssue { query_id: Some(key.2), reason: "query statistics entry disappeared by the ending snapshot; counters may have been reset or evicted".into() });
        }
    }
    b.queries = out;
    b.capture_issues = issues;
    Ok(b)
}
async fn explain(
    path: PathBuf,
    analyze: bool,
    allow: bool,
    allow_non_select: bool,
    output: Option<PathBuf>,
) -> Result<()> {
    if analyze && !allow {
        bail!(
            "EXPLAIN ANALYZE executes the supplied SQL. Pass --allow-execution to authorize it explicitly."
        )
    }
    let sql = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    if sql.trim().is_empty() {
        bail!("SQL file is empty")
    }
    if analyze && !sql.trim_start().to_ascii_lowercase().starts_with("select") && !allow_non_select
    {
        bail!(
            "EXPLAIN ANALYZE of non-SELECT or unrecognized SQL requires --allow-non-select in addition to --analyze --allow-execution"
        )
    }
    if analyze {
        eprintln!(
            "Warning: EXPLAIN ANALYZE executes the statement. SELECT statements can still call functions with side effects."
        );
    }
    let c = connect().await?;
    let statement = if analyze {
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) ".to_string() + &sql
    } else {
        "EXPLAIN (FORMAT JSON) ".to_string() + &sql
    };
    let rows = c.query(&statement, &[]).await.context(
        "EXPLAIN failed; the statement was not run unless --analyze was explicitly supplied",
    )?;
    let value: serde_json::Value = rows[0].get(0);
    let parsed = plan::PlanDocument::parse(value, true)?;
    let findings = parsed.findings();
    let json = serde_json::to_vec_pretty(&parsed)?;
    if let Some(path) = output {
        atomic_write(&path, &json)?;
        println!("Plan saved to {}", path.display());
    } else {
        println!("{}", String::from_utf8(json)?);
    }
    for finding in findings {
        eprintln!(
            "{} [{} confidence] {}: {}\n  Next: {}",
            finding.rule_id,
            finding.confidence,
            finding.node_type,
            finding.evidence,
            finding.next_step
        );
    }
    Ok(())
}
fn load(p: &Path) -> Result<Snapshot> {
    let file = if p.is_dir() {
        p.join("investigation.json")
    } else {
        p.to_path_buf()
    };
    let snapshot: Snapshot = serde_json::from_slice(
        &fs::read(&file).with_context(|| format!("reading {}", file.display()))?,
    )
    .context("invalid investigation JSON")?;
    if snapshot.schema_version != 1 {
        bail!(
            "unsupported investigation schema version {}",
            snapshot.schema_version
        );
    }
    Ok(snapshot)
}
fn diagnose(p: PathBuf, plan_path: Option<PathBuf>) -> Result<()> {
    let s = load(&p)?;
    let mut n = 0;
    if !s.statements_available {
        println!(
            "No query statistics captured. Ensure pg_stat_statements is installed and accessible."
        );
    } else {
        for issue in &s.capture_issues {
            println!(
                "Capture limitation queryid={:?}: {}",
                issue.query_id, issue.reason
            );
        }
        for q in &s.queries {
            if q.calls == 0 {
                continue;
            }
            let ratio = q.shared_blks_read as f64 / (q.rows.max(1) as f64);
            if q.total_exec_time > 100.0 && ratio > 1.0 {
                n += 1;
                println!(
                    "HIGH_BUFFER_READS [medium confidence] queryid={} calls={} shared_reads={} rows={} total_exec_ms={:.2}\n  Evidence: {:.2} shared block reads per returned row.\n  Hypothesis: this query may be doing substantial buffer work. Inspect its plan and workload selectivity before considering a change.\n  Caveat: block reads may be cached and queryid may vary across server versions.",
                    q.queryid, q.calls, q.shared_blks_read, q.rows, q.total_exec_time, ratio
                )
            }
            if q.calls > 100 && q.total_exec_time > 500.0 {
                n += 1;
                println!(
                    "REPEATED_EXECUTION_COST [medium confidence] queryid={} calls={} total_exec_ms={:.2} mean_ms={:.3}\n  Evidence: repeated calls contributed cumulative recorded execution time. Consider batching or reducing request frequency if semantics allow.",
                    q.queryid, q.calls, q.total_exec_time, q.mean_exec_time
                )
            }
        }
    }
    for l in &s.locks {
        n += 1;
        println!(
            "LOCK_WAIT_OBSERVED [high confidence] pid={} queryid={:?} event={:?} blockers={:?}",
            l.pid, l.query_id, l.wait_event, l.blockers
        )
    }
    if let Some(path) = plan_path {
        let plan: plan::PlanDocument = serde_json::from_slice(
            &fs::read(&path).with_context(|| format!("reading plan {}", path.display()))?,
        )
        .context("invalid normalized plan JSON")?;
        for finding in plan.findings() {
            n += 1;
            println!(
                "{} [{} confidence; {} severity] {:?}: {}\n  Evidence: {}\n  Next: {}",
                finding.rule_id,
                finding.confidence,
                finding.severity,
                finding.relation,
                finding.node_type,
                finding.evidence,
                finding.next_step
            );
        }
    }
    if n == 0 {
        println!(
            "No implemented diagnosis rule crossed its threshold. This is not proof that performance is healthy."
        )
    }
    Ok(())
}
fn compare(before: PathBuf, after: PathBuf) -> Result<()> {
    let a = load(&before)?;
    let b = load(&after)?;
    let mut comparable = true;
    if a.database != b.database {
        comparable = false;
        println!(
            "Warning: databases differ ({} vs {})",
            a.database, b.database
        )
    }
    if a.server_version.split('.').next() != b.server_version.split('.').next() {
        comparable = false;
        println!(
            "Warning: PostgreSQL major versions differ; query identity and planner behavior may not be comparable"
        )
    };
    println!(
        "Before: {} ({})\nAfter:  {} ({})",
        a.captured_at, a.database, b.captured_at, b.database
    );
    if !a.capture_issues.is_empty() || !b.capture_issues.is_empty() {
        comparable = false;
        println!(
            "Capture contains {} before and {} after counter integrity issue(s); PostgreSQL counter comparison is limited.",
            a.capture_issues.len(),
            b.capture_issues.len()
        );
    }
    let am: std::collections::HashMap<(i64, i64, i64, bool), &QueryStat> =
        std::collections::HashMap::from_iter(
            a.queries
                .iter()
                .map(|q| ((q.dbid, q.userid, q.queryid, q.toplevel), q)),
        );
    for q in &b.queries {
        if let Some(x) = am.get(&(q.dbid, q.userid, q.queryid, q.toplevel)) {
            println!(
                "queryid {}: exec total {:.2} -> {:.2} ms ({}) | calls {} -> {} | shared reads {} -> {}",
                q.queryid,
                x.total_exec_time,
                q.total_exec_time,
                change(x.total_exec_time, q.total_exec_time),
                x.calls,
                q.calls,
                x.shared_blks_read,
                q.shared_blks_read
            )
        }
    }
    let before_run = load_run_summary(&before)?;
    let after_run = load_run_summary(&after)?;
    match (before_run, after_run) {
        (Some(x), Some(y)) => {
            let workload_matches = x.workload_name == y.workload_name
                && x.duration_seconds == y.duration_seconds
                && x.warmup_seconds == y.warmup_seconds
                && x.cooldown_seconds == y.cooldown_seconds
                && x.target_rps == y.target_rps
                && x.max_in_flight == y.max_in_flight
                && x.seed == y.seed
                && x.operations == y.operations
                && x.target_origin == y.target_origin
                && x.application_version == y.application_version
                && x.workload_version == y.workload_version;
            if !workload_matches {
                comparable = false;
                println!("Workload conditions differ; request results are NOT_COMPARABLE.");
            }
            if x.database_settings != y.database_settings {
                comparable = false;
                println!(
                    "Database settings changed; query performance measurements are NOT_COMPARABLE."
                );
            }
            for (label, run) in [("before", &x), ("after", &y)] {
                let arrival_interval_ms = 1000.0 / run.target_rps as f64;
                let scheduling_p95 = run.metrics.scheduling_delay_ms.p95.unwrap_or(0.0);
                if run.metrics.dropped_at_concurrency_cap > 0
                    || scheduling_p95 > arrival_interval_ms
                {
                    comparable = false;
                    println!(
                        "{label} run overloaded its scheduler: dropped={} p95 scheduling delay={scheduling_p95:.2} ms for {arrival_interval_ms:.2} ms arrival intervals.",
                        run.metrics.dropped_at_concurrency_cap
                    );
                }
                if run.metrics.completed < 100 {
                    comparable = false;
                    println!(
                        "{label} run has only {} completed requests; at least 100 are required for this comparison.",
                        run.metrics.completed
                    );
                }
                if run.metrics.error_rate.is_some_and(|rate| rate > 0.01) {
                    comparable = false;
                    println!(
                        "{label} run error rate exceeds 1%; performance comparison is NOT_COMPARABLE."
                    );
                }
            }
            if x.dataset_fingerprint_approx_sha256 != y.dataset_fingerprint_approx_sha256 {
                comparable = false;
                println!(
                    "Approximate table-size/tuple-estimate fingerprint changed; dataset comparability is NOT_ESTABLISHED."
                );
            }
            if x.postgres_version.split('.').next() != y.postgres_version.split('.').next() {
                comparable = false;
            }
            let before_indexes: std::collections::HashSet<_> = x.index_definitions.iter().collect();
            let after_indexes: std::collections::HashSet<_> = y.index_definitions.iter().collect();
            for definition in after_indexes.difference(&before_indexes) {
                println!("Index added after baseline: {definition}");
            }
            for definition in before_indexes.difference(&after_indexes) {
                println!("Index removed after baseline: {definition}");
            }
            if x.workload_hash_sha256 != y.workload_hash_sha256 {
                println!(
                    "Workload manifest hashes differ. Operation IDs and weights {} match, but route/query semantics require review.",
                    if workload_matches { "do" } else { "do not" }
                );
            }
            if x.query_variant != y.query_variant {
                println!(
                    "Query variant changed: {:?} -> {:?}",
                    x.query_variant, y.query_variant
                );
            }
            println!(
                "HTTP request measurements (API latency, distinct from PostgreSQL execution time):"
            );
            print_latency_change(
                "p50",
                x.metrics.latency_ms.median,
                y.metrics.latency_ms.median,
            );
            print_latency_change("p95", x.metrics.latency_ms.p95, y.metrics.latency_ms.p95);
            print_latency_change("p99", x.metrics.latency_ms.p99, y.metrics.latency_ms.p99);
            println!(
                "Throughput: {:.2} -> {:.2} requests/s | errors: {} -> {} | completed: {} -> {}",
                x.metrics.throughput_requests_per_second,
                y.metrics.throughput_requests_per_second,
                x.metrics
                    .error_rate
                    .map(|v| format!("{:.2}%", v * 100.0))
                    .unwrap_or_else(|| "unavailable".into()),
                y.metrics
                    .error_rate
                    .map(|v| format!("{:.2}%", v * 100.0))
                    .unwrap_or_else(|| "unavailable".into()),
                x.metrics.completed,
                y.metrics.completed
            );
            println!(
                "Verdict: {}. A single trial does not estimate run-to-run variation; repeated trials and semantic validation are still required.",
                if !comparable {
                    "NOT_COMPARABLE"
                } else {
                    "INCONCLUSIVE"
                }
            );
        }
        _ => println!(
            "Verdict: INCONCLUSIVE. These artifacts contain PostgreSQL counter deltas only; HTTP request latency and comparable workload metadata were not found."
        ),
    }
    Ok(())
}
fn load_run_summary(path: &Path) -> Result<Option<RunArtifact>> {
    let file = if path.is_dir() {
        path.join("summary.json")
    } else {
        return Ok(None);
    };
    if !file.exists() {
        return Ok(None);
    }
    let summary: RunArtifact = serde_json::from_slice(
        &fs::read(&file).with_context(|| format!("reading {}", file.display()))?,
    )
    .context("invalid workload run summary")?;
    if summary.schema_version != 1 {
        bail!(
            "unsupported workload run schema version {}",
            summary.schema_version
        );
    }
    Ok(Some(summary))
}
fn print_latency_change(name: &str, before: Option<f64>, after: Option<f64>) {
    match (before, after) {
        (Some(x), Some(y)) => println!("  {name}: {:.3} -> {:.3} ms ({})", x, y, change(x, y)),
        _ => println!("  {name}: unavailable"),
    }
}
fn change(x: f64, y: f64) -> String {
    if x == 0.0 {
        "inconclusive".into()
    } else {
        format!("{:+.1}%", (y - x) / x * 100.0)
    }
}
fn report(before: PathBuf, after: PathBuf, format: ReportFormat, output: PathBuf) -> Result<()> {
    let a = load(&before)?;
    let b = load(&after)?;
    let before_run = load_run_summary(&before)?;
    let after_run = load_run_summary(&after)?;
    let run_text = match (&before_run, &after_run) {
        (Some(x), Some(y)) => format!("\n## HTTP workload\n\n| Metric | Before | After | Change |\n|---|---:|---:|---:|\n| p50 latency | {} | {} | {} |\n| p95 latency | {} | {} | {} |\n| p99 latency | {} | {} | {} |\n| throughput | {:.2} req/s | {:.2} req/s | {} |\n| errors | {} | {} | — |\n\nVerdict: INCONCLUSIVE (single trial; no run-to-run variation estimate). Query variants: {:?} → {:?}.\n", metric(x.metrics.latency_ms.median), metric(y.metrics.latency_ms.median), metric_change(x.metrics.latency_ms.median,y.metrics.latency_ms.median), metric(x.metrics.latency_ms.p95), metric(y.metrics.latency_ms.p95), metric_change(x.metrics.latency_ms.p95,y.metrics.latency_ms.p95), metric(x.metrics.latency_ms.p99), metric(y.metrics.latency_ms.p99), metric_change(x.metrics.latency_ms.p99,y.metrics.latency_ms.p99), x.metrics.throughput_requests_per_second, y.metrics.throughput_requests_per_second, change(x.metrics.throughput_requests_per_second,y.metrics.throughput_requests_per_second), metric(x.metrics.error_rate.map(|v|v*100.0)), metric(y.metrics.error_rate.map(|v|v*100.0)), x.query_variant,y.query_variant),
        _ => "\nHTTP request latency was not captured in both artifacts; comparison verdict is INCONCLUSIVE.\n".to_string(),
    };
    let mut safe_a = a.clone();
    let mut safe_b = b.clone();
    for query in safe_a.queries.iter_mut().chain(safe_b.queries.iter_mut()) {
        query.query = None;
    }
    for lock in safe_a.locks.iter_mut().chain(safe_b.locks.iter_mut()) {
        lock.query = None;
    }
    let body = match format {
        ReportFormat::Terminal => format!(
            "POSTGRESQL PERFORMANCE COMPARISON\nBefore: {} ({})\nAfter: {} ({})\nMatched PostgreSQL query deltas: {}\n{}\nRequest latency evidence is a single trial; interpret changes as observations, not a validated optimization.",
            a.captured_at,
            a.database,
            b.captured_at,
            b.database,
            b.queries.len(),
            run_text
        ),
        ReportFormat::Markdown => format!(
            "# PostgreSQL comparison\n\n- Before: {}\n- After: {}\n- Database: {} → {}\n- Matched query deltas: {}\n\nPostgreSQL execution time is cumulative execution time in milliseconds, not request wall-clock latency.\n{}\n",
            a.captured_at,
            b.captured_at,
            a.database,
            b.database,
            b.queries.len(),
            run_text
        ),
        ReportFormat::Html => format!(
            "<!doctype html><html lang=\"en\"><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>PostgreSQL comparison</title><style>body{{font:16px system-ui,sans-serif;max-width:900px;margin:3rem auto;padding:0 1rem;color:#172033;background:#f7f8fa}}main{{background:white;border:1px solid #d9deea;border-radius:12px;padding:2rem}}table{{border-collapse:collapse;width:100%}}td,th{{text-align:left;padding:.65rem;border-bottom:1px solid #ddd}}code{{font-family:ui-monospace,monospace}}@media(prefers-color-scheme:dark){{body{{background:#10151f;color:#e6edf8}}main{{background:#171f2c;border-color:#344056}}td,th{{border-color:#344056}}}}</style><main><h1>PostgreSQL comparison</h1><p>Before: {}</p><p>After: {}</p><p>Database: {} → {}</p><p>Matched query deltas: {}</p><div>{}</div><p>Request latency is one-trial evidence; it does not estimate run-to-run variation.</p></main></html>",
            a.captured_at,
            b.captured_at,
            html_escape(&a.database),
            html_escape(&b.database),
            b.queries.len(),
            html_escape(&run_text).replace("\n", "<br>"),
        ),
        ReportFormat::Json => serde_json::to_string_pretty(
            &serde_json::json!({"schema_version":1,"before":safe_a,"after":safe_b,"before_run":before_run,"after_run":after_run,"verdict":"INCONCLUSIVE: single-trial comparisons are not statistically validated"}),
        )?,
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?
    }
    atomic_write(&output, body.as_bytes())?;
    println!("Report written to {}", output.display());
    Ok(())
}
fn metric(value: Option<f64>) -> String {
    value
        .map(|x| format!("{x:.3}"))
        .unwrap_or_else(|| "unavailable".into())
}
fn metric_change(before: Option<f64>, after: Option<f64>) -> String {
    match (before, after) {
        (Some(x), Some(y)) => change(x, y),
        _ => "unavailable".into(),
    }
}
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duration_units() {
        assert_eq!(parse_duration("2s").unwrap(), 2.0);
        assert_eq!(parse_duration("250ms").unwrap(), 0.25)
    }
    #[test]
    fn percentage_marks_zero_baseline_inconclusive() {
        assert_eq!(change(0.0, 4.0), "inconclusive")
    }
    #[test]
    fn html_escapes_text() {
        assert_eq!(html_escape("<db&>"), "&lt;db&amp;&gt;")
    }
    #[test]
    fn documented_workload_manifest_parses_and_validates() {
        let manifest: WorkloadManifest =
            toml::from_str(include_str!("../demo/workloads/orders-search.toml")).unwrap();
        validate_manifest(&manifest).unwrap();
        assert_eq!(manifest.workload.name, "orders-search");
        assert_eq!(manifest.operations.len(), 3);
    }
    #[test]
    fn workload_validation_rejects_unbounded_and_credentialed_targets() {
        let mut manifest: WorkloadManifest =
            toml::from_str(include_str!("../demo/workloads/orders-search.toml")).unwrap();
        manifest.target.base_url = "http://user:secret@example.test".into();
        assert!(validate_manifest(&manifest).is_err());
        manifest.target.base_url = "http://localhost:8080".into();
        manifest.workload.max_in_flight = 513;
        assert!(validate_manifest(&manifest).is_err());
    }
    #[test]
    fn latency_summary_uses_nearest_rank_and_sample_deviation() {
        let result = latency_summary(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(result.median, Some(2.5));
        assert_eq!(result.p95, Some(4.0));
        assert_eq!(result.samples, 4);
    }
    #[test]
    fn seeded_operation_selection_and_parameters_are_repeatable() {
        let manifest: WorkloadManifest =
            toml::from_str(include_str!("../demo/workloads/orders-search.toml")).unwrap();
        assert_eq!(
            choose_operation(&manifest.operations, 42017).unwrap().id,
            choose_operation(&manifest.operations, 42017).unwrap().id
        );
        assert_eq!(
            expand_values("/orders/{order_id}?tenant={tenant_id}", 42017),
            expand_values("/orders/{order_id}?tenant={tenant_id}", 42017)
        );
    }
    #[test]
    fn remote_database_requires_tls_and_loopback_uses_local_mode() {
        use tokio_postgres::config::{Config, SslMode};
        let mut remote: Config = "host=db.example.test user=pgpa sslmode=prefer"
            .parse()
            .unwrap();
        assert!(prepare_db_tls(&mut remote).unwrap());
        assert_eq!(remote.get_ssl_mode(), SslMode::Require);
        let mut insecure: Config = "host=203.0.113.20 user=pgpa sslmode=disable"
            .parse()
            .unwrap();
        assert!(prepare_db_tls(&mut insecure).is_err());
        let mut local: Config = "host=localhost user=pgpa".parse().unwrap();
        assert!(!prepare_db_tls(&mut local).unwrap());
        assert_eq!(local.get_ssl_mode(), SslMode::Disable);
    }
    fn test_snapshot(queries: Vec<QueryStat>) -> Snapshot {
        Snapshot {
            schema_version: 1,
            id: "test".into(),
            captured_at: Utc::now(),
            duration_seconds: 1.0,
            server_version: "18".into(),
            database: "test".into(),
            role: "test".into(),
            settings: BTreeMap::new(),
            statements_available: true,
            include_sql: false,
            queries,
            tables: vec![],
            indexes: vec![],
            locks: vec![],
            capture_issues: vec![],
        }
    }
    fn query_stat(calls: i64, total: f64, max: f64) -> QueryStat {
        QueryStat {
            dbid: 1,
            userid: 1,
            queryid: 99,
            toplevel: true,
            calls,
            total_exec_time: total,
            plans: calls,
            total_plan_time: 0.0,
            mean_exec_time: total / (calls.max(1) as f64),
            max_exec_time: Some(max),
            rows: calls * 2,
            shared_blks_hit: calls,
            shared_blks_read: calls,
            shared_blks_dirtied: 0,
            shared_blks_written: 0,
            temp_blks_read: 0,
            temp_blks_written: 0,
            blk_read_time: 0.0,
            blk_write_time: 0.0,
            wal_bytes: 0.0,
            query: None,
        }
    }
    #[test]
    fn capture_delta_marks_reset_and_window_max_unavailable() {
        let before = test_snapshot(vec![query_stat(10, 100.0, 20.0)]);
        let after = test_snapshot(vec![query_stat(13, 160.0, 40.0)]);
        let delta = delta_snapshots(before, after).unwrap();
        assert_eq!(delta.queries[0].calls, 3);
        assert_eq!(delta.queries[0].total_exec_time, 60.0);
        assert_eq!(delta.queries[0].mean_exec_time, 20.0);
        assert_eq!(delta.queries[0].max_exec_time, None);
        let reset = delta_snapshots(
            test_snapshot(vec![query_stat(10, 100.0, 20.0)]),
            test_snapshot(vec![query_stat(0, 0.0, 0.0)]),
        )
        .unwrap();
        assert!(reset.queries.is_empty());
        assert_eq!(reset.capture_issues.len(), 1);
    }
}
