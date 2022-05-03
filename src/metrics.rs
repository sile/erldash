use crate::erlang::RpcClient;
use crate::Options;
use std::sync::mpsc;

pub type MetricsReceiver = mpsc::Receiver<Metrics>;
pub type MetricsSender = mpsc::Sender<Metrics>;

#[derive(Debug, Clone)]
pub struct Metrics {}

#[derive(Debug)]
pub struct MetricsPoller {
    options: Options,
    rpc_client: RpcClient,
}

impl MetricsPoller {
    pub fn start_thread(options: Options) -> anyhow::Result<MetricsReceiver> {
        let (tx, rx) = mpsc::channel();
        Ok(rx)
    }
}
