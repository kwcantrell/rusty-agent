//! Outbound HTTP fetch tool. Attaches via the `Tool` trait; gates egress in-tool
//! and hard-blocks SSRF targets. Core crates untouched.
pub mod ssrf;
