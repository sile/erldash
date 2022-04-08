use erl_dist::term::{Atom, FixInteger, List, Map, Term};
use erl_rpc::RpcClientHandle;
use std::collections::BTreeMap;
use std::time::Duration;

pub(crate) async fn get_msacc_stats(
    mut handle: RpcClientHandle,
    duration: Duration,
) -> anyhow::Result<MsaccData> {
    handle
        .call(
            "msacc".into(),
            "start".into(),
            List::from(vec![FixInteger::from(duration.as_millis() as i32).into()]),
        )
        .await?;

    let stats = handle
        .call("msacc".into(), "stats".into(), List::nil())
        .await?;
    MsaccData::new(stats, duration)
}

const THREAD_STATE_SLEEP: &str = "sleep";

pub type MsaccThreadId = i32;
pub type MsaccThreadType = String;
pub type MsaccThreadState = String;

#[derive(Debug, Clone)]
pub struct MsaccData {
    pub threads: Vec<MsaccDataThread>,
    pub duration: Duration,
}

impl MsaccData {
    fn new(term: Term, duration: Duration) -> anyhow::Result<Self> {
        let list: List = term.try_into().map_err(|_| anyhow::anyhow!("not a list"))?;
        let mut threads = Vec::new();
        for msacc_data_thread in list.elements {
            threads.push(MsaccDataThread::new(msacc_data_thread)?);
        }
        Ok(Self { threads, duration })
    }

    pub fn get_utilization_per_type(&self) -> BTreeMap<MsaccThreadType, f64> {
        let mut aggregated = BTreeMap::<_, Time>::new();
        for thread in &self.threads {
            let x = aggregated.entry(&thread.thread_type).or_default();
            let realtime = thread.counters.values().copied().sum();
            x.realtime += realtime;
            x.runtime += realtime - thread.sleep_time();
            x.count += 1;
        }
        aggregated
            .into_iter()
            .map(|(k, v)| (format!("{}({})", k, v.count), v.utilization()))
            .collect()
    }

    pub fn get_utilization_per_state(&self) -> BTreeMap<MsaccThreadState, f64> {
        let mut aggregated = BTreeMap::<_, Duration>::new();
        let mut total = Duration::default();
        for thread in &self.threads {
            for (state, c) in thread
                .counters
                .iter()
                .filter(|(state, _)| state.as_str() != THREAD_STATE_SLEEP)
            {
                *aggregated.entry(state).or_default() += *c;
                total += *c;
            }
        }
        aggregated
            .into_iter()
            .map(|(state, c)| {
                let k = state.to_owned();
                let v = c.as_secs_f64() / total.as_secs_f64() * 100.0;
                (k, v)
            })
            .collect()
    }
}

// TODO: rename
#[derive(Debug, Default)]
struct Time {
    runtime: Duration,
    realtime: Duration,
    count: usize,
}

impl Time {
    fn utilization(&self) -> f64 {
        self.runtime.as_secs_f64() / self.realtime.as_secs_f64() * 100.0
    }
}

#[derive(Debug, Clone)]
pub struct MsaccDataThread {
    pub thread_id: MsaccThreadId,
    pub thread_type: MsaccThreadType,
    pub counters: BTreeMap<MsaccThreadState, Duration>,
}

impl MsaccDataThread {
    fn new(term: Term) -> anyhow::Result<Self> {
        let mut map: Map = term.try_into().map_err(|_| anyhow::anyhow!("not a map"))?;
        let id: FixInteger = remove_map_entry(&mut map, "id")?;
        let ty: Atom = remove_map_entry(&mut map, "type")?;
        let mut counters = BTreeMap::new();
        for (k, v) in remove_map_entry::<Map>(&mut map, "counters")?.entries {
            let k: Atom = k.try_into().map_err(|_| anyhow::anyhow!("not an atom"))?;

            let v = match v {
                Term::FixInteger(v) => v.value.try_into()?,
                Term::BigInteger(v) => v.value.try_into()?,
                v => anyhow::bail!("{} is not an integer", v),
            };
            let v = Duration::from_micros(v);

            counters.insert(k.name, v);
        }

        Ok(Self {
            thread_id: id.value,
            thread_type: ty.name,
            counters,
        })
    }

    fn sleep_time(&self) -> Duration {
        self.counters
            .get(THREAD_STATE_SLEEP)
            .copied()
            .unwrap_or_default()
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
