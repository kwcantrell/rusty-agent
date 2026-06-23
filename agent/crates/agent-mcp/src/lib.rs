//! Model Context Protocol (MCP) client: connect to external MCP servers over
//! stdio and surface their tools through the agent's `Tool`/`ToolRegistry` seam.

mod client;
mod config;
mod error;
mod transport;

pub use config::{McpServerSpec, McpServersConfig, Trust};
pub use error::McpError;
pub use transport::{McpTransport, StdioTransport};
