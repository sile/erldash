use std::process::Command;
use std::time::Duration;

fn start_erlang_node() -> std::process::Child {
    Command::new("erl")
        .args([
            "-sname",
            "erldash_test",
            "-setcookie",
            "test",
            "-noshell",
            "-noinput",
        ])
        .spawn()
        .expect("failed to start Erlang node (is Erlang installed?)")
}

#[test]
fn connect_and_poll_metrics() {
    let mut node = start_erlang_node();
    std::thread::sleep(Duration::from_secs(2));

    let result = smol::block_on(async {
        let node_name: erl_dist::node::NodeName = "erldash_test@localhost".parse().unwrap();
        let client = erldash::erlang::RpcClient::connect(&node_name, None, "test").await?;

        let version = client.get_system_version().await?;
        assert!(
            version.get().contains("Erlang"),
            "unexpected system version: {}",
            version.get()
        );

        let memory = client.get_memory().await?;
        assert!(
            memory.contains_key("total"),
            "memory should contain 'total'"
        );
        assert!(*memory.get("total").unwrap() > 0);

        let process_count = client.get_system_info_u64("process_count").await?;
        assert!(process_count > 0);

        let port_count = client.get_system_info_u64("port_count").await?;
        assert!(port_count > 0);

        let atom_count = client.get_system_info_u64("atom_count").await?;
        assert!(atom_count > 0);

        let old = client
            .set_system_flag_bool("microstate_accounting", "true")
            .await?;
        assert!(!old || old); // just check it returns a bool without error

        let msacc = client.get_statistics_microstate_accounting().await?;
        assert!(
            !msacc.is_empty(),
            "microstate accounting should return threads"
        );

        client
            .set_system_flag_bool("microstate_accounting", "false")
            .await?;

        Ok::<_, erldash::error::Error>(())
    });

    node.kill().ok();
    node.wait().ok();

    result.unwrap();
}
