use erl_dist::node::LocalNode;
use erl_dist::term::{Atom, List, Pid};
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
        local_node: LocalNode,
    ) -> anyhow::Result<Self> {
        //let _ = handle.call0("eprof", "stop").await?;

        // TODO: make sure to stop before returing this method.
        // TODO: handle {error, {alerady_started, _}}
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

        // TODO: use GROUP_LEADER control message(?)
        // let group_leader_pid =
        // Pid::new(local_node.name.to_string(), 1, 0, local_node.creation.get()); // TODO
        // handle
        //     .call2("erlang", "group_leader", group_leader_pid, eprof_pid)
        //     .await?;
        let result = handle
            .call1("eprof", "analyze", Atom::from("total"))
            .await?
            .try_into_atom()?;
        if result.name == "ok" {
            // TDOO
        }
        handle.call0("eprof", "stop").await?;

        Ok(Self {})
    }
}
