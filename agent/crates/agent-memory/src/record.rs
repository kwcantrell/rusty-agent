use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MemoryScope {
    Project(String), // sha256 hex of the canonical project root
    Global,
}

impl MemoryScope {
    pub fn kind(&self) -> &'static str {
        match self { MemoryScope::Project(_) => "project", MemoryScope::Global => "global" }
    }
    pub fn key(&self) -> &str {
        match self { MemoryScope::Project(k) => k, MemoryScope::Global => "" }
    }
}

/// How a query selects rows. `Exact` is used for dedup (same scope only);
/// `ProjectAndGlobal` is used for recall (current project + the global tier).
#[derive(Debug, Clone)]
pub enum ScopeFilter {
    Exact(MemoryScope),
    ProjectAndGlobal { project_key: String },
}

impl ScopeFilter {
    /// Does a stored record's scope satisfy this filter?
    pub fn matches(&self, scope: &MemoryScope) -> bool {
        match self {
            ScopeFilter::Exact(s) => s == scope,
            ScopeFilter::ProjectAndGlobal { project_key } => match scope {
                MemoryScope::Global => true,
                MemoryScope::Project(k) => k == project_key,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryRecord {
    pub id: String,
    pub text: String,
    pub scope: MemoryScope,
    pub tags: Vec<String>,
    pub vector: Vec<f32>,
    pub created_at: i64,
    pub updated_at: i64,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Scored {
    pub record: MemoryRecord,
    pub score: f32,
}

pub fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_and_global_filter_admits_global_and_matching_project_only() {
        let f = ScopeFilter::ProjectAndGlobal { project_key: "abc".into() };
        assert!(f.matches(&MemoryScope::Global));
        assert!(f.matches(&MemoryScope::Project("abc".into())));
        assert!(!f.matches(&MemoryScope::Project("other".into())));
    }

    #[test]
    fn exact_filter_admits_only_same_scope() {
        let f = ScopeFilter::Exact(MemoryScope::Project("abc".into()));
        assert!(f.matches(&MemoryScope::Project("abc".into())));
        assert!(!f.matches(&MemoryScope::Global));
    }
}
