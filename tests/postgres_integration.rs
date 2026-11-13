//! Real PostgreSQL integration checks. Set PGPA_TEST_DATABASE_URL to a disposable test database.
use std::time::Duration;
use tokio_postgres::{Client, Config, NoTls};

async fn connect(
    url: &str,
    application: &str,
) -> (
    Client,
    tokio_postgres::Connection<tokio_postgres::Socket, tokio_postgres::tls::NoTlsStream>,
) {
    let mut config: Config = url.parse().expect("valid PGPA_TEST_DATABASE_URL");
    config.application_name(application);
    let (client, connection) = config.connect(NoTls).await.expect("connect PostgreSQL");
    (client, connection)
}

#[tokio::test]
async fn postgres_stats_explain_and_lock_blocking_are_observable() {
    let Ok(url) = std::env::var("PGPA_TEST_DATABASE_URL") else {
        eprintln!("skipping PostgreSQL integration: PGPA_TEST_DATABASE_URL is unset");
        return;
    };
    let (mut first, first_conn) = connect(&url, "pgpa-integration-parent").await;
    tokio::spawn(async move {
        let _ = first_conn.await;
    });

    let version: i32 = first
        .query_one("SELECT current_setting('server_version_num')::int", &[])
        .await
        .unwrap()
        .get(0);
    assert!(
        version >= 160000,
        "integration target must be PostgreSQL 16 or newer"
    );
    let _ = first
        .batch_execute("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .await;
    let extension: bool = first
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname='pg_stat_statements')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        extension,
        "CI integration service must install pg_stat_statements"
    );
    let stats = first
        .query_one("SELECT count(*)::bigint FROM pg_stat_statements", &[])
        .await
        .unwrap()
        .get::<_, i64>(0);
    assert!(stats > 0);
    let (read_col, write_col) = if version >= 170000 {
        ("shared_blk_read_time", "shared_blk_write_time")
    } else {
        ("blk_read_time", "blk_write_time")
    };
    let compat_sql = format!(
        "SELECT toplevel, plans, total_plan_time, shared_blks_dirtied, shared_blks_written, {read_col}, {write_col}, wal_bytes FROM pg_stat_statements LIMIT 1"
    );
    let _schema_compatibility = first
        .query_one(&compat_sql, &[])
        .await
        .expect("PostgreSQL 16+ pg_stat_statements fields are available");

    let plan: serde_json::Value = first
        .query_one("EXPLAIN (FORMAT JSON) SELECT 1", &[])
        .await
        .unwrap()
        .get(0);
    assert!(plan[0]["Plan"]["Node Type"].as_str().is_some());
    let blocks: Vec<i32> = first
        .query_one("SELECT pg_blocking_pids(pg_backend_pid())", &[])
        .await
        .unwrap()
        .get(0);
    assert!(blocks.is_empty());

    let table = format!("pgpa_lock_test_{}", uuid::Uuid::new_v4().simple());
    first.batch_execute(&format!("CREATE TABLE {table} (id integer PRIMARY KEY, value integer NOT NULL); INSERT INTO {table} VALUES (1, 0);")).await.unwrap();
    let tx = first.transaction().await.unwrap();
    tx.query_one(
        &format!("SELECT id FROM {table} WHERE id=1 FOR UPDATE"),
        &[],
    )
    .await
    .unwrap();

    let (second, second_conn) = connect(&url, "pgpa-integration-waiter").await;
    tokio::spawn(async move {
        let _ = second_conn.await;
    });
    let (monitor, monitor_conn) = connect(&url, "pgpa-integration-monitor").await;
    tokio::spawn(async move {
        let _ = monitor_conn.await;
    });
    let update_table = table.clone();
    let update = tokio::spawn(async move {
        second
            .execute(
                &format!("UPDATE {update_table} SET value=value+1 WHERE id=1"),
                &[],
            )
            .await
    });
    let mut observed = false;
    for _ in 0..40 {
        let waiting: Option<i32> = monitor.query_opt("SELECT cardinality(pg_blocking_pids(pid))::int FROM pg_stat_activity WHERE application_name='pgpa-integration-waiter'", &[]).await.unwrap().map(|row| row.get(0));
        if waiting.is_some_and(|count| count > 0) {
            observed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        observed,
        "expected pg_blocking_pids to expose the controlled row-lock wait"
    );
    tx.commit().await.unwrap();
    update.await.unwrap().unwrap();
    let value: i32 = first
        .query_one(&format!("SELECT value FROM {table} WHERE id=1"), &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(value, 1);
    first
        .batch_execute(&format!("DROP TABLE {table}"))
        .await
        .unwrap();
}
