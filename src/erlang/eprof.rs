use erl_dist::term::{List, Pid};
use erl_rpc::{ConvertTerm, RpcClientHandle};
use std::time::Duration;

// TODO: rename
#[derive(Debug)]
pub struct Eprof {}

impl Eprof {
    pub async fn profile(
        mut handle: RpcClientHandle,
        pid: Pid,
        duration: Duration,
    ) -> anyhow::Result<Self> {
        let eprof_pid = handle
            .call0("eprof", "start")
            .await?
            .try_into_result()?
            .map_err(|e| anyhow::anyhow!("failed to start the eprof server: {e}"))?
            .try_into_pid()?;
        handle
            .call1("eprof", "start_profiling", List::from(vec![pid.into()]))
            .await?
            .expect_atom("profiling")?;
        smol::Timer::after(duration).await;
        handle
            .call0("eprof", "stop_profiling")
            .await?
            .expect_atom("profiling_stopped")?;
        // TODO: set gropu leader
        handle.call0("eprof", "analyze").await?.expect_atom("ok")?;
        handle
            .call0("eprof", "stop")
            .await?
            .expect_atom("stopped")?;
        Ok(Self {})
    }
}
