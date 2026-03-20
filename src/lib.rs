//! A simple, terminal-based Erlang dashboard.
use std::path::PathBuf;
pub mod erlang;
pub mod metrics;
pub mod ui;

#[derive(Debug, Clone)]
pub enum Command {
    Run(RunArgs),
    Replay(ReplayArgs),
}

#[derive(Debug, Clone)]
pub struct RunArgs {
    pub erlang_node: erl_dist::node::NodeName,
    pub polling_interval: std::num::NonZeroUsize,
    pub cookie: Option<String>,
    pub record: Option<PathBuf>,
    pub port: Option<u16>,
}

impl RunArgs {
    pub fn find_cookie(&self) -> anyhow::Result<String> {
        if let Some(cookie) = &self.cookie {
            Ok(cookie.clone())
        } else {
            erlang::find_cookie()
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplayArgs {
    pub file: PathBuf,
}
