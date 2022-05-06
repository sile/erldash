use crate::erlang::{MSAccThread, RpcClient, SystemVersion};
use crate::Options;
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

type MetricsReceiver = mpsc::Receiver<Metrics>;
type MetricsSender = mpsc::Sender<Metrics>;

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
                raw_value, value, ..
            } = value
            {
                if let Some(MetricValue::Counter {
                    raw_value: prev, ..
                }) = prev.items.get(name)
                {
                    if let Some(delta) = raw_value.checked_sub(*prev) {
                        *value = Some(delta as f64 / duration.as_secs_f64());
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
        raw_value: u64,
        value: Option<f64>, // delta per second
        parent: Option<String>,
    },
    Utilization {
        value: f64,
        parent: Option<String>,
    },
}

impl MetricValue {
    pub fn utilization(value: f64) -> Self {
        Self::Utilization {
            value,
            parent: None,
        }
    }

    fn utilization_with_parent(value: f64, parent: &str) -> Self {
        Self::Utilization {
            value,
            parent: Some(parent.to_owned()),
        }
    }

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

    fn counter(raw_value: u64) -> Self {
        Self::Counter {
            raw_value,
            value: None,
            parent: None,
        }
    }

    fn counter_with_parent(raw_value: u64, parent: &str) -> Self {
        Self::Counter {
            raw_value,
            value: None,
            parent: Some(parent.to_owned()),
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Gauge { value, .. } => Some(*value as f64),
            Self::Counter { value: Some(v), .. } => Some(v.round()),
            Self::Counter { .. } => None,
            Self::Utilization { value, .. } => Some(*value),
        }
    }

    fn parent(&self) -> Option<&str> {
        match self {
            Self::Gauge { parent, .. } => parent.as_ref().map(|x| x.as_str()),
            Self::Counter { parent, .. } => parent.as_ref().map(|x| x.as_str()),
            Self::Utilization { parent, .. } => parent.as_ref().map(|x| x.as_str()),
        }
    }
}

impl std::fmt::Display for MetricValue {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Gauge { value, .. } => {
                write!(f, "{}", format_u64(*value, "  "))
            }
            Self::Utilization { value, .. } => {
                write!(f, "{:.1} %", value)
            }
            Self::Counter {
                value: Some(value), ..
            } => {
                write!(f, "{}", format_u64(value.round() as u64, "/s"))
            }
            Self::Counter { .. } => {
                write!(f, "")
            }
        }
    }
}

impl std::ops::AddAssign for MetricValue {
    fn add_assign(&mut self, rhs: Self) {
        match (self, rhs) {
            (Self::Gauge { value, .. }, Self::Gauge { value: rhs, .. }) => {
                *value += rhs;
            }
            (Self::Utilization { value, .. }, Self::Utilization { value: rhs, .. }) => {
                *value += rhs;
            }
            (Self::Counter { value: lhs, .. }, Self::Counter { value: rhs, .. }) => {
                if let (Some(lhs), Some(rhs)) = (lhs.as_mut(), rhs) {
                    *lhs += rhs;
                } else {
                    *lhs = rhs;
                }
            }
            (lhs, rhs) => {
                panic!("cannot apply `MetricValue::add_assign()` to {lhs:?} and {rhs:?}",);
            }
        }
    }
}

impl std::ops::SubAssign for MetricValue {
    fn sub_assign(&mut self, rhs: Self) {
        match (self, rhs) {
            (Self::Gauge { value, .. }, Self::Gauge { value: rhs, .. }) => {
                *value -= rhs;
            }
            (Self::Utilization { value, .. }, Self::Utilization { value: rhs, .. }) => {
                *value -= rhs;
            }
            (Self::Counter { value: lhs, .. }, Self::Counter { value: rhs, .. }) => {
                if let (Some(lhs), Some(rhs)) = (lhs.as_mut(), rhs) {
                    *lhs -= rhs;
                }
            }
            (lhs, rhs) => {
                panic!("cannot apply `MetricValue::sub_assign()` to {lhs:?} and {rhs:?}",);
            }
        }
    }
}

pub fn format_u64(mut n: u64, suffix: &str) -> String {
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
    s.push_str(suffix);
    s
}

#[derive(Debug)]
pub struct MetricsPoller {
    pub rx: MetricsReceiver,
    pub system_version: SystemVersion,
    rpc_client: RpcClient,
    old_microstate_accounting_flag: bool,
}

impl MetricsPoller {
    pub fn start_thread(options: Options) -> anyhow::Result<Self> {
        MetricsPollerThread::start_thread(options)
    }
}

impl Drop for MetricsPoller {
    fn drop(&mut self) {
        if !self.old_microstate_accounting_flag {
            if let Err(e) = smol::block_on(
                self.rpc_client
                    .set_system_flag_bool("microstate_accounting", "false"),
            ) {
                log::warn!("faild to disable microstate accounting: {e}");
            } else {
                log::debug!("disabled microstate accounting");
            }
        }
    }
}

#[derive(Debug)]
struct MetricsPollerThread {
    options: Options,
    rpc_client: RpcClient,
    tx: MetricsSender,
    prev_metrics: Metrics,
}

impl MetricsPollerThread {
    fn start_thread(options: Options) -> anyhow::Result<MetricsPoller> {
        let (tx, rx) = mpsc::channel();

        let rpc_client: RpcClient = smol::block_on(async {
            let cookie = options.find_cookie()?;
            let client = RpcClient::connect(&options.erlang_node, &cookie).await?;
            Ok(client) as anyhow::Result<_>
        })?;
        let system_version = smol::block_on(rpc_client.get_system_version())?;
        let old_microstate_accounting_flag =
            smol::block_on(rpc_client.set_system_flag_bool("microstate_accounting", "true"))?;
        log::debug!(
            "enabled microstate accounting (old flag state is {old_microstate_accounting_flag})"
        );

        let poller = MetricsPoller {
            rx,
            system_version,
            rpc_client: rpc_client.clone(),
            old_microstate_accounting_flag,
        };

        std::thread::spawn(|| {
            Self {
                options,
                rpc_client,
                tx,
                prev_metrics: Metrics::new(),
            }
            .run()
        });
        Ok(poller)
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

    fn insert_msacc_metrics(&self, metrics: &mut Metrics, msacc_threads: &[MSAccThread]) {
        let mut aggregated_per_type = BTreeMap::<_, ThreadTime>::new();
        let mut aggregated_per_state_per_type = BTreeMap::<_, BTreeMap<&str, u64>>::new();
        let mut aggregated_per_thread_per_type = BTreeMap::<_, BTreeMap<u64, ThreadTime>>::new();

        for thread in msacc_threads {
            let x = aggregated_per_type.entry(&thread.thread_type).or_default();
            let realtime = thread.counters.values().copied().sum::<u64>();
            let sleeptime = thread.counters["sleep"];
            x.realtime += realtime;
            x.runtime += realtime - sleeptime;

            let x = aggregated_per_thread_per_type
                .entry(&thread.thread_type)
                .or_default()
                .entry(thread.thread_id)
                .or_default();
            x.realtime += realtime;
            x.runtime += realtime - sleeptime;

            for (state, value) in &thread.counters {
                *aggregated_per_state_per_type
                    .entry(&thread.thread_type)
                    .or_default()
                    .entry(state)
                    .or_default() += *value;
            }
        }
        for (ty, time) in aggregated_per_type {
            let root_name = format!("utilization.{ty}");
            metrics.insert(&root_name, MetricValue::utilization(time.utilization()));
            for (state, value) in &aggregated_per_state_per_type[ty] {
                let u = *value as f64 / time.realtime as f64 * 100.0;
                metrics.insert(
                    &format!("{root_name}.state.{state}"),
                    MetricValue::utilization_with_parent(u, &root_name),
                );
            }

            let id_width = aggregated_per_thread_per_type[ty]
                .keys()
                .map(|id| id / 10 + 1)
                .max()
                .unwrap_or(1) as usize;
            for (thread_id, time) in &aggregated_per_thread_per_type[ty] {
                metrics.insert(
                    &format!("{root_name}.thread.{:0id_width$}", thread_id),
                    MetricValue::utilization_with_parent(time.utilization(), &root_name),
                );
            }
        }
    }

    async fn poll_once(&mut self) -> anyhow::Result<Metrics> {
        let mut metrics = Metrics::new();

        let msacc = self
            .rpc_client
            .get_statistics_microstate_accounting()
            .await?;
        self.insert_msacc_metrics(&mut metrics, &msacc);

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

        let width = run_queue_lengths.len() / 10 + 1;
        for (i, n) in run_queue_lengths.into_iter().enumerate() {
            metrics.insert(
                &format!("statistics.run_queue.{:0width$}", i),
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

        self.rpc_client
            .set_system_flag_bool("microstate_accounting", "reset")
            .await?;

        log::debug!(
            "MetricsPoller::poll_once(): elapsed={:?}",
            metrics.timestamp.elapsed()
        );
        metrics.calc_delta(&self.prev_metrics);

        self.prev_metrics = metrics.clone();

        Ok(metrics)
    }
}

#[derive(Debug, Default)]
struct ThreadTime {
    runtime: u64,
    realtime: u64,
}

impl ThreadTime {
    fn utilization(&self) -> f64 {
        self.runtime as f64 / self.realtime as f64 * 100.0
    }
}
