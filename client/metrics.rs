use crate::grpc_client::pb;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub tx_received: AtomicU64,
    pub tx_forward_succeeded: AtomicU64,
    pub tx_forward_failed: AtomicU64,
}

impl From<&Metrics> for pb::RequestMessageEnvelope {
    fn from(metrics: &Metrics) -> Self {
        pb::RequestMessageEnvelope {
            metrics: Some(pb::Metrics {
                tx_received: metrics.tx_received.load(Ordering::Relaxed),
                tx_forward_succeeded: metrics.tx_forward_succeeded.load(Ordering::Relaxed),
                tx_forward_failed: metrics.tx_forward_failed.load(Ordering::Relaxed),
                version: crate::VERSION.to_string(),
            }),
            ..Default::default()
        }
    }
}
