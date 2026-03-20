use crate::erlang::{MSAccThread, RpcClient, SystemVersion};
use crate::error::{self, Context};
use crate::{Command, ReplayArgs, RunArgs};
use nojson::DisplayJson;
use smol::fs::File;
use smol::io::AsyncWriteExt;
use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::mpsc;
use std::time::{Duration, Instant};

type MetricsReceiver = mpsc::Receiver<Metrics>;
type MetricsSender = mpsc::Sender<Metrics>;

#[derive(Debug, Clone)]
pub struct Metrics {
    pub timestamp: Duration,
    pub items: BTreeMap<String, MetricValue>,
}

impl DisplayJson for Metrics {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            let ts = &self.timestamp;
            f.member(
                "timestamp",
                nojson::object(|f| {
                    f.member("secs", ts.as_secs())?;
                    f.member("nanos", ts.subsec_nanos())
                }),
            )?;
            f.member("items", &self.items)
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for Metrics {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let ts = value.to_member("timestamp")?.required()?;
        let secs: u64 = ts.to_member("secs")?.required()?.try_into()?;
        let nanos: u32 = ts.to_member("nanos")?.required()?.try_into()?;
        let items: BTreeMap<String, MetricValue> =
            value.to_member("items")?.required()?.try_into()?;
        Ok(Metrics {
            timestamp: Duration::new(secs, nanos),
            items,
        })
    }
}

impl Metrics {
    fn new(start: Instant) -> Self {
        Self {
            timestamp: start.elapsed(),
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
            .filter(move |(_, v)| v.parent().as_ref().is_some_and(|&x| x == parent))
            .map(|(k, v)| (k.as_str(), v))
    }

    fn calc_delta(&mut self, prev: &Self) {
        let duration = self.timestamp - prev.timestamp;
        for (name, value) in &mut self.items {
            if let MetricValue::Counter {
                raw_value, value, ..
            } = value
                && let Some(MetricValue::Counter {
                    raw_value: prev, ..
                }) = prev.items.get(name)
                && let Some(delta) = raw_value.checked_sub(*prev)
            {
                *value = Some(delta as f64 / duration.as_secs_f64());
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

impl DisplayJson for MetricValue {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| match self {
            MetricValue::Gauge { value, parent } => {
                let (value, parent) = (*value, parent);
                f.member(
                    "Gauge",
                    nojson::object(move |f| {
                        f.member("value", value)?;
                        f.member("parent", parent)
                    }),
                )
            }
            MetricValue::Counter {
                raw_value,
                value,
                parent,
            } => {
                let (raw_value, value, parent) = (*raw_value, value, parent);
                f.member(
                    "Counter",
                    nojson::object(move |f| {
                        f.member("raw_value", raw_value)?;
                        f.member("value", value)?;
                        f.member("parent", parent)
                    }),
                )
            }
            MetricValue::Utilization { value, parent } => {
                let (value, parent) = (*value, parent);
                f.member(
                    "Utilization",
                    nojson::object(move |f| {
                        f.member("value", value)?;
                        f.member("parent", parent)
                    }),
                )
            }
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for MetricValue {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        if let Some(inner) = value.to_member("Gauge")?.optional() {
            Ok(MetricValue::Gauge {
                value: inner.to_member("value")?.required()?.try_into()?,
                parent: inner.to_member("parent")?.required()?.try_into()?,
            })
        } else if let Some(inner) = value.to_member("Counter")?.optional() {
            Ok(MetricValue::Counter {
                raw_value: inner.to_member("raw_value")?.required()?.try_into()?,
                value: inner.to_member("value")?.required()?.try_into()?,
                parent: inner.to_member("parent")?.required()?.try_into()?,
            })
        } else if let Some(inner) = value.to_member("Utilization")?.optional() {
            Ok(MetricValue::Utilization {
                value: inner.to_member("value")?.required()?.try_into()?,
                parent: inner.to_member("parent")?.required()?.try_into()?,
            })
        } else {
            Err(value.invalid("expected a MetricValue (Gauge, Counter, or Utilization)"))
        }
    }
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
pub enum MetricsPoller {
    Realtime(RealtimeMetricsPoller),
    Replay(ReplayMetricsPoller),
}

impl MetricsPoller {
    pub fn start_thread(command: Command) -> error::Result<Self> {
        match command {
            Command::Run(args) => RealtimeMetricsPoller::start_thread(args).map(Self::Realtime),
            Command::Replay(args) => ReplayMetricsPoller::new(args).map(Self::Replay),
        }
    }

    pub fn is_replay(&self) -> bool {
        matches!(self, Self::Replay(_))
    }

    pub fn header(&self) -> &Header {
        match self {
            Self::Realtime(poller) => &poller.header,
            Self::Replay(poller) => &poller.header,
        }
    }

    pub fn poll_metrics(&self, timeout: Duration) -> Result<Metrics, mpsc::RecvTimeoutError> {
        match self {
            Self::Realtime(poller) => poller.rx.recv_timeout(timeout),
            Self::Replay(_) => {
                unreachable!()
            }
        }
    }

    pub fn replay_last_time(&self) -> Duration {
        match self {
            Self::Realtime(_) => Duration::from_secs(0),
            Self::Replay(poller) => poller
                .metrics_log
                .last()
                .map(|m| m.timestamp)
                .unwrap_or_default(),
        }
    }

    pub fn get_metrics_range(
        &self,
        start_time: Duration,
        end_time: Duration,
    ) -> error::Result<impl '_ + Iterator<Item = &Metrics>> {
        let Self::Replay(poller) = self else {
            return Err(error::Error::new(
                "`get_metrics_range()` is only available in replay mode",
            ));
        };
        Ok(poller.metrics_log.iter().filter(move |metrics| {
            let time = metrics.timestamp;
            start_time <= time && time <= end_time
        }))
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub system_version: SystemVersion,
    pub node_name: String,
    pub start_time: chrono::DateTime<chrono::Local>,
}

impl DisplayJson for Header {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("system_version", &self.system_version)?;
            f.member("node_name", &self.node_name)?;
            f.member("start_time", self.start_time.to_rfc3339())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for Header {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let system_version = value.to_member("system_version")?.required()?.try_into()?;
        let node_name: String = value.to_member("node_name")?.required()?.try_into()?;
        let start_time_str: String = value.to_member("start_time")?.required()?.try_into()?;
        let start_time = chrono::DateTime::parse_from_rfc3339(&start_time_str)
            .map_err(|e| value.invalid(e))?
            .with_timezone(&chrono::Local);
        Ok(Header {
            system_version,
            node_name,
            start_time,
        })
    }
}

#[derive(Debug)]
pub struct ReplayMetricsPoller {
    header: Header,
    metrics_log: Vec<Metrics>,
}

impl ReplayMetricsPoller {
    fn new(args: ReplayArgs) -> error::Result<Self> {
        let record_file_path = args.file;
        let file = std::fs::File::open(&record_file_path).with_context(|| {
            format!("failed to open record file: {}", record_file_path.display())
        })?;
        let reader = std::io::BufReader::new(file);

        let mut header = None;
        let mut metrics_log = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            if i == 0 {
                header = Some(
                    line.parse::<nojson::Json<Header>>()
                        .map(|j| j.0)
                        .with_context(|| format!("failed to parse record file: line={}", i + 1))?,
                );
                continue;
            }
            let metrics = line
                .parse::<nojson::Json<Metrics>>()
                .map(|j| j.0)
                .with_context(|| format!("failed to parse record file: line={}", i + 1))?;
            metrics_log.push(metrics);
        }
        let header = header.ok_or_else(|| error::Error::new("record file is empty"))?;
        Ok(Self {
            header,
            metrics_log,
        })
    }
}

#[derive(Debug)]
pub struct RealtimeMetricsPoller {
    rx: MetricsReceiver,
    header: Header,
    rpc_client: RpcClient,
    old_microstate_accounting_flag: bool,
}

impl RealtimeMetricsPoller {
    fn start_thread(args: RunArgs) -> error::Result<Self> {
        MetricsPollerThread::start_thread(args)
    }
}

impl Drop for RealtimeMetricsPoller {
    fn drop(&mut self) {
        if !self.old_microstate_accounting_flag {
            if let Err(e) = smol::block_on(
                self.rpc_client
                    .set_system_flag_bool("microstate_accounting", "false"),
            ) {
                log::warn!("faild to disable microstate accounting: {e:?}");
            } else {
                log::debug!("disabled microstate accounting");
            }
        }
    }
}

#[derive(Debug)]
struct MetricsPollerThread {
    args: RunArgs,
    rpc_client: RpcClient,
    tx: MetricsSender,
    prev_metrics: Metrics,
    start: Instant,
    header: Header,
    record_file: Option<File>,
}

impl MetricsPollerThread {
    fn start_thread(args: RunArgs) -> error::Result<RealtimeMetricsPoller> {
        let (tx, rx) = mpsc::channel();

        let rpc_client: RpcClient = smol::block_on(async {
            let cookie = args.find_cookie()?;
            let client = RpcClient::connect(&args.erlang_node, args.port, &cookie).await?;
            Ok(client) as error::Result<_>
        })?;
        let system_version = smol::block_on(rpc_client.get_system_version())?;
        let old_microstate_accounting_flag =
            smol::block_on(rpc_client.set_system_flag_bool("microstate_accounting", "true"))?;
        log::debug!(
            "enabled microstate accounting (old flag state is {old_microstate_accounting_flag})"
        );

        let header = Header {
            system_version: system_version.clone(),
            node_name: args.erlang_node.to_string(),
            start_time: chrono::Local::now(),
        };
        let poller = RealtimeMetricsPoller {
            rx,
            header: header.clone(),
            rpc_client: rpc_client.clone(),
            old_microstate_accounting_flag,
        };

        let record_file = if let Some(path) = &args.record {
            Some(File::from(std::fs::File::create(path).with_context(
                || format!("failed to record file {}", path.display()),
            )?))
        } else {
            None
        };

        std::thread::spawn(|| {
            let start = Instant::now();
            Self {
                args,
                rpc_client,
                tx,
                prev_metrics: Metrics::new(start),
                start,
                header,
                record_file,
            }
            .run()
        });
        Ok(poller)
    }

    async fn write_json_line(&mut self, value: &impl DisplayJson) -> error::Result<()> {
        if let Some(file) = &mut self.record_file {
            file.write_all(format!("{}\n", nojson::Json(value)).as_bytes())
                .await?;
            file.flush().await?;
        }
        Ok(())
    }

    fn run(mut self) {
        let interval = Duration::from_secs(self.args.polling_interval.get() as u64);
        let mut next_time = Duration::from_secs(0);
        smol::block_on(async {
            if let Err(e) = self.write_json_line(&self.header.clone()).await {
                log::error!("faild to write record file: {e:?}");
                return;
            }

            loop {
                match self.poll_once().await {
                    Err(e) => {
                        log::error!("faild to poll metrics: {e:?}");
                        break;
                    }
                    Ok(metrics) => {
                        let elapsed = metrics.timestamp;

                        if let Err(e) = self.write_json_line(&metrics).await {
                            log::error!("faild to write record file: {e:?}");
                            break;
                        }

                        if self.tx.send(metrics).is_err() {
                            log::debug!("the main thread has terminated");
                            break;
                        }

                        next_time += interval;
                        if let Some(sleep_duration) = next_time.checked_sub(elapsed) {
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

    async fn poll_once(&mut self) -> error::Result<Metrics> {
        let mut metrics = Metrics::new(self.start);

        let msacc = self
            .rpc_client
            .get_statistics_microstate_accounting()
            .await?;
        self.insert_msacc_metrics(&mut metrics, &msacc);

        let processes = self.rpc_client.get_system_info_u64("process_count").await?;
        metrics.insert("system_info.process_count", MetricValue::gauge(processes));

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
            metrics.timestamp
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

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T>(value: &T, expected_json: &str)
    where
        T: DisplayJson
            + for<'text, 'raw> TryFrom<
                nojson::RawJsonValue<'text, 'raw>,
                Error = nojson::JsonParseError,
            >,
        T: std::fmt::Debug,
    {
        let json = nojson::Json(value).to_string();
        assert_eq!(json, expected_json);
        let parsed: nojson::Json<T> = json.parse().unwrap();
        let re_json = nojson::Json(&parsed.0).to_string();
        assert_eq!(json, re_json);
    }

    #[test]
    fn metric_value_gauge() {
        roundtrip(
            &MetricValue::gauge(42),
            r#"{"Gauge":{"value":42,"parent":null}}"#,
        );
    }

    #[test]
    fn metric_value_gauge_with_parent() {
        roundtrip(
            &MetricValue::gauge_with_parent(10, "root"),
            r#"{"Gauge":{"value":10,"parent":"root"}}"#,
        );
    }

    #[test]
    fn metric_value_counter() {
        roundtrip(
            &MetricValue::counter(100),
            r#"{"Counter":{"raw_value":100,"value":null,"parent":null}}"#,
        );
    }

    #[test]
    fn metric_value_counter_with_value() {
        roundtrip(
            &MetricValue::Counter {
                raw_value: 300,
                value: Some(42.5),
                parent: None,
            },
            r#"{"Counter":{"raw_value":300,"value":42.5,"parent":null}}"#,
        );
    }

    #[test]
    fn metric_value_counter_with_parent() {
        roundtrip(
            &MetricValue::counter_with_parent(200, "stats"),
            r#"{"Counter":{"raw_value":200,"value":null,"parent":"stats"}}"#,
        );
    }

    #[test]
    fn metric_value_utilization() {
        roundtrip(
            &MetricValue::utilization(75.5),
            r#"{"Utilization":{"value":75.5,"parent":null}}"#,
        );
    }

    #[test]
    fn metric_value_utilization_with_parent() {
        roundtrip(
            &MetricValue::utilization_with_parent(50.0, "cpu"),
            r#"{"Utilization":{"value":50,"parent":"cpu"}}"#,
        );
    }

    #[test]
    fn metrics_roundtrip() {
        let mut metrics = Metrics {
            timestamp: Duration::new(10, 500_000_000),
            items: BTreeMap::new(),
        };
        metrics.insert("mem.total", MetricValue::gauge(1024));
        metrics.insert("cpu.util", MetricValue::utilization(85.3));
        roundtrip(
            &metrics,
            r#"{"timestamp":{"secs":10,"nanos":500000000},"items":{"cpu.util":{"Utilization":{"value":85.3,"parent":null}},"mem.total":{"Gauge":{"value":1024,"parent":null}}}}"#,
        );
    }

    #[test]
    fn header_roundtrip() {
        let start_time = chrono::DateTime::parse_from_rfc3339("2025-01-15T12:30:00+09:00")
            .unwrap()
            .with_timezone(&chrono::Local);
        let header = Header {
            system_version: r#""Erlang/OTP 26""#.parse::<nojson::Json<SystemVersion>>().unwrap().0,
            node_name: "test@localhost".to_string(),
            start_time,
        };
        let json = nojson::Json(&header).to_string();
        let parsed: nojson::Json<Header> = json.parse().unwrap();
        assert_eq!(parsed.0.system_version.get(), header.system_version.get());
        assert_eq!(parsed.0.node_name, header.node_name);
        assert_eq!(parsed.0.start_time, header.start_time);
    }
}
