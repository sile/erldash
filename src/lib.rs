pub mod erlang;
pub mod metrics;
pub mod ui;

#[derive(Debug, Clone, clap::Parser)]
pub struct Options {
    pub erlang_node: erl_dist::node::NodeName,

    #[clap(long, short = 'i', default_value = "1")]
    pub polling_interval: std::num::NonZeroUsize,

    #[clap(long, short = 'c')]
    pub cookie: Option<String>,
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
