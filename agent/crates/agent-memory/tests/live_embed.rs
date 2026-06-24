//! Live DoD: real embedding model, cross-process semantic recall by paraphrase.
//! Run with: cargo test -p agent-memory --test live_embed -- --ignored
#![cfg(feature = "onnx")]
use agent_memory::*;
use std::sync::Arc;

#[tokio::test]
#[ignore = "downloads + runs the real embedding model"]
async fn paraphrase_recall_across_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = MemoryConfig {
        db_path: tmp.path().join("memory.db"),
        model_cache_dir: Some(tmp.path().join("models")),
        ..MemoryConfig::default()
    };

    // Session 1: remember, then drop everything.
    {
        let tools = build_tools(cfg.clone(), tmp.path()).unwrap();
        let remember = tools.iter().find(|t| t.name() == "remember").unwrap();
        let ctx = test_ctx();
        remember.execute(serde_json::json!({
            "text": "The deployment pipeline runs every Friday afternoon."
        }), &ctx).await.unwrap();
    }
    // Session 2: fresh build over the same DB; recall by a paraphrase (no lexical overlap).
    let tools = build_tools(cfg, tmp.path()).unwrap();
    let recall = tools.iter().find(|t| t.name() == "recall").unwrap();
    let out = recall.execute(serde_json::json!({
        "query": "When do we ship releases?"
    }), &test_ctx()).await.unwrap();
    assert!(out.content.contains("Friday"), "paraphrase failed to retrieve: {}", out.content);
}

fn test_ctx() -> agent_tools::ToolCtx {
    agent_tools::ToolCtx {
        workspace: std::path::PathBuf::from("."),
        timeout: std::time::Duration::from_secs(30),
        cancel: tokio_util::sync::CancellationToken::new(),
        sandbox: Arc::new(agent_tools::HostExecutor),
    }
}
