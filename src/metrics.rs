use crate::erlang::{RpcClient, SystemVersion};
use crate::Options;
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub type MetricsReceiver = mpsc::Receiver<Metrics>;
pub type MetricsSender = mpsc::Sender<Metrics>;

#[derive(Debug, Clone)]
pub struct Metrics {
    pub timestamp: Instant,
    pub items: BTreeMap<String, MetricValue>,
}

impl Metrics {
    fn new() -> Self {
        Self {
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

    pub fn child_items<'a, 'b>(
        &'a self,
        parent: &'b str,
    ) -> impl 'a + Iterator<Item = (&'a str, &'a MetricValue)>
    where
        'b: 'a,
    {
        self.items
            .iter()
            .filter(move |(_, v)| v.parent().as_ref().map_or(false, |&x| x == parent))
            .map(|(k, v)| (k.as_str(), v))
    }

    fn calc_delta(&mut self, prev: &Self) {
        let duration = self.timestamp - prev.timestamp;
        for (name, value) in &mut self.items {
            if let MetricValue::Counter {
                value,
                delta_per_sec,
                ..
            } = value
            {
                if let Some(MetricValue::Counter { value: prev, .. }) = prev.items.get(name) {
                    if let Some(delta) = value.checked_sub(*prev) {
                        *delta_per_sec = Some(delta as f64 / duration.as_secs_f64());
                    }
                }
            }
        }
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
        delta_per_sec: Option<f64>,
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

    fn gauge_with_parent(value: u64, parent: &str) -> Self {
        Self::Gauge {
            value,
            parent: Some(parent.to_owned()),
        }
    }

    fn counter(value: u64) -> Self {
        Self::Counter {
            value,
            delta_per_sec: None,
            parent: None,
        }
    }

    fn counter_with_parent(value: u64, parent: &str) -> Self {
        Self::Counter {
            value,
            delta_per_sec: None,
            parent: Some(parent.to_owned()),
        }
    }

    // TODO: rename
    pub fn value(&self) -> Option<u64> {
        match self {
            Self::Gauge { value, .. } => Some(*value),
            Self::Counter {
                delta_per_sec: Some(v),
                ..
            } => Some(v.round() as u64),
            Self::Counter { .. } => None,
        }
    }

    pub fn parent(&self) -> Option<&str> {
        match self {
            Self::Gauge { parent, .. } => parent.as_ref().map(|x| x.as_str()),
            Self::Counter { parent, .. } => parent.as_ref().map(|x| x.as_str()),
        }
    }

    pub fn is_counter(&self) -> bool {
        matches!(self, Self::Counter { .. })
    }
}

impl std::fmt::Display for MetricValue {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if let Some(v) = self.value() {
            write!(f, "{}", format_u64(v, self.is_counter()))
        } else {
            write!(f, "")
        }
    }
}

pub fn format_u64(mut n: u64, is_delta: bool) -> String {
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
    let mut s = String::from_utf8(s).expect("unreachable");
    if is_delta {
        s.push_str("/s");
    } else {
        s.push_str("  ");
    }
    s
}

#[derive(Debug, Clone)]
pub struct Msacc {}

#[derive(Debug)]
pub struct MetricsPoller {
    options: Options,
    rpc_client: RpcClient,
    tx: MetricsSender,
    prev_metrics: Metrics,
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
            prev_metrics: Metrics::new(),
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
        metrics.insert("system_info.processe_count", MetricValue::gauge(processes));

        let ports = self.rpc_client.get_system_info_u64("port_count").await?;
        metrics.insert("system_info.port_count", MetricValue::gauge(ports));

        let atoms = self.rpc_client.get_system_info_u64("atom_count").await?;
        metrics.insert("system_info.atom_count", MetricValue::gauge(atoms));

        let ets_tables = self.rpc_client.get_system_info_u64("ets_count").await?;
        metrics.insert("system_info.ets_count", MetricValue::gauge(ets_tables));

        let context_switches = self
            .rpc_client
            .get_statistics_1st_u64("context_switches")
            .await?;
        metrics.insert(
            "statistics.context_switches",
            MetricValue::counter(context_switches),
        );

        let exact_reductions = self
            .rpc_client
            .get_statistics_1st_u64("exact_reductions")
            .await?;
        metrics.insert(
            "statistics.exact_reductions",
            MetricValue::counter(exact_reductions),
        );

        let garbage_collection = self
            .rpc_client
            .get_statistics_1st_u64("garbage_collection")
            .await?;
        metrics.insert(
            "statistics.garbage_collection",
            MetricValue::counter(garbage_collection),
        );

        let runtime = self.rpc_client.get_statistics_1st_u64("runtime").await?;
        metrics.insert("statistics.runtime", MetricValue::counter(runtime));

        let (in_bytes, out_bytes) = self.rpc_client.get_statistics_io().await?;
        metrics.insert(
            "statistics.io.total_bytes",
            MetricValue::counter(in_bytes + out_bytes),
        );
        metrics.insert(
            "statistics.io.input_bytes",
            MetricValue::counter_with_parent(in_bytes, "statistics.io.total_bytes"),
        );
        metrics.insert(
            "statistics.io.output_bytes",
            MetricValue::counter_with_parent(out_bytes, "statistics.io.total_bytes"),
        );

        let run_queue_lengths = self
            .rpc_client
            .get_statistics_u64_list("run_queue_lengths_all")
            .await?;
        let run_queue_total = run_queue_lengths.iter().copied().sum();
        metrics.insert("statistics.run_queue", MetricValue::gauge(run_queue_total));

        for (i, n) in run_queue_lengths.into_iter().enumerate() {
            metrics.insert(
                &format!("statistics.run_queue.{}", i),
                MetricValue::gauge_with_parent(n, "statistics.run_queue"),
            );
        }

        let mut memory = self.rpc_client.get_memory().await?;
        metrics.insert(
            "memory.total_bytes",
            MetricValue::gauge(memory.remove("total").expect("unreachable")),
        );
        for (k, v) in memory {
            metrics.insert(
                &format!("memory.{k}_bytes"),
                MetricValue::gauge_with_parent(v, "memory.total_bytes"),
            );
        }

        // pub microstate_accounting: Msacc

        log::debug!(
            "MetricsPoller::poll_once(): elapsed={:?}",
            metrics.timestamp.elapsed()
        );
        metrics.calc_delta(&self.prev_metrics);

        self.prev_metrics = metrics.clone();

        Ok(metrics)
    }
}
