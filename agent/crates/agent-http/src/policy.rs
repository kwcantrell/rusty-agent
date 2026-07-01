//! In-tool host-approval policy, mapped onto the core Read/Write axis by the tool.

/// Whether a host may be contacted without approval.
pub enum HostDecision {
    Allow,
    Ask,
}

/// Case-insensitive host allowlist. Entries are either an exact host (`docs.rs`)
/// or a leading-dot suffix (`.rust-lang.org`, matching the apex and any subdomain).
pub struct NetworkPolicy {
    allow: Vec<String>,
}

impl NetworkPolicy {
    pub fn new(hosts: &[String]) -> Self {
        let allow = hosts
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        Self { allow }
    }

    pub fn decide(&self, host: &str) -> HostDecision {
        let h = host.to_ascii_lowercase();
        let allowed = self.allow.iter().any(|a| match a.strip_prefix('.') {
            Some(apex) => h == apex || h.ends_with(a.as_str()),
            None => &h == a,
        });
        if allowed {
            HostDecision::Allow
        } else {
            HostDecision::Ask
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowlist_asks_for_everything() {
        let p = NetworkPolicy::new(&[]);
        assert!(matches!(p.decide("example.com"), HostDecision::Ask));
    }

    #[test]
    fn exact_host_is_allowed_case_insensitively() {
        let p = NetworkPolicy::new(&["Docs.RS".to_string()]);
        assert!(matches!(p.decide("docs.rs"), HostDecision::Allow));
        assert!(matches!(
            p.decide("evil.docs.rs.attacker.com"),
            HostDecision::Ask
        ));
    }

    #[test]
    fn leading_dot_matches_apex_and_subdomains_only() {
        let p = NetworkPolicy::new(&[".rust-lang.org".to_string()]);
        assert!(matches!(p.decide("doc.rust-lang.org"), HostDecision::Allow));
        assert!(matches!(p.decide("rust-lang.org"), HostDecision::Allow));
        assert!(matches!(p.decide("notrust-lang.org"), HostDecision::Ask));
    }
}
