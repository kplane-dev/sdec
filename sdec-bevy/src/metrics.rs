use std::time::Duration;

#[derive(Debug, Default, Clone, Copy)]
pub struct EncodeMetrics {
    pub bytes: usize,
    pub encode_time: Duration,
}

pub trait MetricsSink {
    fn record_encode(&mut self, metrics: EncodeMetrics);
}
