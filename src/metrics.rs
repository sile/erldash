use crate::erlang::RpcClient;
use crate::Options;
use std::sync::mpsc;
use std::time::{Instant, SystemTime};

pub type MetricsReceiver = mpsc::Receiver<Metrics>;
pub type MetricsSender = mpsc::Sender<Metrics>;

#[derive(Debug, Clone)]
pub struct Metrics {
    pub time: SystemTime,
    pub timestamp: Instant,
    pub stats: StatsMetrics,
    pub memory: MemoryMetrics,
    pub utilization: ThreadUtilizationMetrics,
}

#[derive(Debug, Clone)]
pub struct Gauge {
    pub value: u64,
}

#[derive(Debug, Clone)]
pub struct Counter {
    pub value: u64,
    pub delta_per_sec: Option<u64>,
}

// erlang:system_flag(scheduler_wall_time, true).

#[derive(Debug, Clone)]
pub struct SystemInfo {
    // erlang:system_info(schedulers)
    pub schedulers: u64,

    //  erlang:system_info(dirty_cpu_schedulers)
    pub dirty_cpu_schedulers: u64,

    // erlang:system_flag(scheduler_wall_time, true).
    pub dirty_io_schedulers: u64,
}

#[derive(Debug, Clone)]
pub struct StatsMetrics {
    pub processes: Gauge,
    pub ports: Gauge,
    pub context_switches: Counter,
    pub exact_reductions: Counter,
    pub io_input_bytes: Counter,
    pub io_output_bytes: Counter,
    pub run_queue_lengths: Vec<Gauge>,
    pub garbage_collection: u64,
    pub runtime: Counter,
    pub microstate_accounting: Msacc,
}

#[derive(Debug, Clone)]
pub struct Msacc {}

#[derive(Debug, Clone)]
pub struct MemoryMetrics {}

#[derive(Debug, Clone)]
pub struct ThreadUtilizationMetrics {}

#[derive(Debug)]
pub struct MetricsPoller {
    options: Options,
    rpc_client: RpcClient,
    tx: MetricsSender,
}

impl MetricsPoller {
    pub fn start_thread(options: Options) -> anyhow::Result<MetricsReceiver> {
        let (tx, rx) = mpsc::channel();

        let rpc_client = smol::block_on(async {
            let cookie = options.find_cookie()?;
            let client = RpcClient::connect(&options.erlang_node, &cookie).await?;
            Ok(client) as anyhow::Result<_>
        })?;

        let poller = Self {
            options,
            rpc_client,
            tx,
        };
        std::thread::spawn(|| poller.run());
        Ok(rx)
    }

    fn run(self) {}
}
