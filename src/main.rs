pub mod prelude {
    pub use chrono::prelude::*;

    pub use actix_session::{Session, SessionMiddleware, storage::CookieSessionStore};
    pub use actix_web::{
        App, HttpResponse, HttpServer, Responder,
        cookie::Key,
        delete, error, get, middleware, post, put,
        web::{self, Data, Json, get as web_get, resource},
    };
    pub use actix_web_opentelemetry::{PrometheusMetricsHandler, RequestMetrics, RequestTracing};
    pub use futures_util::future::join_all;
    pub use opentelemetry::global;
    pub use opentelemetry_sdk::metrics::MeterProvider;
    pub use r2d2::Pool;
    pub use r2d2_sqlite::SqliteConnectionManager;
    pub use serde::{Deserialize, Serialize};

    pub use crate::error::{AppError, AppResult};
    pub use actix_web::Error;
    pub use actix_web::{Result, guard};
    pub use async_graphql::{EmptyMutation, EmptySubscription, Schema, http::GraphiQLSource};
    pub use async_graphql_actix_web::GraphQL;
    pub use maud::{DOCTYPE, Markup, html};
    pub use r2d2::PooledConnection;
    pub use rusqlite::Connection;
    pub use rusqlite::{OptionalExtension, params};
    pub use rusqlite_migration::{M, Migrations};
    pub use std::time::{SystemTime, UNIX_EPOCH};
}

mod db;
mod error;
mod metrics;
mod poller;
mod web;

use crate::db::{SqliteConnectionCustomizer, migrations::migrate};
use crate::poller::start_update_poller;
use crate::prelude::*;
use crate::web::{create_repo, get_repo, list_repos, set_repo_active, webhook_recent_list};
use watchtower::serve_static_file;

async fn start_http(
    registry: prometheus::Registry,
    pool: Pool<SqliteConnectionManager>,
) -> Result<(), std::io::Error> {
    log::info!("Starting HTTP server at http://localhost:8080/api");

    HttpServer::new(move || {
        let app = App::new();

        app.wrap(RequestTracing::new())
            .wrap(RequestMetrics::default())
            .route(
                "/api/metrics",
                web_get().to(PrometheusMetricsHandler::new(registry.clone())),
            )
            .wrap(
                SessionMiddleware::builder(CookieSessionStore::default(), Key::from(&[0; 64]))
                    .cookie_secure(false)
                    .build(),
            )
            .app_data(Data::new(pool.clone()))
            .wrap(middleware::Logger::default())
            .service(list_repos)
            .service(get_repo)
            .service(create_repo)
            .service(set_repo_active)
            .service(serve_static_file!("htmx.min.js"))
            .service(serve_static_file!("idiomorph.min.js"))
            .service(serve_static_file!("idiomorph-ext.min.js"))
            .service(serve_static_file!("styles.css"))
            .service(serve_static_file!("deploy.css"))
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}

#[actix_web::main]
#[allow(clippy::expect_used)]
async fn main() -> std::io::Result<()> {
    // Configure logger with custom filter to prioritize Discord logs
    env_logger::builder()
        .filter_level(log::LevelFilter::Info) // Set default level to Info for most modules
        .filter_module("actix_web::middleware::logger", log::LevelFilter::Warn) // Actix web middleware logs every request at info
        .parse_default_env()
        .init();

    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .build()
        .expect("Failed to build OpenTelemetry Prometheus exporter");
    let provider = MeterProvider::builder().with_reader(exporter).build();
    global::set_meter_provider(provider);
    metrics::init(&registry).expect("Failed to initialize metrics");

    // connect to SQLite DB
    let manager = SqliteConnectionManager::file(
        std::env::var("DATABASE_PATH").unwrap_or("db.db".to_string()),
    );
    let pool = Pool::builder()
        .connection_customizer(Box::new(SqliteConnectionCustomizer))
        .build(manager)
        .expect("Failed to create database pool");
    {
        let conn = pool.get().expect("Failed to get database connection");
        migrate(conn).expect("Failed to run database migrations");
    }

    tokio::select! {
        _ = Box::pin(start_http(
            registry,
            pool.clone(),
        )) => {},
        _ = Box::pin(start_update_poller(
            pool.clone()
        )) => {},
    };

    Ok(())
}
