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

fn render_age(updated_at: i64) -> String {
    let secs = (now_secs() - updated_at).max(0);
    if secs < 60 { "just now".into() }
    else if secs < 3600 { format!("{}m ago", secs / 60) }
    else if secs < 86400 { format!("{}h ago", secs / 3600) }
    else { format!("{}d ago", secs / 86400) }
}

pub struct Recall {
    pub embedder: Arc<dyn Embedder>,
    pub store: Arc<dyn MemoryStore>,
    pub cfg: Arc<MemoryConfig>,
    pub project_key: String,
}

#[async_trait]
impl Tool for Recall {
    fn name(&self) -> &str { "recall" }
    fn description(&self) -> &str {
        "Search long-term memory for facts relevant to a query. Returns the most similar \
         stored memories from this project and the global tier. Args: query (required), k (optional)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "recall".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "k": {"type": "integer", "minimum": 1}
                },
                "required": ["query"]
            }),
        }
    }
    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        Ok(read_intent("recall", "search long-term memory".into()))
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let query = args.get("query").and_then(Value::as_str)
            .map(str::trim).filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidArgs("missing non-empty 'query'".into()))?;
        let k = args.get("k").and_then(Value::as_u64).map(|n| n as usize)
            .unwrap_or(self.cfg.default_k).clamp(1, self.cfg.max_k);
        let qv = self.embedder.embed(&[query.to_string()]).await
            .map_err(embed_failed)?.into_iter().next().unwrap();

        let filter = ScopeFilter::ProjectAndGlobal { project_key: self.project_key.clone() };
        let mut hits = self.store.query(&qv, self.cfg.max_k, &filter).await.map_err(store_failed)?;
        hits.retain(|h| h.score >= self.cfg.relevance_threshold);
        hits.truncate(k);
        tracing::info!(target: "memory", returned = hits.len(),
            top = hits.first().map(|h| h.score).unwrap_or(0.0), "recall");

        if hits.is_empty() {
            return Ok(ToolOutput { content: "No relevant memories found.".into(), display: None });
        }
        let body = render_hits(&hits, self.cfg.max_recall_chars);
        Ok(ToolOutput { content: body, display: None })
    }
}

fn render_hits(hits: &[crate::record::Scored], max_chars: usize) -> String {
    let mut out = String::new();
    for h in hits {
        let tags = if h.record.tags.is_empty() { String::new() }
                   else { format!("; tags: {}", h.record.tags.join(",")) };
        let line = format!("[{:.2}] {} ({}{})\n",
            h.score, h.record.text, render_age(h.record.updated_at), tags);
        if out.len() + line.len() > max_chars {
            out.push_str("[truncated: more memories matched]\n");
            break;
        }
        out.push_str(&line);
    }
    out
}

#[cfg(test)]
mod recall_tests {
    use super::test_support::ctx;
    use super::*;
    use crate::config::MemoryConfig;
    use crate::embedder::StubEmbedder;
    use crate::store::InMemoryStore;

    async fn seed() -> (Recall, Arc<dyn MemoryStore>) {
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let cfg = Arc::new(MemoryConfig::default());
        let rem = Remember { embedder: embedder.clone(), store: store.clone(), cfg: cfg.clone(),
            project_key: "A".into() };
        rem.execute(json!({"text": "deploys run on fridays"}), &ctx()).await.unwrap();
        rem.execute(json!({"text": "user prefers tabs", "scope": "global"}), &ctx()).await.unwrap();
        let rec = Recall { embedder, store: store.clone(), cfg, project_key: "A".into() };
        (rec, store)
    }

    #[tokio::test]
    async fn exact_query_returns_match_unrelated_returns_none() {
        let (rec, _s) = seed().await;
        // Exact stored text → cosine 1.0 ≥ relevance_threshold.
        let hit = rec.execute(json!({"query": "deploys run on fridays"}), &ctx()).await.unwrap();
        assert!(hit.content.contains("deploys run on fridays"));
        // Unrelated query → below threshold → "no relevant memories".
        let miss = rec.execute(json!({"query": "zxcv qwerty nonsense token"}), &ctx()).await.unwrap();
        assert!(miss.content.contains("No relevant memories"));
    }

    #[tokio::test]
    async fn global_visible_but_other_projects_hidden() {
        let (rec, store) = seed().await;
        // Add a project-B memory directly; project-A recall must not see it.
        let embedder = StubEmbedder::d384();
        let v = embedder.embed(&["secret from project b".to_string()]).await.unwrap().pop().unwrap();
        store.upsert(MemoryRecord { id: "b1".into(), text: "secret from project b".into(),
            scope: MemoryScope::Project("B".into()), tags: vec![], vector: v,
            created_at: 1, updated_at: 1, source: "test".into() }).await.unwrap();
        let out = rec.execute(json!({"query": "secret from project b"}), &ctx()).await.unwrap();
        assert!(!out.content.contains("secret from project b"), "cross-project leak");
        // But the global memory is reachable.
        let g = rec.execute(json!({"query": "user prefers tabs"}), &ctx()).await.unwrap();
        assert!(g.content.contains("user prefers tabs"));
    }

    #[tokio::test]
    async fn render_budget_truncates() {
        use crate::record::Scored;
        let hits: Vec<Scored> = (0..100).map(|i| Scored {
            record: MemoryRecord { id: i.to_string(), text: "x".repeat(100),
                scope: MemoryScope::Global, tags: vec![], vector: vec![1.0],
                created_at: 0, updated_at: 0, source: "t".into() },
            score: 0.9,
        }).collect();
        let body = render_hits(&hits, 512);
        assert!(body.len() <= 512 + 64);
        assert!(body.contains("[truncated"));
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
