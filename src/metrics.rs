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

    // TODO: remove
    pub fn root_metrics_count(&self) -> usize {
        self.items.values().filter(|x| x.parent().is_none()).count()
    }

    pub fn root_items(&self) -> impl Iterator<Item = (&str, &MetricValue)> {
        self.items
            .iter()
            .filter(|(_, v)| v.parent().is_none())
            .map(|(k, v)| (k.as_str(), v))
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

impl std::fmt::Display for MetricValue {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Gauge { value, .. } => write!(f, "{}", format_u64(*value)),
            Self::Counter {
                delta_per_sec: Some(value),
                ..
            } => write!(f, "{}", format_u64(*value)),
            Self::Counter { .. } => write!(f, ""),
        }
    }
}

pub fn format_u64(mut n: u64) -> String {
    let mut s = Vec::new();
    for i in 0.. {
        if i % 3 == 0 && i != 0 {
            s.push(b',');
        }
        let m = n % 10;
        s.push(b'0' + m as u8);
        n /= 10;
        if n == 0 {
            break;
        }
    }
    s.reverse();
    String::from_utf8(s).expect("unreachable")
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

        let atoms = self.rpc_client.get_system_info_u64("atom_count").await?;
        metrics.insert("count.atoms", MetricValue::gauge(atoms));

        let ets_tables = self.rpc_client.get_system_info_u64("ets_count").await?;
        metrics.insert("count.ets_tables", MetricValue::gauge(ets_tables));

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
