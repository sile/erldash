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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MsaccThreadId(pub i32);

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
}

#[derive(Debug, Clone)]
pub struct MsaccDataThread {
    pub thread_id: MsaccThreadId,
    pub thread_type: String,
    pub counters: BTreeMap<String, Duration>,
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
            thread_id: MsaccThreadId(id.value),
            thread_type: ty.name,
            counters,
        })
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
