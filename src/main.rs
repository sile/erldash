use clap::Parser;
use erl_dist::term::{Atom, FixInteger, List, Map, Term};
use std::time::Duration;

#[derive(Debug, Parser)]
struct Args {
    erlang_node: erl_dist::node::NodeName, // TODO: make this optional

    #[clap(long, default_value_t = 1.0)]
    interval: f32,

    #[clap(long)]
    cookie: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let cookie = if let Some(cookie) = &args.cookie {
        cookie.clone()
    } else {
        find_cookie()?
    };
    let interval = Duration::from_secs_f32(args.interval);

    smol::block_on(async {
        let client = erl_rpc::RpcClient::connect(&args.erlang_node.to_string(), &cookie).await?;
        let handle = client.handle();
        smol::spawn(async {
            if let Err(e) = client.run().await {
                eprintln!("RpcClient Error: {}", e);
            }
        })
        .detach();

        let mut msacc = Msacc::start(handle).await?;
        loop {
            let data = msacc.get_stats(interval).await?;
            println!("System Realtime: {:?}", data.system_realtime());
            println!("System Runtime: {:?}", data.system_runtime());
            println!("Type Stats: {:?}", data.type_stats());
        }
    })
}

fn find_cookie() -> anyhow::Result<String> {
    if let Some(dir) = dirs::home_dir().filter(|dir| dir.join(".erlang.cookie").exists()) {
        let cookie = std::fs::read_to_string(dir.join(".erlang.cookie"))?;
        Ok(cookie)
    } else {
        anyhow::bail!("Could not find the cookie file $HOME/.erlang.cookie. Please specify `-cookie` arg instead.");
    }
}

#[derive(Debug)]
struct Msacc {
    rpc: erl_rpc::RpcClientHandle,
    time_unit: u32,
}

impl Msacc {
    async fn start(mut rpc: erl_rpc::RpcClientHandle) -> anyhow::Result<Self> {
        let time_unit: FixInteger = rpc
            .call(
                "erlang".into(),
                "convert_time_unit".into(),
                List::from(vec![
                    FixInteger::from(1).into(),
                    FixInteger::from(1).into(),
                    Atom::from("native").into(),
                ]),
            )
            .await?
            .try_into()
            .map_err(|_| anyhow::anyhow!("not an integer"))?;

        Ok(Self {
            rpc,
            time_unit: time_unit.value as u32,
        })
    }

    async fn get_stats(&mut self, interval: Duration) -> anyhow::Result<MsaccData> {
        self.rpc
            .call(
                "msacc".into(),
                "start".into(),
                List::from(vec![FixInteger::from(interval.as_millis() as i32).into()]),
            )
            .await?;

        let stats = self
            .rpc
            .call("msacc".into(), "stats".into(), List::nil())
            .await?;
        MsaccData::from_term(stats, self.time_unit)
    }
}

#[derive(Debug)]
struct MsaccDataThread {
    id: i32,
    ty: MsaccType,
    counters: MsaccCounters,
}

impl MsaccDataThread {
    fn from_term(term: Term, time_unit: u32) -> anyhow::Result<Self> {
        let mut map: Map = term.try_into().map_err(|_| anyhow::anyhow!("not a map"))?;
        let id: FixInteger = remove_map_entry(&mut map, "id")?;

        let ty: Atom = remove_map_entry(&mut map, "type")?;
        let ty = match ty.name.as_str() {
            "scheduler" => MsaccType::Scheduler,
            "aux" => MsaccType::Aux,
            "async" => MsaccType::Async,
            "dirty_cpu_scheduler" => MsaccType::DirtyCpuScheduler,
            "dirty_io_scheduler" => MsaccType::DirtyIoScheduler,
            "poll" => MsaccType::Poll,
            ty => anyhow::bail!("unknown msacc type {:?}", ty),
        };

        let msacc_data_counters: Map = remove_map_entry(&mut map, "counters")?;
        let mut counters = MsaccCounters::default();
        for (k, v) in msacc_data_counters.entries {
            let v: u64 = match v {
                Term::FixInteger(v) => v.value.try_into()?,
                Term::BigInteger(v) => v.value.try_into()?,
                _ => anyhow::bail!("not an integer"),
            };
            let v = Duration::from_secs_f64(v as f64 / time_unit as f64);

            let k: Atom = k.try_into().map_err(|_| anyhow::anyhow!("not an atom"))?;
            match k.name.as_str() {
                "alloc" => counters.alloc += v,
                "aux" => counters.aux += v,
                "bif" => counters.bif += v,
                "busy_wait" => counters.busy_wait += v,
                "check_io" => counters.check_io += v,
                "emulator" => counters.emulator += v,
                "ets" => counters.ets += v,
                "gc" => counters.gc += v,
                "gc_fullsweep" => counters.gc_fullsweep += v,
                "nif" => counters.nif += v,
                "other" => counters.other += v,
                "port" => counters.port += v,
                "send" => counters.send += v,
                "sleep" => counters.sleep += v,
                "timers" => counters.timers += v,
                k => anyhow::bail!("unknown msacc state {:?}", k),
            }
        }
        Ok(Self {
            id: id.value,
            ty,
            counters,
        })
    }
}

#[derive(Debug, Clone, Default)]
struct MsaccCounters {
    alloc: Duration,
    aux: Duration,
    bif: Duration,
    busy_wait: Duration,
    check_io: Duration,
    emulator: Duration,
    ets: Duration,
    gc: Duration,
    gc_fullsweep: Duration,
    nif: Duration,
    other: Duration,
    port: Duration,
    send: Duration,
    sleep: Duration,
    timers: Duration,
}

impl MsaccCounters {
    fn realtime_sum(&self) -> Duration {
        self.alloc
            + self.aux
            + self.bif
            + self.busy_wait
            + self.check_io
            + self.emulator
            + self.ets
            + self.gc
            + self.gc_fullsweep
            + self.nif
            + self.other
            + self.port
            + self.send
            + self.sleep
            + self.timers
    }

    fn runtime_sum(&self) -> Duration {
        self.alloc
            + self.aux
            + self.bif
            + self.busy_wait
            + self.check_io
            + self.emulator
            + self.ets
            + self.gc
            + self.gc_fullsweep
            + self.nif
            + self.other
            + self.port
            + self.send
            + self.timers
    }

    fn add_assign(&mut self, other: &Self) {
        self.alloc += other.alloc;
        self.aux += other.aux;
        self.bif += other.bif;
        self.busy_wait += other.busy_wait;
        self.check_io += other.check_io;
        self.emulator += other.emulator;
        self.ets += other.ets;
        self.gc += other.gc;
        self.gc_fullsweep += other.gc_fullsweep;
        self.nif += other.nif;
        self.other += other.other;
        self.port += other.port;
        self.send += other.send;
        self.sleep += other.sleep;
        self.timers += other.timers;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum MsaccType {
    Scheduler,
    Aux,
    Async,
    DirtyCpuScheduler,
    DirtyIoScheduler,
    Poll,
}

#[derive(Debug, Clone, Default)]
struct MsaccTypeStats {
    scheduler: MsaccCounters,
    aux: MsaccCounters,
    r#async: MsaccCounters,
    dirty_cpu_scheduler: MsaccCounters,
    dirty_io_scheduler: MsaccCounters,
    poll: MsaccCounters,
}

#[derive(Debug)]
struct MsaccData {
    pub threads: Vec<MsaccDataThread>,
}

impl MsaccData {
    fn from_term(term: Term, time_unit: u32) -> anyhow::Result<Self> {
        let list: List = term.try_into().map_err(|_| anyhow::anyhow!("not a list"))?;
        let mut threads = Vec::new();
        for msacc_data_thread in list.elements {
            threads.push(MsaccDataThread::from_term(msacc_data_thread, time_unit)?);
        }
        Ok(Self { threads })
    }

    fn system_realtime(&self) -> Duration {
        self.threads.iter().map(|t| t.counters.realtime_sum()).sum()
    }

    fn system_runtime(&self) -> Duration {
        self.threads.iter().map(|t| t.counters.runtime_sum()).sum()
    }

    fn type_stats(&self) -> MsaccTypeStats {
        let mut stats = MsaccTypeStats::default();
        for thread in &self.threads {
            match thread.ty {
                MsaccType::Scheduler => stats.scheduler.add_assign(&thread.counters),
                MsaccType::Async => stats.r#async.add_assign(&thread.counters),
                MsaccType::Aux => stats.aux.add_assign(&thread.counters),
                MsaccType::DirtyCpuScheduler => {
                    stats.dirty_cpu_scheduler.add_assign(&thread.counters)
                }
                MsaccType::DirtyIoScheduler => {
                    stats.dirty_io_scheduler.add_assign(&thread.counters)
                }
                MsaccType::Poll => stats.poll.add_assign(&thread.counters),
            }
        }
        stats
    }
}

fn remove_map_entry<T>(map: &mut Map, key: &str) -> anyhow::Result<T>
where
    Term: TryInto<T, Error = Term>,
{
    let pos = map
        .entries
        .iter()
        .position(|(k, _)| {
            if let Term::Atom(k) = k {
                k.name == key
            } else {
                false
            }
        })
        .ok_or_else(|| anyhow::anyhow!("no such key: {:?}", key))?;
    let (_, v) = map.entries.swap_remove(pos);
    v.try_into()
        .map_err(|t| anyhow::anyhow!("unexpected term type: {}", t))
}
