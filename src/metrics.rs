use std::sync::OnceLock;

use prometheus::{IntCounterVec, Opts};

pub struct Metrics {
    /// Requests made to container registries, labeled by registry hostname,
    /// operation (list_tags | fetch_digest), and outcome (ok | error |
    /// rate_limited). list_tags counts one increment per page request.
    pub registry_api_calls: IntCounterVec,
    /// Completed repo refresh attempts by outcome (ok | error).
    pub repo_refreshes: IntCounterVec,
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

pub fn init(registry: &prometheus::Registry) -> Result<(), anyhow::Error> {
    let registry_api_calls = IntCounterVec::new(
        Opts::new(
            "watchtower_registry_api_calls_total",
            "Requests made to container registries",
        ),
        &["registry", "operation", "outcome"],
    )?;
    registry.register(Box::new(registry_api_calls.clone()))?;

    let repo_refreshes = IntCounterVec::new(
        Opts::new(
            "watchtower_repo_refreshes_total",
            "Completed repo refresh attempts",
        ),
        &["outcome"],
    )?;
    registry.register(Box::new(repo_refreshes.clone()))?;

    let metrics = Metrics {
        registry_api_calls,
        repo_refreshes,
    };

    METRICS
        .set(metrics)
        .map_err(|_| anyhow::anyhow!("Metrics already initialized"))?;

    Ok(())
}

#[allow(clippy::expect_used)]
pub fn get() -> &'static Metrics {
    METRICS
        .get()
        .expect("Metrics not initialized - call metrics::init() first")
}
