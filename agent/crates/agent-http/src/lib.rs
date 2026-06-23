//! Outbound HTTP fetch tool. Attaches via the `Tool` trait; gates egress in-tool
//! and hard-blocks SSRF targets. Core crates untouched.
pub mod content;
pub mod policy;
pub mod ssrf;
pub mod tool;

pub use policy::NetworkPolicy;
pub use tool::FetchUrl;
