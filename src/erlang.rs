use erl_dist::node::NodeName;
use erl_dist::term::{Atom, List, Map, Term, Tuple};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemVersion(String);

impl SystemVersion {
    pub fn get(&self) -> &str {
        &self.0
    }
}

pub fn find_cookie() -> anyhow::Result<String> {
    if let Some(dir) = dirs::home_dir().filter(|dir| dir.join(".erlang.cookie").exists()) {
        let cookie = std::fs::read_to_string(dir.join(".erlang.cookie"))?;
        Ok(cookie)
    } else {
        anyhow::bail!("Could not find the cookie file $HOME/.erlang.cookie. Please specify `-cookie` arg instead.");
    }
}

#[derive(Debug, Clone)]
pub struct RpcClient {
    handle: erl_rpc::RpcClientHandle,
}

impl RpcClient {
    pub async fn connect(
        erlang_node: &NodeName,
        port: Option<u16>,
        cookie: &str,
    ) -> anyhow::Result<Self> {
        let client = if let Some(port) = port {
            erl_rpc::RpcClient::connect_with_port(&erlang_node.to_string(), port, cookie).await?
        } else {
            erl_rpc::RpcClient::connect(&erlang_node.to_string(), cookie).await?
        };
        let handle = client.handle();
        smol::spawn(async {
            if let Err(e) = client.run().await {
                log::error!("Erlang RPC Client error: {e}");
            }
        })
        .detach();

        Ok(Self { handle })
    }

    pub async fn get_system_version(&self) -> anyhow::Result<SystemVersion> {
        let term = self
            .handle
            .clone()
            .call(
                "erlang".into(),
                "system_info".into(),
                List::from(vec![Atom::from("system_version").into()]),
            )
            .await?;
        term_to_string(term).map(SystemVersion)
    }

    pub async fn get_system_info_u64(&self, item_name: &str) -> anyhow::Result<u64> {
        let term = self
            .handle
            .clone()
            .call(
                "erlang".into(),
                "system_info".into(),
                List::from(vec![Atom::from(item_name).into()]),
            )
            .await?;
        term_to_u64(term)
    }

    pub async fn get_statistics_1st_u64(&self, item_name: &str) -> anyhow::Result<u64> {
        let term = self.get_statistics(item_name).await?;
        term_to_tuple_1st_u64(term)
    }

    pub async fn get_statistics_u64_list(&self, item_name: &str) -> anyhow::Result<Vec<u64>> {
        let term = self.get_statistics(item_name).await?;
        term_to_u64_list(term)
    }

    pub async fn get_statistics_io(&self) -> anyhow::Result<(u64, u64)> {
        let term = self.get_statistics("io").await?;
        let tuple = term_to_tuple(term)?;
        let in_bytes = term_to_tuple_2nd_u64(tuple.elements[0].clone())?;
        let out_bytes = term_to_tuple_2nd_u64(tuple.elements[1].clone())?;
        Ok((in_bytes, out_bytes))
    }

    pub async fn get_statistics_microstate_accounting(&self) -> anyhow::Result<Vec<MSAccThread>> {
        let term = self.get_statistics("microstate_accounting").await?;
        term_to_list(term)?
            .elements
            .into_iter()
            .map(MSAccThread::from_term)
            .collect()
    }

    pub async fn set_system_flag_bool(&self, name: &str, value: &str) -> anyhow::Result<bool> {
        let term = self
            .handle
            .clone()
            .call(
                "erlang".into(),
                "system_flag".into(),
                List::from(vec![Atom::from(name).into(), Atom::from(value).into()]),
            )
            .await?;
        term_to_bool(term)
    }

    pub async fn get_memory(&self) -> anyhow::Result<BTreeMap<String, u64>> {
        let term = self
            .handle
            .clone()
            .call("erlang".into(), "memory".into(), List::nil())
            .await?;
        term_to_list(term)?
            .elements
            .into_iter()
            .map(|x| {
                let tuple = term_to_tuple(x)?;
                anyhow::ensure!(
                    tuple.elements.len() == 2,
                    "expected a two-elements tuple, but got {}",
                    tuple
                );
                let key = term_to_atom(tuple.elements[0].clone())?;
                let value = term_to_u64(tuple.elements[1].clone())?;
                Ok((key.name, value))
            })
            .collect()
    }

    async fn get_statistics(&self, item_name: &str) -> anyhow::Result<Term> {
        let term = self
            .handle
            .clone()
            .call(
                "erlang".into(),
                "statistics".into(),
                List::from(vec![Atom::from(item_name).into()]),
            )
            .await?;
        Ok(term)
    }
}

fn term_to_tuple_1st_u64(term: Term) -> anyhow::Result<u64> {
    let tuple = term_to_tuple(term)?;
    anyhow::ensure!(
        !tuple.elements.is_empty(),
        "expected a non empty tuple, but got {}",
        tuple
    );
    term_to_u64(tuple.elements[0].clone())
}

fn term_to_tuple_2nd_u64(term: Term) -> anyhow::Result<u64> {
    let tuple = term_to_tuple(term)?;
    anyhow::ensure!(
        tuple.elements.len() >= 2,
        "expected a tuple having 2 or more elements, but got {}",
        tuple
    );
    term_to_u64(tuple.elements[1].clone())
}

fn term_to_u64(term: Term) -> anyhow::Result<u64> {
    let v = match term {
        Term::FixInteger(v) => v.value.try_into()?,
        Term::BigInteger(v) => v.value.try_into()?,
        v => anyhow::bail!("{} is not an integer", v),
    };
    Ok(v)
}

fn term_to_string(term: Term) -> anyhow::Result<String> {
    let bytes = term_to_list(term)?
        .elements
        .into_iter()
        .map(term_to_u8)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(String::from_utf8(bytes)?)
}

fn term_to_u8(term: Term) -> anyhow::Result<u8> {
    if let Term::FixInteger(v) = term {
        Ok(u8::try_from(v.value)?)
    } else {
        anyhow::bail!("expected an integer, but got {}", term)
    }
}

fn term_to_u64_list(term: Term) -> anyhow::Result<Vec<u64>> {
    term_to_list(term)?
        .elements
        .into_iter()
        .map(term_to_u64)
        .collect()
}

fn term_to_bool(term: Term) -> anyhow::Result<bool> {
    let atom = term_to_atom(term)?;
    match atom.name.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => anyhow::bail!("expected 'true' or 'false', but got {}", atom.name),
    }
}

fn term_to_atom(term: Term) -> anyhow::Result<Atom> {
    term.try_into()
        .map_err(|x| anyhow::anyhow!("expected an atom, but got {x}"))
}

fn term_to_tuple(term: Term) -> anyhow::Result<Tuple> {
    term.try_into()
        .map_err(|x| anyhow::anyhow!("expected a tuple, but got {x}"))
}

fn term_to_list(term: Term) -> anyhow::Result<List> {
    term.try_into()
        .map_err(|x| anyhow::anyhow!("expected a list, but got {x}"))
}

#[derive(Debug, Clone)]
pub struct MSAccThread {
    pub thread_id: u64,
    pub thread_type: String,
    pub counters: BTreeMap<String, u64>,
}

impl MSAccThread {
    fn from_term(term: Term) -> anyhow::Result<Self> {
        let map: Map = term
            .try_into()
            .map_err(|x| anyhow::anyhow!("expected a map, but got {x}"))?;
        let mut thread_id = None;
        let mut thread_type = None;
        let mut counters = BTreeMap::new();
        for (k, v) in map.entries {
            match term_to_atom(k)?.name.as_str() {
                "id" => {
                    thread_id = Some(term_to_u64(v)?);
                }
                "type" => {
                    thread_type = Some(term_to_atom(v)?.name);
                }
                "counters" => {
                    let counters_map: Map = v
                        .try_into()
                        .map_err(|x| anyhow::anyhow!("expected a map, but got {x}"))?;
                    for (k, v) in counters_map.entries {
                        counters.insert(term_to_atom(k)?.name, term_to_u64(v)?);
                    }
                }
                k => {
                    log::debug!("unknown msacc key: {:?}", k);
                }
            }
        }
        Ok(Self {
            thread_id: thread_id.ok_or_else(|| anyhow::anyhow!("missing 'id' key"))?,
            thread_type: thread_type.ok_or_else(|| anyhow::anyhow!("missing 'type' key"))?,
            counters,
        })
    }
}
