//! Model Context Protocol (MCP) client: connect to external MCP servers over
//! stdio and surface their tools through the agent's `Tool`/`ToolRegistry` seam.

mod config;
mod error;

pub use config::{McpServerSpec, McpServersConfig, Trust};
pub use error::McpError;
