use crate::erlang::{RpcClient, SystemVersion};
use crate::Options;
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

pub type MetricsReceiver = mpsc::Receiver<Metrics>;
pub type MetricsSender = mpsc::Sender<Metrics>;

#[derive(Debug, Clone)]
pub struct Metrics {
    pub time: SystemTime,
    pub timestamp: Instant,
    pub items: BTreeMap<String, MetricValue>,
}

impl Metrics {
    fn new() -> Self {
        Self {
            time: SystemTime::now(),
            timestamp: Instant::now(),
            items: BTreeMap::new(),
        }
    }

    fn insert(&mut self, name: &str, value: MetricValue) {
        self.items.insert(name.to_owned(), value);
    }

    pub fn root_metrics_count(&self) -> usize {
        self.items.values().filter(|x| x.parent().is_none()).count()
    }
}

#[derive(Debug, Clone)]
pub enum MetricValue {
    Gauge {
        value: u64,
        parent: Option<String>,
    },
    Counter {
        value: u64,
        delta_per_sec: Option<u64>,
        parent: Option<String>,
    },
}

impl MetricValue {
    fn gauge(value: u64) -> Self {
        Self::Gauge {
            value,
            parent: None,
        }
    }

    pub fn parent(&self) -> Option<&str> {
        match self {
            Self::Gauge { parent, .. } => parent.as_ref().map(|x| x.as_str()),
            Self::Counter { parent, .. } => parent.as_ref().map(|x| x.as_str()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Msacc {}

#[derive(Debug)]
pub struct MetricsPoller {
    options: Options,
    rpc_client: RpcClient,
    tx: MetricsSender,
}

impl MetricsPoller {
    pub fn start_thread(options: Options) -> anyhow::Result<(SystemVersion, MetricsReceiver)> {
        let (tx, rx) = mpsc::channel();

        let rpc_client: RpcClient = smol::block_on(async {
            let cookie = options.find_cookie()?;
            let client = RpcClient::connect(&options.erlang_node, &cookie).await?;
            Ok(client) as anyhow::Result<_>
        })?;
        let system_version = smol::block_on(rpc_client.get_system_version())?;

        let poller = Self {
            options,
            rpc_client,
            tx,
        };
        std::thread::spawn(|| poller.run());
        Ok((system_version, rx))
    }

    fn run(mut self) {
        let interval = Duration::from_secs(self.options.polling_interval.get() as u64);
        smol::block_on(async {
            loop {
                match self.poll_once().await {
                    Err(e) => {
                        log::error!("faild to poll metrics: {e}");
                        break;
                    }
                    Ok(metrics) => {
                        let elapsed = metrics.timestamp.elapsed();
                        if self.tx.send(metrics).is_err() {
                            log::debug!("the main thread has terminated");
                            break;
                        }
                        if let Some(sleep_duration) = interval.checked_sub(elapsed) {
                            std::thread::sleep(sleep_duration);
                        }
                    }
                }
            }
        })
    }

    async fn poll_once(&mut self) -> anyhow::Result<Metrics> {
        let mut metrics = Metrics::new();
        let processes = self.rpc_client.get_system_info_u64("process_count").await?;
        metrics.insert("count.processes", MetricValue::gauge(processes));

        let ports = self.rpc_client.get_system_info_u64("port_count").await?;
        metrics.insert("count.ports", MetricValue::gauge(ports));

        // pub context_switches: Counter,
        // pub exact_reductions: Counter,
        // pub io_input_bytes: Counter,
        // pub io_output_bytes: Counter,
        // pub run_queue_lengths: Vec<Gauge>,
        // pub garbage_collection: u64,
        // pub runtime: Counter,
        // pub microstate_accounting: Msacc

        log::debug!(
            "MetricsPoller::poll_once(): elapsed={:?}",
            metrics.timestamp.elapsed()
        );

        Ok(metrics)
    }
}
