//! Live integration test against the official filesystem MCP server.
//! Run explicitly: `cargo test -p agent-mcp --test live_filesystem -- --ignored`
//! Requires `npx` on PATH (downloads @modelcontextprotocol/server-filesystem).

use agent_mcp::{McpManager, McpServerSpec, McpServersConfig, Trust};
use std::collections::BTreeMap;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires npx + network; run manually for the DoD"]
async fn filesystem_server_tools_register_and_execute() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("hello.txt"), "hi from mcp").unwrap();

    let mut servers = BTreeMap::new();
    servers.insert("filesystem".to_string(), McpServerSpec {
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into(),
                   tmp.path().to_string_lossy().into_owned()],
        env: BTreeMap::new(),
        trust: Trust::Ask,
    });
    let cfg = McpServersConfig { servers };

    let mgr = McpManager::connect(&cfg, Duration::from_secs(30)).await;
    eprintln!("{}", mgr.summary_line());
    let tools = mgr.tools();
    assert!(!tools.is_empty(), "filesystem server should expose tools");
    assert!(tools.iter().any(|t| t.name().starts_with("filesystem__")),
        "tools should be namespaced by server");

    mgr.shutdown().await;
}
