//! A simple, terminal-based Erlang dashboard.
use std::path::PathBuf;
pub mod erlang;
pub mod metrics;
pub mod ui;

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Run the dashboard.
    Run(RunArgs),

    /// Replay a previously recorded dashboard session.
    Replay(ReplayArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub struct RunArgs {
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

    /// If specified, the collected metrics will be recorded to the given file and can be replayed later.
    #[clap(long, value_name = "FILE")]
    pub record: Option<PathBuf>,

    /// Port number on which the target node listens.
    ///
    /// If specified, `erldash` will connect directly to the node without using EPMD.
    #[clap(long, short)]
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

#[derive(Debug, Clone, clap::Args)]
pub struct ReplayArgs {
    /// Path to a file containing recorded metrics.
    pub file: PathBuf,
}
