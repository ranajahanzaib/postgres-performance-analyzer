use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{NaiveDate, Utc};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, env, net::SocketAddr, time::Duration};
use tokio_postgres::{Config, NoTls};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
struct App {
    pool: Pool,
    variant: String,
}
#[derive(Serialize)]
struct Health {
    status: &'static str,
    timestamp: chrono::DateTime<Utc>,
    application_version: &'static str,
    query_variant: String,
}
#[derive(Serialize)]
struct Order {
    id: i64,
    tenant_id: i32,
    customer_id: i64,
    status: String,
    total_cents: i32,
    created_at: chrono::DateTime<Utc>,
}
#[derive(Serialize)]
struct Summary {
    customer_id: i64,
    orders: i64,
    total_cents: i64,
}
#[derive(Deserialize)]
struct Search {
    tenant_id: i32,
    status: String,
    date: NaiveDate,
    next_date: NaiveDate,
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    50
}
#[derive(Deserialize)]
struct Update {
    status: String,
    #[serde(default)]
    hold_ms: u64,
}
#[derive(Serialize)]
struct Updated {
    id: i64,
    status: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();
    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://pgpa:pgpa@postgres:5432/pgpa_demo".into());
    let mut config: Config = database_url.parse()?;
    config.application_name("pgpa-demo-api");
    config.options("-c statement_timeout=8000 -c lock_timeout=5000");
    let manager = Manager::from_config(
        config,
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        },
    );
    let pool = Pool::builder(manager).max_size(16).build()?;
    let app = App {
        pool,
        variant: env::var("ORDER_QUERY_VARIANT")
            .unwrap_or_else(|_| "missing_index_baseline".into()),
    };
    let router = Router::new()
        .route("/health", get(health))
        .route("/orders/search", get(search))
        .route("/orders/{id}", get(order))
        .route("/customers/{id}/summary", get(customer_summary))
        .route("/orders/{id}/status", post(update_status))
        .with_state(app)
        .layer(TraceLayer::new_for_http());
    let address: SocketAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
async fn health(State(app): State<App>) -> impl IntoResponse {
    match app.pool.get().await {
        Ok(client) => match client.query_one("SELECT 1", &[]).await {
            Ok(_) => (
                StatusCode::OK,
                Json(Health {
                    status: "ok",
                    timestamp: Utc::now(),
                    application_version: env!("CARGO_PKG_VERSION"),
                    query_variant: app.variant.clone(),
                }),
            )
                .into_response(),
            Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response(),
        },
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response(),
    }
}
async fn order(State(app): State<App>, Path(id): Path<i64>) -> Result<Json<Order>, StatusCode> {
    let client = app
        .pool
        .get()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let row = client
        .query_opt(
            "SELECT id,tenant_id,customer_id,status,total_cents,created_at FROM orders WHERE id=$1",
            &[&id],
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(Order {
        id: row.get(0),
        tenant_id: row.get(1),
        customer_id: row.get(2),
        status: row.get(3),
        total_cents: row.get(4),
        created_at: row.get(5),
    }))
}
async fn search(
    State(app): State<App>,
    Query(q): Query<Search>,
) -> Result<Json<Vec<Order>>, StatusCode> {
    if q.limit < 1 || q.limit > 500 || q.next_date <= q.date {
        return Err(StatusCode::BAD_REQUEST);
    }
    let client = app
        .pool
        .get()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let rows=if app.variant=="query_rewrite_baseline"{client.query("SELECT id,tenant_id,customer_id,status,total_cents,created_at FROM orders WHERE tenant_id=$1 AND status=$2 AND DATE(created_at AT TIME ZONE 'UTC')=$3 ORDER BY created_at DESC LIMIT $4",&[&q.tenant_id,&q.status,&q.date,&q.limit]).await}else{client.query("SELECT id,tenant_id,customer_id,status,total_cents,created_at FROM orders WHERE tenant_id=$1 AND status=$2 AND created_at >= ($3::date::timestamp AT TIME ZONE 'UTC') AND created_at < ($4::date::timestamp AT TIME ZONE 'UTC') ORDER BY created_at DESC LIMIT $5",&[&q.tenant_id,&q.status,&q.date,&q.next_date,&q.limit]).await}.map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
    let out = rows
        .into_iter()
        .map(|r| Order {
            id: r.get(0),
            tenant_id: r.get(1),
            customer_id: r.get(2),
            status: r.get(3),
            total_cents: r.get(4),
            created_at: r.get(5),
        })
        .collect();
    Ok(Json(out))
}
async fn customer_summary(
    State(app): State<App>,
    Path(id): Path<i64>,
) -> Result<Json<Summary>, StatusCode> {
    let client = app
        .pool
        .get()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let row=client.query_one("SELECT $1::bigint, count(o.id)::bigint, COALESCE(sum(o.total_cents),0)::bigint FROM orders o WHERE o.customer_id=$1",&[&id]).await.map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(Summary {
        customer_id: row.get(0),
        orders: row.get(1),
        total_cents: row.get(2),
    }))
}
async fn update_status(
    State(app): State<App>,
    Path(id): Path<i64>,
    Query(params): Query<HashMap<String, String>>,
    Json(update): Json<Update>,
) -> Result<Json<Updated>, StatusCode> {
    if !["pending", "complete", "cancelled"].contains(&update.status.as_str()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let hold_ms = params
        .get("hold_ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(update.hold_ms)
        .min(5000);
    let slow = app.variant == "lock_contention_baseline";
    if slow {
        let mut client = app
            .pool
            .get()
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let tx = client
            .transaction()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let row = tx
            .query_opt(
                "UPDATE orders SET status=$2,updated_at=now() WHERE id=$1 RETURNING id,status",
                &[&id, &update.status],
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        tokio::time::sleep(Duration::from_millis(hold_ms)).await;
        tx.commit()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(Json(Updated {
            id: row.get(0),
            status: row.get(1),
        }))
    } else {
        tokio::time::sleep(Duration::from_millis(hold_ms)).await;
        let client = app
            .pool
            .get()
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let row = client
            .query_opt(
                "UPDATE orders SET status=$2,updated_at=now() WHERE id=$1 RETURNING id,status",
                &[&id, &update.status],
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        Ok(Json(Updated {
            id: row.get(0),
            status: row.get(1),
        }))
    }
}
