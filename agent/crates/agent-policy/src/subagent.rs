//! Per-sub-agent permission floors (spec 3B-1c): a flat tool-name pattern
//! dialect and a monotone-narrowing wrapper over a base `PolicyEngine`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPattern {
    Exact(String),
    Prefix(String), // "github__*" → names starting with "github__"
    Any,            // "*"
}

impl ToolPattern {
    /// Flat dialect (spec §2.3): exact name, trailing-`*` prefix, or bare `*`.
    /// Leading/interior `*`, empty, and multi-`*` are errors (Suffix cut at gate E1).
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            return Err("empty pattern".into());
        }
        if s == "*" {
            return Ok(Self::Any);
        }
        match s.matches('*').count() {
            0 => Ok(Self::Exact(s.to_string())),
            1 if s.ends_with('*') => Ok(Self::Prefix(s[..s.len() - 1].to_string())),
            _ => Err(format!(
                "pattern '{s}': '*' is allowed only as a trailing wildcard (or the bare \"*\")"
            )),
        }
    }

    pub fn matches(&self, tool: &str) -> bool {
        match self {
            Self::Exact(n) => tool == n,
            Self::Prefix(p) => tool.starts_with(p.as_str()),
            Self::Any => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_exact_prefix_and_any() {
        assert_eq!(
            ToolPattern::parse("write_file").unwrap(),
            ToolPattern::Exact("write_file".into())
        );
        assert_eq!(
            ToolPattern::parse("github__*").unwrap(),
            ToolPattern::Prefix("github__".into())
        );
        assert_eq!(ToolPattern::parse("*").unwrap(), ToolPattern::Any);
    }

    #[test]
    fn parse_rejects_empty_interior_leading_and_double_star() {
        for bad in ["", "a*b", "*a*", "*_file", "**"] {
            assert!(
                ToolPattern::parse(bad).is_err(),
                "{bad:?} must be a dialect error"
            );
        }
    }

    #[test]
    fn prefix_affix_is_nonempty_by_construction() {
        // "*" alone is Any, so Prefix("") can never arise.
        assert!(!matches!(
            ToolPattern::parse("*").unwrap(),
            ToolPattern::Prefix(_)
        ));
    }

    #[test]
    fn matches_semantics_case_sensitive() {
        assert!(ToolPattern::Exact("write_file".into()).matches("write_file"));
        assert!(!ToolPattern::Exact("write_file".into()).matches("Write_File"));
        assert!(ToolPattern::Prefix("github__".into()).matches("github__create_issue"));
        assert!(!ToolPattern::Prefix("github__".into()).matches("GitHub__create_issue"));
        assert!(!ToolPattern::Prefix("github__".into()).matches("gitlab__x"));
        assert!(ToolPattern::Any.matches("anything"));
    }
}
