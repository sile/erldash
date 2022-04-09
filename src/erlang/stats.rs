use crate::erlang;
use erl_dist::term::{Atom, FixInteger, List, Term, Tuple};
// use std::time::Duration;

#[derive(Debug, Default)]
pub struct Stats {
    pub processes: u64,
    pub ports: u64,
    pub context_switches: u64,
    pub garbage_collection: u64,
    pub in_bytes: u64, // words?
    pub out_bytes: u64,
    pub run_queue: u64,
}

impl Stats {
    // TODO
    // pub fn to_gauge(&self, prev: &Self, prev_time: Duration, interval: Duration) -> Self {
    //     todo!()
    // }

    pub fn iter(&self) -> impl Iterator<Item = (&str, u64)> {
        [
            ("processes", self.processes),
            ("ports", self.ports),
            ("context-switches", self.context_switches),
            ("gc", self.garbage_collection),
            ("in", self.in_bytes),
            ("out", self.out_bytes),
            ("run_queue", self.run_queue),
        ]
        .into_iter()
    }

    pub async fn collect(handle: erl_rpc::RpcClientHandle) -> anyhow::Result<Self> {
        // TODO: Parallel call.
        let processes = Self::get_system_info_u64(handle.clone(), "process_count").await?;
        let ports = Self::get_system_info_u64(handle.clone(), "port_count").await?;
        let context_switches =
            Self::get_statistics_tuple_1st_u64(handle.clone(), "context_switches").await?;
        let garbage_collection =
            Self::get_statistics_tuple_1st_u64(handle.clone(), "garbage_collection").await?;
        let (in_bytes, out_bytes) = Self::get_statistics_io(handle.clone()).await?;
        let run_queue = Self::get_statistics_u64(handle.clone(), "run_queue").await?;
        Ok(Self {
            processes,
            ports,
            context_switches,
            garbage_collection,
            in_bytes,
            out_bytes,
            run_queue,
        })
    }

    async fn get_statistics(
        mut handle: erl_rpc::RpcClientHandle,
        key: &str,
    ) -> anyhow::Result<Term> {
        let term = handle
            .call(
                "erlang".into(),
                "statistics".into(),
                List::from(vec![Atom::from(key).into()]),
            )
            .await?;
        Ok(term)
    }

    async fn get_statistics_u64(
        handle: erl_rpc::RpcClientHandle,
        key: &str,
    ) -> anyhow::Result<u64> {
        erlang::term_to_u64(Self::get_statistics(handle, key).await?)
    }

    async fn get_statistics_tuple_1st_u64(
        handle: erl_rpc::RpcClientHandle,
        key: &str,
    ) -> anyhow::Result<u64> {
        let x: Tuple = Self::get_statistics(handle, key)
            .await?
            .try_into()
            .map_err(|e| anyhow::anyhow!("not a tuple: {}", e))?;
        erlang::term_to_u64(x.elements[0].clone())
    }

    async fn get_statistics_io(handle: erl_rpc::RpcClientHandle) -> anyhow::Result<(u64, u64)> {
        let x: Tuple = Self::get_statistics(handle, "io")
            .await?
            .try_into()
            .map_err(|e| anyhow::anyhow!("not a tuple: {}", e))?;
        let in_tuple: Tuple = x.elements[0]
            .clone() // TODO: length check
            .try_into()
            .map_err(|e| anyhow::anyhow!("not a tuple {e}"))?;
        let out_tuple: Tuple = x.elements[1]
            .clone() // TODO: length check
            .try_into()
            .map_err(|e| anyhow::anyhow!("not a tuple {e}"))?;
        Ok((
            erlang::term_to_u64(in_tuple.elements[1].clone())?,
            erlang::term_to_u64(out_tuple.elements[1].clone())?,
        ))
    }

    async fn get_system_info(
        mut handle: erl_rpc::RpcClientHandle,
        key: &str,
    ) -> anyhow::Result<Term> {
        let term = handle
            .call(
                "erlang".into(),
                "system_info".into(),
                List::from(vec![Atom::from(key).into()]),
            )
            .await?;
        Ok(term)
    }

    async fn get_system_info_u64(
        handle: erl_rpc::RpcClientHandle,
        key: &str,
    ) -> anyhow::Result<u64> {
        let x: FixInteger = Self::get_system_info(handle, key)
            .await?
            .try_into()
            .map_err(|e| anyhow::anyhow!("not an integer: {}", e))?;
        Ok(u64::try_from(x.value)?)
    }
}
