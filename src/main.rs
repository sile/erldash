use clap::Parser;
use erl_dist::term::{Atom, FixInteger, List, Map, Term};
use std::collections::BTreeMap;

#[derive(Debug, Parser)]
struct Args {
    erlang_node: erl_dist::node::NodeName, // TODO: make this optional

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
        let data = msacc.get_stats().await?;
        println!("{:?}", data);

        Ok(())
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
}

impl Msacc {
    async fn start(mut rpc: erl_rpc::RpcClientHandle) -> anyhow::Result<Self> {
        let _already_started = rpc
            .call("msacc".into(), "start".into(), List::nil())
            .await?;
        dbg!(_already_started);
        Ok(Self { rpc })
    }

    async fn get_stats(&mut self) -> anyhow::Result<MsaccData> {
        let stats = self
            .rpc
            .call("msacc".into(), "stats".into(), List::nil())
            .await?;
        MsaccData::from_term(stats)
    }
}

impl Drop for Msacc {
    fn drop(&mut self) {
        let mut rpc = self.rpc.clone();
        smol::spawn(async move {
            let _ = rpc.call("msacc".into(), "stop".into(), List::nil()).await;
        })
        .detach();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[derive(Debug)]
struct MsaccDataThread {
    id: i32,
    ty: MsaccType,
    counters: BTreeMap<MsaccState, u64>,
}

impl MsaccDataThread {
    fn from_term(term: Term) -> anyhow::Result<Self> {
        let mut map: Map = term.try_into().map_err(|_| anyhow::anyhow!("not a map"))?;
        let id: FixInteger = remove_map_entry(&mut map, "id")?;

        let ty: Atom = remove_map_entry(&mut map, "type")?;
        let ty = match ty.name.as_str() {
            "scheduler" => MsaccType::Scheduler,
            "aux" => MsaccType::Aux,
            "async" => MsaccType::Async,
            "dirty_cpu_scheduler" => MsaccType::DsirtyCpuScheduler,
            "dirty_io_scheduler" => MsaccType::DsirtyIoScheduler,
            "poll" => MsaccType::Poll,
            ty => anyhow::bail!("unknown msacc type {:?}", ty),
        };

        let msacc_data_counters: Map = remove_map_entry(&mut map, "counters")?;
        let mut counters = BTreeMap::new();
        for (k, v) in msacc_data_counters.entries {
            let k: Atom = k.try_into().map_err(|_| anyhow::anyhow!("not an atom"))?;
            let k = match k.name.as_str() {
                "alloc" => MsaccState::Alloc,
                "aux" => MsaccState::Aux,
                "bif" => MsaccState::Bif,
                "busy_wait" => MsaccState::BusyWait,
                "check_io" => MsaccState::CheckIo,
                "emulator" => MsaccState::Emulator,
                "ets" => MsaccState::Ets,
                "gc" => MsaccState::Gc,
                "gc_fullsweep" => MsaccState::GcFullsweep,
                "nif" => MsaccState::Nif,
                "other" => MsaccState::Other,
                "port" => MsaccState::Port,
                "send" => MsaccState::Send,
                "sleep" => MsaccState::Sleep,
                "timers" => MsaccState::Timers,
                k => anyhow::bail!("unknown msacc state {:?}", k),
            };

            let v = match v {
                Term::FixInteger(v) => v.value.try_into()?,
                Term::BigInteger(v) => v.value.try_into()?,
                _ => anyhow::bail!("not an integer"),
            };

            counters.insert(k, v);
        }
        Ok(Self {
            id: id.value,
            ty,
            counters,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum MsaccState {
    Alloc,
    Aux,
    Bif,
    BusyWait,
    CheckIo,
    Emulator,
    Ets,
    Gc,
    GcFullsweep,
    Nif,
    Other,
    Port,
    Send,
    Sleep,
    Timers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MsaccType {
    Scheduler,
    Aux,
    Async,
    DsirtyCpuScheduler,
    DsirtyIoScheduler,
    Poll,
}

#[derive(Debug)]
struct MsaccData {
    pub threads: Vec<MsaccDataThread>,
}

impl MsaccData {
    fn from_term(term: Term) -> anyhow::Result<Self> {
        let list: List = term.try_into().map_err(|_| anyhow::anyhow!("not a list"))?;
        let mut threads = Vec::new();
        for msacc_data_thread in list.elements {
            threads.push(MsaccDataThread::from_term(msacc_data_thread)?);
        }
        Ok(Self { threads })
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
