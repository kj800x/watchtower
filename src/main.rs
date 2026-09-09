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
use crate::poller::{RefreshRequester, refresh_requester, start_update_poller};
use crate::prelude::*;
use crate::web::{
    add_exclusion, add_exclusion_form, create_repo, create_repo_form, get_hydrated_repo, get_repo,
    list_events, list_exclusions, list_hydrated_repos, list_repos, lookup_repo, refresh_repo_now,
    remove_exclusion, remove_exclusion_form, repo_detail_fragment, repo_detail_page,
    repo_rows_fragment, repos_page, set_repo_active, toggle_repo_active,
};
use watchtower::serve_static_file;

async fn start_http(
    registry: prometheus::Registry,
    pool: Pool<SqliteConnectionManager>,
    refresh: RefreshRequester,
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
            .app_data(Data::new(refresh.clone()))
            .wrap(middleware::Logger::default())
            .service(list_repos)
            .service(get_repo)
            .service(create_repo)
            .service(set_repo_active)
            .service(refresh_repo_now)
            .service(list_hydrated_repos)
            .service(get_hydrated_repo)
            .service(lookup_repo)
            .service(list_events)
            .service(list_exclusions)
            .service(add_exclusion)
            .service(remove_exclusion)
            .service(repos_page)
            .service(repo_rows_fragment)
            .service(create_repo_form)
            .service(toggle_repo_active)
            .service(repo_detail_page)
            .service(repo_detail_fragment)
            .service(add_exclusion_form)
            .service(remove_exclusion_form)
            .service(serve_static_file!("htmx.min.js"))
            .service(serve_static_file!("idiomorph.min.js"))
            .service(serve_static_file!("idiomorph-ext.min.js"))
            .service(serve_static_file!("styles.css"))
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

    let (refresh_tx, refresh_rx) = refresh_requester();

    tokio::select! {
        _ = Box::pin(start_http(
            registry,
            pool.clone(),
            refresh_tx,
        )) => {},
        _ = Box::pin(start_update_poller(
            pool.clone(),
            refresh_rx,
        )) => {},
    };

    Ok(())
}
