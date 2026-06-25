use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub db_path: PathBuf,
    pub model_cache_dir: Option<PathBuf>,
    pub default_k: usize,
    pub max_k: usize,
    pub relevance_threshold: f32,
    pub dedup_threshold: f32,
    pub forget_threshold: f32,
    pub max_text_len: usize,
    pub max_tags: usize,
    pub max_tag_len: usize,
    pub max_memories_per_scope: usize,
    pub max_recall_chars: usize,
    pub candidate_warn_threshold: usize,
    pub auto_recall: bool,
    pub recall_token_budget: usize,
}

pub fn default_db_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".agent").join("memory.db")
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            model_cache_dir: None,
            default_k: 5,
            max_k: 20,
            relevance_threshold: 0.3,
            dedup_threshold: 0.95,
            forget_threshold: 0.85,
            max_text_len: 8 * 1024,
            max_tags: 16,
            max_tag_len: 64,
            max_memories_per_scope: 10_000,
            max_recall_chars: 4 * 1024,
            candidate_warn_threshold: 50_000,
            auto_recall: true,
            recall_token_budget: 512,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec_table() {
        let c = MemoryConfig::default();
        assert_eq!(c.default_k, 5);
        assert_eq!(c.max_k, 20);
        assert_eq!(c.max_text_len, 8 * 1024);
        assert_eq!(c.max_memories_per_scope, 10_000);
        assert_eq!(c.max_recall_chars, 4 * 1024);
        assert!((c.dedup_threshold - 0.95).abs() < 1e-6);
        assert!((c.relevance_threshold - 0.3).abs() < 1e-6);
        assert!((c.forget_threshold - 0.85).abs() < 1e-6);
        assert!(c.auto_recall);
        assert_eq!(c.recall_token_budget, 512);
    }
}
