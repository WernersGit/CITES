//! This crate contains all shared fullstack server functions.
use dioxus::prelude::*;

pub mod ble_constants;

pub mod metrics;
pub use metrics::{MetricsService, NodeStatus, SystemMetrics, TrackingReport, VirtualVehicle};

pub mod storage;

/// Gets the current system metrics via Dioxus Server Function (for Cloud/Web Mode testing)
#[post("/api/metrics")]
pub async fn get_system_metrics() -> Result<SystemMetrics, ServerFnError> {
    let mut service = MetricsService::new();
    Ok(service.gather_metrics())
}
