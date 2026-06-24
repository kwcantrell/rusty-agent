use crate::config::MemoryConfig;
use crate::embedder::Embedder;
use crate::record::{now_secs, MemoryRecord, MemoryScope, ScopeFilter};
use crate::store::MemoryStore;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Memory ops touch only the private local store, so they declare a Read-access,
/// path-less, command-less intent → `RulePolicy` auto-allows them. Approval-gating
/// memory writes is deferred per spec §1; `summary` stays truthful for the audit log.
fn read_intent(tool: &str, summary: String) -> ToolIntent {
    ToolIntent { tool: tool.into(), access: Access::Read, paths: vec![], command: None, summary }
}

pub(crate) fn parse_scope(args: &Value, project_key: &str) -> MemoryScope {
    match args.get("scope").and_then(Value::as_str) {
        Some("global") => MemoryScope::Global,
        _ => MemoryScope::Project(project_key.to_string()),
    }
}

pub(crate) fn parse_tags(args: &Value, cfg: &MemoryConfig) -> Vec<String> {
    args.get("tags").and_then(Value::as_array).map(|a| {
        a.iter().filter_map(Value::as_str)
            .map(|s| s.chars().take(cfg.max_tag_len).collect::<String>())
            .take(cfg.max_tags).collect()
    }).unwrap_or_default()
}

fn embed_failed(e: impl std::fmt::Display) -> ToolError {
    ToolError::Failed { message: format!("embedding failed: {e}"), stderr: None }
}
fn store_failed(e: impl std::fmt::Display) -> ToolError {
    ToolError::Failed { message: format!("memory store error: {e}"), stderr: None }
}

pub struct Remember {
    pub embedder: Arc<dyn Embedder>,
    pub store: Arc<dyn MemoryStore>,
    pub cfg: Arc<MemoryConfig>,
    pub project_key: String,
}

#[async_trait]
impl Tool for Remember {
    fn name(&self) -> &str { "remember" }
    fn description(&self) -> &str {
        "Store a fact in long-term memory for recall in future sessions. \
         Args: text (required), tags (optional string array), scope ('project'|'global', default project)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "remember".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "The fact to remember"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "scope": {"type": "string", "enum": ["project", "global"]}
                },
                "required": ["text"]
            }),
        }
    }
    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        Ok(read_intent("remember", "write to long-term memory store".into()))
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let text = args.get("text").and_then(Value::as_str)
            .map(str::trim).filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidArgs("missing non-empty 'text'".into()))?;
        if text.len() > self.cfg.max_text_len {
            return Err(ToolError::InvalidArgs(format!(
                "text too long ({} bytes; max {})", text.len(), self.cfg.max_text_len)));
        }
        let scope = parse_scope(&args, &self.project_key);
        let tags = parse_tags(&args, &self.cfg);
        let vector = self.embedder.embed(&[text.to_string()]).await
            .map_err(embed_failed)?.into_iter().next().unwrap();

        // Dedup: supersede a near-identical memory in the same scope instead of duplicating.
        let near = self.store.query(&vector, 1, &ScopeFilter::Exact(scope.clone()))
            .await.map_err(store_failed)?;
        if let Some(top) = near.first() {
            if top.score >= self.cfg.dedup_threshold {
                let mut rec = top.record.clone();
                rec.text = text.to_string();
                rec.tags = tags;
                rec.vector = vector;
                rec.updated_at = now_secs();
                let id = rec.id.clone();
                self.store.upsert(rec).await.map_err(store_failed)?;
                tracing::info!(target: "memory", %id, scope = scope.kind(), "remember: superseded");
                return Ok(ToolOutput { content: format!("Updated existing memory {id}."), display: None });
            }
        }

        // Cap: evict least-recently-updated while at the per-scope ceiling.
        while self.store.count(&ScopeFilter::Exact(scope.clone())).await.map_err(store_failed)?
            >= self.cfg.max_memories_per_scope {
            if let Some(ev) = self.store.evict_oldest(&scope).await.map_err(store_failed)? {
                tracing::warn!(target: "memory", evicted = %ev, "remember: scope cap reached, evicted oldest");
            } else { break; }
        }

        let now = now_secs();
        let id = Uuid::new_v4().to_string();
        let rec = MemoryRecord { id: id.clone(), text: text.to_string(), scope: scope.clone(),
            tags, vector, created_at: now, updated_at: now, source: "remember".into() };
        self.store.upsert(rec).await.map_err(store_failed)?;
        tracing::info!(target: "memory", %id, scope = scope.kind(), "remember: stored new");
        Ok(ToolOutput { content: format!("Stored memory {id}."), display: None })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::embedder::StubEmbedder;
    use crate::store::InMemoryStore;

    pub fn remember(project_key: &str) -> (Remember, Arc<dyn MemoryStore>, Arc<dyn Embedder>, Arc<MemoryConfig>) {
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let cfg = Arc::new(MemoryConfig::default());
        let r = Remember { embedder: embedder.clone(), store: store.clone(), cfg: cfg.clone(),
            project_key: project_key.into() };
        (r, store, embedder, cfg)
    }

    pub fn ctx() -> ToolCtx {
        ToolCtx {
            workspace: std::path::PathBuf::from("/tmp"),
            timeout: std::time::Duration::from_secs(5),
            cancel: tokio_util::sync::CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::record::ScopeFilter;

    #[tokio::test]
    async fn stores_new_then_supersedes_identical() {
        let (r, store, _e, _c) = remember("A");
        r.execute(json!({"text": "the build uses cargo"}), &ctx()).await.unwrap();
        // Identical text → cosine 1.0 ≥ dedup_threshold → supersede, not duplicate.
        r.execute(json!({"text": "the build uses cargo"}), &ctx()).await.unwrap();
        let scope = MemoryScope::Project("A".into());
        assert_eq!(store.count(&ScopeFilter::Exact(scope)).await.unwrap(), 1, "deduped");
    }

    #[tokio::test]
    async fn distinct_text_inserts_separately() {
        let (r, store, _e, _c) = remember("A");
        r.execute(json!({"text": "fact one about networking"}), &ctx()).await.unwrap();
        r.execute(json!({"text": "an unrelated fact about cooking"}), &ctx()).await.unwrap();
        let scope = MemoryScope::Project("A".into());
        assert_eq!(store.count(&ScopeFilter::Exact(scope)).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn oversized_text_is_rejected() {
        let (r, _s, _e, cfg) = remember("A");
        let big = "x".repeat(cfg.max_text_len + 1);
        let err = r.execute(json!({"text": big}), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn empty_text_is_rejected() {
        let (r, _s, _e, _c) = remember("A");
        let err = r.execute(json!({"text": "   "}), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }
}
