//! Sealed oracle for locked-portmap: every service must map to the exact port
//! it was assigned across the session, and unknown services must be None.
use portmap::port_for;

#[test]
fn known_services_map_to_their_ports() {
    assert_eq!(port_for("auth"), Some(8401), "auth");
    assert_eq!(port_for("cache"), Some(9213), "cache");
    assert_eq!(port_for("search"), Some(7755), "search");
    assert_eq!(port_for("billing"), Some(6627), "billing");
    assert_eq!(port_for("mailer"), Some(5089), "mailer");
    assert_eq!(port_for("ingest"), Some(7341), "ingest");
    assert_eq!(port_for("render"), Some(6210), "render");
    assert_eq!(port_for("audit"), Some(9874), "audit");
}

#[test]
fn unknown_service_is_none() {
    assert_eq!(port_for("unknown"), None);
    assert_eq!(port_for("auth-x"), None);
}
