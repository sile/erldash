use crate::erlang;
use erl_dist::term::{Atom, List, Pid, Term, Tuple};

#[derive(Debug)]
pub struct Top {
    pub pids: Vec<Pid>, // TODO: delete
    pub processes: Vec<Process>,
}

impl Top {
    pub async fn collect(handle: erl_rpc::RpcClientHandle) -> anyhow::Result<Self> {
        let pids = Self::get_pids(handle.clone()).await?;
        let mut processes = Vec::with_capacity(pids.len());
        for pid in &pids {
            if let Some(p) = Process::collect(handle.clone(), pid.clone()).await? {
                processes.push(p);
            }
        }
        processes.sort_by_key(|x| (x.reductions, x.message_queue_len, x.memory));
        processes.reverse();
        Ok(Self { pids, processes })
    }

    async fn get_pids(mut handle: erl_rpc::RpcClientHandle) -> anyhow::Result<Vec<Pid>> {
        let list: List = handle
            .call("erlang".into(), "processes".into(), List::nil())
            .await?
            .try_into()
            .map_err(|e| anyhow::anyhow!("not a list: {}", e))?;
        let mut pids = Vec::with_capacity(list.elements.len());
        for x in list.elements {
            let pid: Pid = x
                .try_into()
                .map_err(|e| anyhow::anyhow!("not a pid: {}", e))?;
            pids.push(pid);
        }
        Ok(pids)
    }
}

#[derive(Debug, Clone)]
pub struct Process {
    pub pid: Pid,
    pub message_queue_len: u64,
    pub memory: u64,
    pub reductions: u64,
    // pub status: String, // TODO: enum
}

impl Process {
    async fn collect(
        mut handle: erl_rpc::RpcClientHandle,
        pid: Pid,
    ) -> anyhow::Result<Option<Self>> {
        let term = handle
            .call(
                "erlang".into(),
                "process_info".into(),
                List::from(vec![
                    pid.clone().into(),
                    List::from(vec![
                        Atom::from("message_queue_len").into(),
                        Atom::from("memory").into(),
                        Atom::from("reductions").into(),
                    ])
                    .into(),
                ]),
            )
            .await?;
        if let Term::List(list) = term {
            let mut proc = Self {
                pid,
                message_queue_len: 0,
                memory: 0,
                reductions: 0,
            };
            for x in list.elements {
                let x: Tuple = x
                    .try_into()
                    .map_err(|e| anyhow::anyhow!("not a tuple: {}", e))?;
                assert_eq!(x.elements.len(), 2); // TODO

                let k: Atom = x.elements[0]
                    .clone()
                    .try_into()
                    .map_err(|e| anyhow::anyhow!("not an atom: {e}"))?;
                let v = x.elements[1].clone();
                match k.name.as_str() {
                    "message_queue_len" => {
                        proc.message_queue_len = erlang::term_to_u64(v)?;
                    }
                    "memory" => {
                        proc.memory = erlang::term_to_u64(v)?;
                    }
                    "reductions" => {
                        proc.reductions = erlang::term_to_u64(v)?;
                    }
                    name => {
                        // TODO
                        panic!("Unknown name: {}", name);
                    }
                }
            }
            Ok(Some(proc))
        } else {
            // undefined
            Ok(None)
        }
    }
}
