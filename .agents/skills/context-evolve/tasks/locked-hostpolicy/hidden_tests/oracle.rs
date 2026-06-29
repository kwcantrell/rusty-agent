//! Sealed oracle for locked-hostpolicy — the real NetworkPolicy tests (from
//! agent-http/src/policy.rs, commit fbe1312), run as integration tests via cargo.
use hostpolicy::{HostDecision, NetworkPolicy};

#[test]
fn empty_allowlist_asks_for_everything() {
    let p = NetworkPolicy::new(&[]);
    assert!(matches!(p.decide("example.com"), HostDecision::Ask));
}

#[test]
fn exact_host_is_allowed_case_insensitively() {
    let p = NetworkPolicy::new(&["Docs.RS".to_string()]);
    assert!(matches!(p.decide("docs.rs"), HostDecision::Allow));
    assert!(matches!(p.decide("evil.docs.rs.attacker.com"), HostDecision::Ask));
}

#[test]
fn leading_dot_matches_apex_and_subdomains_only() {
    let p = NetworkPolicy::new(&[".rust-lang.org".to_string()]);
    assert!(matches!(p.decide("doc.rust-lang.org"), HostDecision::Allow));
    assert!(matches!(p.decide("rust-lang.org"), HostDecision::Allow));
    assert!(matches!(p.decide("notrust-lang.org"), HostDecision::Ask));
}
