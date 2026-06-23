//! SSRF hard floor: classify resolved IPs against ranges we must never connect to.
use ipnet::{Ipv4Net, Ipv6Net};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// IPv4 ranges that must never be the target of an outbound request.
const BLOCKED_V4: &[&str] = &[
    "0.0.0.0/8", "10.0.0.0/8", "100.64.0.0/10", "127.0.0.0/8", "169.254.0.0/16",
    "172.16.0.0/12", "192.0.0.0/24", "192.0.2.0/24", "192.168.0.0/16",
    "198.18.0.0/15", "198.51.100.0/24", "203.0.113.0/24", "224.0.0.0/4", "240.0.0.0/4",
];
/// IPv6 ranges (loopback/unspecified/multicast handled via std predicates).
const BLOCKED_V6: &[&str] = &["fc00::/7", "fe80::/10"];

/// True if `ip` is in a blocked range (the non-overridable SSRF floor).
pub fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_v4(v4);
            }
            is_blocked_v6(v6)
        }
    }
}

fn is_blocked_v4(ip: Ipv4Addr) -> bool {
    if ip == Ipv4Addr::new(255, 255, 255, 255) {
        return true;
    }
    BLOCKED_V4
        .iter()
        .any(|c| c.parse::<Ipv4Net>().expect("static cidr").contains(&ip))
}

fn is_blocked_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    BLOCKED_V6
        .iter()
        .any(|c| c.parse::<Ipv6Net>().expect("static cidr").contains(&ip))
}

/// Injectable IP gate. Production uses `strict()`; tests against a loopback mock
/// server use the `#[cfg(test)]`-only `allow_all()`.
#[derive(Clone, Copy)]
pub struct SsrfGuard {
    allow_all: bool,
}

impl SsrfGuard {
    pub fn strict() -> Self {
        Self { allow_all: false }
    }

    #[cfg(test)]
    pub fn allow_all() -> Self {
        Self { allow_all: true }
    }

    /// `Ok(())` if the IP may be contacted; `Err(ip)` if the floor blocks it.
    pub fn check(&self, ip: IpAddr) -> Result<(), IpAddr> {
        if self.allow_all || !is_blocked(ip) {
            Ok(())
        } else {
            Err(ip)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn ip(s: &str) -> IpAddr { s.parse().unwrap() }

    #[test]
    fn blocks_loopback_private_linklocal_metadata_and_reserved() {
        for s in [
            "127.0.0.1", "127.10.0.1", "10.1.2.3", "172.16.5.4", "192.168.1.1",
            "169.254.169.254", "169.254.0.1", "100.64.0.1", "0.0.0.0",
            "192.0.0.1", "192.0.2.5", "198.18.0.1", "203.0.113.9", "224.0.0.1",
            "240.0.0.1", "255.255.255.255",
            "::1", "::", "fc00::1", "fd12:3456::1", "fe80::1",
            "::ffff:127.0.0.1", "::ffff:10.0.0.1",
        ] {
            assert!(is_blocked(ip(s)), "expected {s} blocked");
        }
    }

    #[test]
    fn allows_public_addresses() {
        for s in ["1.1.1.1", "8.8.8.8", "93.184.216.34", "2606:2800:220:1::1"] {
            assert!(!is_blocked(ip(s)), "expected {s} allowed");
        }
    }

    #[test]
    fn strict_guard_rejects_blocked_allows_public() {
        let g = SsrfGuard::strict();
        assert!(g.check(ip("127.0.0.1")).is_err());
        assert!(g.check(ip("8.8.8.8")).is_ok());
    }

    #[test]
    fn allow_all_guard_permits_loopback_for_tests() {
        assert!(SsrfGuard::allow_all().check(ip("127.0.0.1")).is_ok());
    }
}
