use std::time::Duration;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use tokio::time::{self};

pub async fn start_update_poller(
    __pool: Pool<SqliteConnectionManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Starting update poller");

    let mut interval = time::interval(Duration::from_mins(1));

    loop {
        interval.tick().await;
        log::info!("update poller tick")
    }
}
