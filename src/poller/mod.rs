use std::time::Duration;

use oci_client::Reference;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use tokio::time::{self};

pub async fn start_update_poller(
    __pool: Pool<SqliteConnectionManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = oci_client::Client::default();

    log::info!("Starting update poller");

    let mut interval = time::interval(Duration::from_mins(1));

    loop {
        interval.tick().await;
        log::info!("update poller tick");
        refresh_manifests(&client).await;
    }
}

pub async fn refresh_manifests(client: &oci_client::Client) {
    let reference: Reference = "ghcr.io/qdm12/gluetun:latest".parse().unwrap();
    let response = client
        .list_tags(
            &reference,
            &oci_client::secrets::RegistryAuth::Anonymous,
            None,
            None,
        )
        .await
        .unwrap();

    let mut result = Vec::new();

    for tag in response.tags.clone() {
        let reference: Reference = format!(
            "{}/{}:{}",
            reference.registry(),
            reference.repository(),
            tag
        )
        .parse()
        .unwrap();

        let digest = client
            .fetch_manifest_digest(&reference, &oci_client::secrets::RegistryAuth::Anonymous)
            .await
            .unwrap();

        result.push((tag, digest));
    }

    log::info!("Registry result: {:?}, mapping: {:?}", response, result);
}
