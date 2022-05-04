use erl_dist::node::NodeName;
use erl_dist::term::{Atom, List, Term, Tuple};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
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
    pub async fn connect(erlang_node: &NodeName, cookie: &str) -> anyhow::Result<Self> {
        let client = erl_rpc::RpcClient::connect(&erlang_node.to_string(), cookie).await?;
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
        if let Term::Tuple(tuple) = term {
            let in_bytes = term_to_tuple_2nd_u64(tuple.elements[0].clone())?;
            let out_bytes = term_to_tuple_2nd_u64(tuple.elements[1].clone())?;
            Ok((in_bytes, out_bytes))
        } else {
            anyhow::bail!("{} is not a tuple", term);
        }
    }

    pub async fn set_system_flag_bool(&self, name: &str, value: bool) -> anyhow::Result<bool> {
        let term = self
            .handle
            .clone()
            .call(
                "erlang".into(),
                "system_flag".into(),
                List::from(vec![
                    Atom::from(name).into(),
                    Atom::from(value.to_string()).into(),
                ]),
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
        if let Term::List(list) = term {
            list.elements
                .into_iter()
                .map(|x| {
                    let tuple: Tuple = x
                        .try_into()
                        .map_err(|x| anyhow::anyhow!("expected a tuple, but got {}", x))?;
                    anyhow::ensure!(
                        tuple.elements.len() == 2,
                        "expected a two-elements tuple, but got {}",
                        tuple
                    );
                    let key: Atom = tuple.elements[0]
                        .clone()
                        .try_into()
                        .map_err(|x| anyhow::anyhow!("expected an atom, but got {}", x))?;
                    let value = term_to_u64(tuple.elements[1].clone())?;
                    Ok((key.name, value))
                })
                .collect()
        } else {
            anyhow::bail!("expected a list, but got {}", term);
        }
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
    if let Term::Tuple(tuple) = term {
        anyhow::ensure!(
            !tuple.elements.is_empty(),
            "expected a non empty tuple, but got {}",
            tuple
        );
        term_to_u64(tuple.elements[0].clone())
    } else {
        anyhow::bail!("{} is not a tuple", term)
    }
}

fn term_to_tuple_2nd_u64(term: Term) -> anyhow::Result<u64> {
    if let Term::Tuple(tuple) = term {
        anyhow::ensure!(
            tuple.elements.len() >= 2,
            "expected a tuple having 2 or more elements, but got {}",
            tuple
        );
        term_to_u64(tuple.elements[1].clone())
    } else {
        anyhow::bail!("{} is not a tuple", term)
    }
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
    if let Term::List(list) = term {
        let bytes = list
            .elements
            .into_iter()
            .map(term_to_u8)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(String::from_utf8(bytes)?)
    } else {
        anyhow::bail!("expected a string, but got {}", term);
    }
}

fn term_to_u8(term: Term) -> anyhow::Result<u8> {
    if let Term::FixInteger(v) = term {
        Ok(u8::try_from(v.value)?)
    } else {
        anyhow::bail!("expected an integer, but got {}", term)
    }
}

fn term_to_u64_list(term: Term) -> anyhow::Result<Vec<u64>> {
    if let Term::List(list) = term {
        list.elements.into_iter().map(term_to_u64).collect()
    } else {
        anyhow::bail!("expected a list, but got {}", term);
    }
}

fn term_to_bool(term: Term) -> anyhow::Result<bool> {
    let atom: Atom = term
        .try_into()
        .map_err(|x| anyhow::anyhow!("expected an atom, but got {}", x))?;
    match atom.name.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => anyhow::bail!("expected 'true' or 'false', but got {}", atom.name),
    }
}
