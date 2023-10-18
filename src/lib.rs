//! A simple, terminal-based Erlang dashboard.
use std::path::PathBuf;
pub mod erlang;
pub mod metrics;
pub mod ui;

#[derive(Debug, Clone, clap::Parser)]
pub struct Options {
    /// Target Erlang node name.
    pub erlang_node: erl_dist::node::NodeName,

    /// Erlang metrics polling interval (in seconds).
    #[clap(long, short = 'i', default_value = "1")]
    pub polling_interval: std::num::NonZeroUsize,

    /// Erlang cookie.
    ///
    /// By default, the content of the `$HOME/.erlang.cookie` file is used.
    #[clap(long, short = 'c')]
    pub cookie: Option<String>,

    /// TODO: doc
    #[clap(long)]
    record: Option<PathBuf>,
}

impl Options {
    pub fn find_cookie(&self) -> anyhow::Result<String> {
        if let Some(cookie) = &self.cookie {
            Ok(cookie.clone())
        } else {
            erlang::find_cookie()
        }
    }
}
