use erl_dist::node::NodeName;
use futures::channel::oneshot;
use std::time::Duration;

pub mod msacc;

#[derive(Debug)]
pub struct RpcClient {
    handle: erl_rpc::RpcClientHandle,
    err_rx: oneshot::Receiver<erl_rpc::RunError>,
}

impl RpcClient {
    pub async fn connect(erlang_node: &NodeName, cookie: &str) -> anyhow::Result<Self> {
        let client = erl_rpc::RpcClient::connect(&erlang_node.to_string(), cookie).await?;
        let handle = client.handle();
        let (err_tx, err_rx) = oneshot::channel();
        smol::spawn(async {
            if let Err(e) = client.run().await {
                let _ = err_tx.send(e);
            }
        })
        .detach();

        Ok(Self { handle, err_rx })
    }

    pub async fn get_msacc_stats(
        &mut self,
        duration: Duration,
    ) -> anyhow::Result<self::msacc::MsaccData> {
        if let Ok(Some(e)) = self.err_rx.try_recv() {
            return Err(e.into());
        }
        self::msacc::get_msacc_stats(self.handle.clone(), duration).await
    }
}

impl Drop for RpcClient {
    fn drop(&mut self) {
        self.handle.terminate();
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
