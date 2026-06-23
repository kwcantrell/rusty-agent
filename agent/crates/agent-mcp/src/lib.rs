//! Model Context Protocol (MCP) client: connect to external MCP servers over
//! stdio and surface their tools through the agent's `Tool`/`ToolRegistry` seam.

mod client;
mod config;
mod error;
mod manager;
mod tool;
mod transport;

pub use config::{McpServerSpec, McpServersConfig, Trust};
pub use error::McpError;
pub use manager::{McpManager, ServerStatus};
pub use tool::McpTool;
pub use transport::{McpTransport, StdioTransport};
