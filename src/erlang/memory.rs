use crate::erlang;
use erl_dist::term::{Atom, List, Tuple};
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone)]
pub struct MemoryStats {
    pub total: u64,     // processes + ssytem
    pub processes: u64, // processes_used + ProecssesNotUsed
    pub processes_used: u64,
    pub system: u64, // atom + binary + code + ets + OtherSystem
    pub atom: u64,   // atom_used + AtomNotUsed
    pub atom_used: u64,
    pub binary: u64,
    pub code: u64,
    pub ets: u64,
    pub unknowns: BTreeMap<String, u64>,
}

impl MemoryStats {
    pub fn iter(&self) -> impl Iterator<Item = (&str, u64)> {
        [
            ("total", self.total),
            ("processes", self.processes),
            ("processes_used", self.processes_used),
            ("system", self.system),
            ("atom", self.atom),
            ("atom_used", self.atom_used),
            ("binary", self.binary),
            ("code", self.code),
            ("ets", self.ets),
        ]
        .into_iter()
        .chain(self.unknowns.iter().map(|(k, v)| (k.as_str(), *v)))
    }
}

pub async fn get_memory_stats(mut handle: erl_rpc::RpcClientHandle) -> anyhow::Result<MemoryStats> {
    let term = handle
        .call(
            "erlang".into(),
            "memory".into(),
            erl_dist::term::List::nil(),
        )
        .await?;

    let mut stats = MemoryStats::default();
    let list: List = term.try_into().map_err(|_| anyhow::anyhow!("not a list"))?;
    for x in list.elements {
        let x: Tuple = x.try_into().map_err(|_| anyhow::anyhow!("not a tuple"))?;
        if x.elements.len() != 2 {
            anyhow::bail!("unexpected tuple size {}", x.elements.len());
        }

        let memory_type: Atom = x.elements[0]
            .clone()
            .try_into()
            .map_err(|_| anyhow::anyhow!("not an atom"))?;
        let bytes = erlang::term_to_u64(x.elements[1].clone())?;
        match memory_type.name.as_str() {
            "total" => stats.total = bytes,
            "processes" => stats.processes = bytes,
            "processes_used" => stats.processes_used = bytes,
            "system" => stats.system = bytes,
            "atom" => stats.atom = bytes,
            "atom_used" => stats.atom_used = bytes,
            "code" => stats.code = bytes,
            "binary" => stats.binary = bytes,
            "ets" => stats.ets = bytes,
            _ => {
                stats.unknowns.insert(memory_type.name, bytes);
            }
        }
    }

    Ok(stats)
}
